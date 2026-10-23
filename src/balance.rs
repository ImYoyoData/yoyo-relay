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
        let quota_preset = !account.balance.quota_preset.trim().is_empty();
        if account.balance.mode != BalanceMode::Auto && !quota_preset {
            continue;
        }
        let url = account.balance.url.clone().filter(|u| !u.trim().is_empty());
        if url.is_none() && !quota_preset {
            continue;
        }
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
        let amount = if let Some(url) = &url {
            match fetch_balance(gw, account, url, &provider.auth, &key).await {
                Ok(amount) => amount,
                Err(err) => {
                    if !quota_preset {
                        results.push((account.id.clone(), Err(err)));
                        continue;
                    }
                    0.0
                }
            }
        } else {
            0.0
        };
        let quota_windows = if quota_preset {
            fetch_quota_windows(gw, account, &provider, &key).await
        } else {
            Vec::new()
        };
        let quota_json = serde_json::to_string(&quota_windows).unwrap_or_default();
        let row = BalanceRow {
            account_id: account.id.clone(),
            amount,
            currency: if account.balance.currency.is_empty() {
                "USD".to_string()
            } else {
                account.balance.currency.clone()
            },
            source: if quota_windows.is_empty() {
                "auto".to_string()
            } else {
                "quota-api".to_string()
            },
            updated_at: now_ts(),
            note: String::new(),
            quota_json,
        };
        if let Err(err) = gw.db.balance_upsert(&row) {
            results.push((account.id.clone(), Err(anyhow!("写入余额失败：{err}"))));
        } else {
            results.push((account.id.clone(), Ok(())));
        }
    }
    results
}

/// Try the built-in usage endpoints for subscription providers, then fall back
/// to the provider's own base URL. The response shape is parsed heuristically
/// because these endpoints are not publicly documented.
async fn fetch_quota_windows(
    gw: &Gateway,
    account: &Account,
    provider: &crate::config::Provider,
    key: &str,
) -> Vec<crate::snapshot::QuotaWindowView> {
    let mut candidates = Vec::new();
    if let Some(url) = account
        .balance
        .url
        .clone()
        .filter(|url| !url.trim().is_empty())
    {
        candidates.push(url);
    }
    match account.balance.quota_preset.as_str() {
        "opencode_go" => {
            candidates.push("https://opencode.ai/inference/go/v1/usage".into());
            candidates.push("https://opencode.ai/inference/go/v1/me".into());
            candidates.push("https://opencode.ai/inference/v1/usage".into());
        }
        "command_code_go" => {
            candidates.push("https://api.commandcode.ai/provider/v1/usage".into());
            candidates.push("https://api.commandcode.ai/provider/v1/me".into());
            candidates.push("https://api.commandcode.ai/v1/usage".into());
        }
        _ => {}
    }
    let base = provider.base_url.trim_end_matches('/');
    if let Some(root) = base.strip_suffix("/openai/v1") {
        candidates.push(format!("{root}/usage"));
    }
    candidates.push(format!("{base}/usage"));
    candidates.sort();
    candidates.dedup();
    for url in candidates {
        let mut req = gw
            .client_for(&url)
            .get(&url)
            .header(reqwest::header::ACCEPT, "application/json");
        let Ok(auth_req) = apply_auth(req, &provider.auth, key) else {
            continue;
        };
        req = auth_req;
        let Ok(Ok(resp)) = tokio::time::timeout(Duration::from_secs(8), req.send()).await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(text) = resp.text().await else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let windows = parse_quota_windows(&value);
        if !windows.is_empty() {
            return windows;
        }
    }
    Vec::new()
}

fn parse_quota_windows(value: &Value) -> Vec<crate::snapshot::QuotaWindowView> {
    let mut out = Vec::new();
    collect_quota_windows(value, "", &mut out, 0);
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out.dedup_by(|a, b| a.label == b.label);
    out
}

