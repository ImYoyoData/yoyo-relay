//! Balance queries. Auto mode performs a provider-specific GET and extracts a
//! number via a small JSON path; manual mode is derived from recorded spend.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde_json::Value;

use crate::config::{now_ts, Account, BalanceMode};
use crate::db::BalanceRow;
use crate::gateway::{apply_auth, readable_reqwest_error, Gateway};

pub fn load_balance_rows(gw: &Gateway) -> HashMap<String, BalanceRow> {
    match gw.db.balances() {
        Ok(rows) => rows
            .into_iter()
            .map(|r| (r.account_id.clone(), r))
            .collect(),
        Err(err) => {
            tracing::warn!("load balances failed: {err}");
            HashMap::new()
        }
    }
}

/// Refresh every auto-configured account (or a single one).
pub async fn refresh_balances(gw: &Gateway, only: Option<&str>) -> Vec<(String, Result<()>)> {
    let cfg = gw.config();
    let mut results = Vec::new();
    for account in &cfg.accounts {
        if let Some(id) = only {
            if account.id != id {
                continue;
            }
        }
        if account.balance.mode != BalanceMode::Auto {
            continue;
        }
        let Some(url) = account.balance.url.clone().filter(|u| !u.trim().is_empty()) else {
            continue;
        };
        let Some(provider) = cfg.provider(&account.provider_id) else {
            results.push((account.id.clone(), Err(anyhow!("供应商不存在"))));
            continue;
        };
        let key = match gw.master.decrypt(&account.api_key) {
            Ok(k) => k,
            Err(err) => {
                results.push((account.id.clone(), Err(anyhow!("Key 解密失败：{err}"))));
                continue;
            }
        };
        match fetch_balance(gw, account, &url, &provider.auth, &key).await {
            Ok(amount) => {
                let row = BalanceRow {
                    account_id: account.id.clone(),
                    amount,
                    currency: if account.balance.currency.is_empty() {
                        "USD".to_string()
                    } else {
                        account.balance.currency.clone()
                    },
                    source: "auto".to_string(),
                    updated_at: now_ts(),
                    note: String::new(),
                };
                if let Err(err) = gw.db.balance_upsert(&row) {
                    results.push((account.id.clone(), Err(anyhow!("写入余额失败：{err}"))));
                } else {
                    results.push((account.id.clone(), Ok(())));
                }
            }
            Err(err) => results.push((account.id.clone(), Err(err))),
        }
    }
    results
}

async fn fetch_balance(
    gw: &Gateway,
    account: &Account,
    url: &str,
    auth: &crate::config::AuthConfig,
    key: &str,
) -> Result<f64> {
    let mut req = gw
        .client_for(url)
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json");
    req = apply_auth(req, auth, key)?;
    let resp = tokio::time::timeout(Duration::from_secs(20), req.send())
        .await
        .map_err(|_| anyhow!("余额查询超时"))?
        .map_err(|err| anyhow!(readable_reqwest_error(&err)))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!(
            "余额接口 HTTP {}：{}",
            status,
            crate::gateway::upstream_error_message(&text)
        );
    }
    let value: Value =
        serde_json::from_str(&text).map_err(|e| anyhow!("余额响应不是 JSON：{e}"))?;
    let path = if account.balance.json_path.trim().is_empty() {
        "balance"
    } else {
        account.balance.json_path.trim()
    };
    let found = json_path(&value, path).ok_or_else(|| anyhow!("余额 JSON 路径「{path}」不存在"))?;
    let mut amount = match found {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s
            .trim()
            .trim_start_matches('$')
            .trim_matches('"')
            .parse::<f64>()
            .map_err(|_| anyhow!("余额不是数字：{s}"))?,
        Value::Object(obj) => obj
            .get("amount")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| anyhow!("余额对象缺少 amount 字段"))?,
        _ => bail!("余额字段类型不支持"),
    };
    let scale = if account.balance.scale == 0.0 {
        1.0
    } else {
        account.balance.scale
    };
    amount *= scale;
    Ok(amount)
}

/// Minimal dot/bracket path lookup, e.g. `data.balance`, `data[0].credits`.
pub fn json_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    let mut token = String::new();
    let mut chars = path.chars().peekable();
    let flush = |token: &mut String, current: &mut &'a Value| -> bool {
        if token.is_empty() {
            return true;
        }
        if let Ok(index) = token.parse::<usize>() {
            if let Some(next) = current.get(index) {
                *current = next;
            } else {
                return false;
            }
        } else if let Some(next) = current.get(token.as_str()) {
            *current = next;
        } else {
            return false;
        }
        token.clear();
        true
    };
    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                if !flush(&mut token, &mut current) {
                    return None;
                }
            }
            '[' => {
                if !flush(&mut token, &mut current) {
                    return None;
                }
                let mut index = String::new();
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    index.push(c);
                }
                let index = index.trim().trim_matches('"').trim_matches('\'');
                if let Some(next) = current.get(index) {
                    current = next;
                } else if let Ok(i) = index.parse::<usize>() {
                    current = current.get(i)?;
                } else {
                    return None;
                }
            }
            _ => token.push(ch),
        }
    }
    if !flush(&mut token, &mut current) {
        return None;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn path_lookup() {
        let v = json!({"data": {"balance": 12.5}, "items": [{"credits": 3}]});
        assert_eq!(json_path(&v, "data.balance").unwrap().as_f64(), Some(12.5));
        assert_eq!(
            json_path(&v, "items[0].credits").unwrap().as_f64(),
            Some(3.0)
        );
        assert!(json_path(&v, "data.nope").is_none());
    }
}