fn collect_quota_windows(
    value: &Value,
    label: &str,
    out: &mut Vec<crate::snapshot::QuotaWindowView>,
    depth: usize,
) {
    if depth > 5 || out.len() >= 12 {
        return;
    }
    let Some(obj) = value.as_object() else {
        return;
    };
    let label = obj
        .get("label")
        .or_else(|| obj.get("name"))
        .or_else(|| obj.get("period"))
        .and_then(Value::as_str)
        .unwrap_or(label);
    let percent = obj
        .get("used_percent")
        .or_else(|| obj.get("utilization"))
        .or_else(|| obj.get("percent"))
        .or_else(|| obj.get("usage_percent"))
        .and_then(Value::as_f64);
    let limit = obj
        .get("limit")
        .or_else(|| obj.get("total"))
        .or_else(|| obj.get("max"))
        .or_else(|| obj.get("quota"))
        .and_then(Value::as_f64);
    let used = obj
        .get("used")
        .or_else(|| obj.get("usage"))
        .and_then(Value::as_f64);
    let remaining = obj
        .get("remaining")
        .or_else(|| obj.get("left"))
        .and_then(Value::as_f64);
    let reset_at = obj
        .get("reset_at")
        .or_else(|| obj.get("resets_at"))
        .or_else(|| obj.get("reset"))
        .and_then(value_to_timestamp)
        .unwrap_or(0);
    let currency = obj
        .get("currency")
        .and_then(Value::as_str)
        .unwrap_or("USD")
        .to_string();
    let (used_value, limit_value, unit) = if let Some(percent) = percent {
        (percent, 100.0, "percent")
    } else if let Some(limit) = limit {
        let used = used.or_else(|| remaining.map(|r| (limit - r).max(0.0)));
        if let Some(used) = used {
            (used, limit, "money")
        } else {
            (0.0, 0.0, "")
        }
    } else {
        (0.0, 0.0, "")
    };
    if !unit.is_empty() && !label.is_empty() {
        out.push(crate::snapshot::QuotaWindowView {
            label: label.to_string(),
            used: used_value,
            limit: limit_value,
            unit: unit.to_string(),
            currency,
            reset_at,
        });
    }
    for (key, child) in obj {
        if child.is_object() || child.is_array() {
            collect_quota_windows(child, key, out, depth + 1);
        }
    }
}

fn value_to_timestamp(value: &Value) -> Option<i64> {
    if let Some(number) = value.as_f64() {
        let number = number as i64;
        return Some(if number > 10_000_000_000 {
            number / 1000
        } else {
            number
        });
    }
    let text = value.as_str()?;
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|dt| dt.timestamp())
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
    let formula = account.balance.formula.trim();
    let mut amount = if formula.is_empty() {
        let found =
            json_path(&value, path).ok_or_else(|| anyhow!("余额 JSON 路径「{path}」不存在"))?;
        value_to_number(found)?
    } else {
        evaluate_formula(&value, formula)
            .map_err(|err| anyhow!("余额公式「{formula}」计算失败：{err}"))?
    };
    let scale = if account.balance.scale == 0.0 {
        1.0
    } else {
        account.balance.scale
    };
    amount *= scale;
    if !amount.is_finite() {
        bail!("余额计算结果不是有效数字");
    }
    if account.balance.percent_used {
        if account.balance.quota_total <= 0.0 {
            bail!("余额接口返回百分比，但账号没有填写总额度");
        }
        // The endpoint reports how much has been used; convert to remaining
        // money: (100 - used%) / 100 * total.
        let used = amount.clamp(0.0, 100.0);
        amount = (100.0 - used) / 100.0 * account.balance.quota_total;
    }
    Ok(amount)
}

/// Read a number from a JSON value, accepting numeric strings, `$12.34`
/// amounts and objects carrying an `amount` / `balance` field.
pub fn value_to_number(value: &Value) -> Result<f64> {
    match value {
        Value::Number(n) => n.as_f64().ok_or_else(|| anyhow!("余额不是有效数字")),
        Value::String(s) => s
            .trim()
            .trim_start_matches('$')
            .trim_matches('"')
            .replace(',', "")
            .parse::<f64>()
            .map_err(|_| anyhow!("余额不是数字：{s}")),
        Value::Object(obj) => obj
            .get("amount")
            .or_else(|| obj.get("balance"))
            .or_else(|| obj.get("credits"))
            .ok_or_else(|| anyhow!("余额对象缺少 amount 字段"))
            .and_then(value_to_number),
        _ => bail!("余额字段类型不支持"),
    }
}

// ---------------------------------------------------------------------------
// Balance formula
// ---------------------------------------------------------------------------
//
// Small arithmetic language for balance endpoints whose value needs more than
// a single multiplier, for example `data.balance / 100` (cents) or
// `data.total_credits - data.used_credits`. Bare identifiers are JSON paths.

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Path(String),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
}

fn tokenize(input: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        if ch.is_whitespace() {
            index += 1;
            continue;
        }
        match ch {
            '+' => {
                tokens.push(Token::Plus);
                index += 1;
            }
            '-' => {
                tokens.push(Token::Minus);
                index += 1;
            }
            '*' => {
                tokens.push(Token::Star);
                index += 1;
            }
            '/' => {
                tokens.push(Token::Slash);
                index += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                index += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                index += 1;
            }
            _ if ch.is_ascii_digit()
                || (ch == '.'
                    && chars
                        .get(index + 1)
                        .map(|next| next.is_ascii_digit())
                        .unwrap_or(false)) =>
            {
                let start = index;
                let mut seen_dot = false;
                while index < chars.len() {
                    let current = chars[index];
                    if current.is_ascii_digit() {
                        index += 1;
                    } else if current == '.' && !seen_dot {
                        seen_dot = true;
                        index += 1;
                    } else {
                        break;
                    }
                }
                let text: String = chars[start..index].iter().collect();
                tokens.push(Token::Number(
                    text.parse::<f64>()
                        .map_err(|_| anyhow!("无法解析数字「{text}」"))?,
                ));
            }
            _ => {
                let start = index;
                let mut bracket_depth = 0usize;
                while index < chars.len() {
                    let current = chars[index];
                    if bracket_depth == 0
                        && (current.is_whitespace()
                            || matches!(current, '+' | '-' | '*' | '/' | '(' | ')'))
                    {
                        break;
                    }
                    if current == '[' {
                        bracket_depth += 1;
                    } else if current == ']' {
                        bracket_depth = bracket_depth.saturating_sub(1);
                    }
                    index += 1;
                }
                if index == start {
                    bail!("公式中有无法识别的字符「{ch}」");
                }
                let text: String = chars[start..index].iter().collect();
                let text = text.trim();
                if text.is_empty() {
                    bail!("公式中有空的变量名");
                }
                tokens.push(Token::Path(text.to_string()));
            }
        }
    }
    if tokens.is_empty() {
        bail!("公式为空");
    }
    Ok(tokens)
}

struct FormulaParser<'a> {
    tokens: &'a [Token],
    index: usize,
    root: Option<&'a Value>,
}

impl<'a> FormulaParser<'a> {
    fn expression(&mut self) -> Result<f64> {
        let mut value = self.term()?;
        while let Some(token) = self.tokens.get(self.index) {
            match token {
                Token::Plus => {
                    self.index += 1;
                    value += self.term()?;
                }
                Token::Minus => {
                    self.index += 1;
                    value -= self.term()?;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn term(&mut self) -> Result<f64> {
        let mut value = self.factor()?;
        while let Some(token) = self.tokens.get(self.index) {
            match token {
                Token::Star => {
                    self.index += 1;
                    value *= self.factor()?;
                }
                Token::Slash => {
                    self.index += 1;
                    let divisor = self.factor()?;
                    if divisor == 0.0 {
                        bail!("除数不能为 0");
                    }
                    value /= divisor;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn factor(&mut self) -> Result<f64> {
        let Some(token) = self.tokens.get(self.index) else {
            bail!("公式意外结束");
        };
        match token {
            Token::Number(value) => {
                self.index += 1;
                Ok(*value)
            }
            Token::Path(path) => {
                self.index += 1;
                let Some(root) = self.root else {
                    // Syntax check only: paths are assumed resolvable.
                    return Ok(1.0);
                };
                let found =
                    json_path(root, path).ok_or_else(|| anyhow!("JSON 路径「{path}」不存在"))?;
                value_to_number(found)
            }
            Token::Plus => {
                self.index += 1;
                self.factor()
            }
            Token::Minus => {
                self.index += 1;
                Ok(-self.factor()?)
            }
            Token::LParen => {
                self.index += 1;
                let value = self.expression()?;
                if !matches!(self.tokens.get(self.index), Some(Token::RParen)) {
                    bail!("缺少右括号");
                }
                self.index += 1;
                Ok(value)
            }
            Token::RParen => bail!("多余的右括号"),
            Token::Star | Token::Slash => bail!("运算符位置不正确"),
        }
    }

    fn finish(&mut self) -> Result<f64> {
        let value = self.expression()?;
        if self.index != self.tokens.len() {
            bail!("公式末尾存在多余内容");
        }
        Ok(value)
    }
}

/// Evaluate an arithmetic formula where bare identifiers are JSON paths.
pub fn evaluate_formula(root: &Value, formula: &str) -> Result<f64> {
    let tokens = tokenize(formula)?;
    let mut parser = FormulaParser {
        tokens: &tokens,
        index: 0,
        root: Some(root),
    };
    let value = parser.finish()?;
    if !value.is_finite() {
        bail!("计算结果不是有效数字");
    }
    Ok(value)
}

/// Parenthesis/operator syntax check that does not need the response body.
/// Used when saving an account so an invalid formula is caught immediately.
pub fn validate_formula(formula: &str) -> Result<()> {
    let tokens = tokenize(formula)?;
    let mut parser = FormulaParser {
        tokens: &tokens,
        index: 0,
        root: None,
    };
    parser.finish()?;
    Ok(())
}

/// Minimal dot/bracket path lookup, e.g. `data.balance`, `$.data[0].credits`.
pub fn json_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let path = path.trim().trim_start_matches('$').trim_start_matches('.');
    if path.is_empty() {
        return Some(value);
    }
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
        assert_eq!(
            json_path(&v, "$.data.balance").unwrap().as_f64(),
            Some(12.5)
        );
        assert!(json_path(&v, "data.nope").is_none());
    }

    #[test]
    fn formula_arithmetic_and_paths() {
        let v = json!({
            "data": {"balance": 1234, "used": 34, "total": 1000},
            "credits": "1,200.5",
            "nested": [{"amount": "9.5"}]
        });
        assert_eq!(evaluate_formula(&v, "data.balance / 100").unwrap(), 12.34);
        assert_eq!(
            evaluate_formula(&v, "data.total - data.used + 2").unwrap(),
            968.0
        );
        assert_eq!(
            evaluate_formula(&v, "(data.total - data.used) / 2").unwrap(),
            483.0
        );
        assert_eq!(evaluate_formula(&v, "-data.used * 2").unwrap(), -68.0);
        assert_eq!(evaluate_formula(&v, "credits / 2").unwrap(), 600.25);
        assert_eq!(evaluate_formula(&v, "nested[0].amount").unwrap(), 9.5);
    }

    #[test]
    fn formula_rejects_invalid_input() {
        let v = json!({"data": {"balance": 10}});
        assert!(evaluate_formula(&v, "").is_err());
        assert!(evaluate_formula(&v, "data.balance / 0").is_err());
        assert!(evaluate_formula(&v, "data.missing + 1").is_err());
        assert!(evaluate_formula(&v, "(data.balance + 1").is_err());
        assert!(evaluate_formula(&v, "data.balance 1").is_err());
    }

    #[test]
    fn formula_syntax_validation() {
        assert!(validate_formula("(data.total - data.used) / 100").is_ok());
        assert!(validate_formula("data.balance / 100").is_ok());
        assert!(validate_formula("(data.balance + 1").is_err());
        assert!(validate_formula("data.balance /").is_err());
        assert!(validate_formula("").is_err());
    }

    #[test]
    fn parses_common_quota_window_shapes() {
        let value = json!({
            "five_hour": {"used_percent": 25, "resets_at": "2026-10-23T18:00:00Z"},
            "weekly": {"used": 30, "limit": 100},
            "monthly": {"used": 60, "limit": 100}
        });
        let windows = parse_quota_windows(&value);
        assert_eq!(windows.len(), 3);
        assert!(windows
            .iter()
            .any(|w| w.label == "five_hour" && w.used == 25.0));
        assert!(windows
            .iter()
            .any(|w| w.label == "monthly" && w.limit == 100.0));
    }
}
