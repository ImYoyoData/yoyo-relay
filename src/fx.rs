//! Live exchange rates.
//!
//! YoyoRelay keeps the rate table in the user configuration and refreshes it
//! from free, key-less public endpoints. The table is always USD-based
//! (`units of currency per 1 USD`), matching `UiConfig::fx_rate`.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct FxRates {
    /// Units of each currency per 1 USD. Always contains USD = 1.
    pub rates: BTreeMap<String, f64>,
    pub source: String,
}

const ENDPOINTS: &[(&str, &str)] = &[
    (
        "https://api.frankfurter.app/latest?from=USD",
        "Frankfurter · 欧洲央行参考汇率",
    ),
    ("https://open.er-api.com/v6/latest/USD", "open.er-api.com"),
];

pub async fn fetch_rates(client: &reqwest::Client) -> Result<FxRates> {
    let mut errors = Vec::new();
    for (url, source) in ENDPOINTS {
        match fetch_one(client, url, source).await {
            Ok(rates) => return Ok(rates),
            Err(err) => errors.push(format!("{source}：{err:#}")),
        }
    }
    bail!("无法获取实时汇率（{}）", errors.join("；"))
}

async fn fetch_one(client: &reqwest::Client, url: &str, source: &str) -> Result<FxRates> {
    let response = tokio::time::timeout(Duration::from_secs(15), client.get(url).send())
        .await
        .context("请求超时")?
        .context("请求失败")?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("HTTP {status}");
    }
    let value: Value = serde_json::from_str(&text).context("响应不是 JSON")?;
    let rates = parse_rates(&value)?;
    Ok(FxRates {
        rates,
        source: source.to_string(),
    })
}

/// Parse either a Frankfurter-style `{ "rates": { ... } }` payload or an
/// open.er-api payload. Unknown/malformed entries are ignored.
pub fn parse_rates(value: &Value) -> Result<BTreeMap<String, f64>> {
    if let Some(result) = value.get("result").and_then(Value::as_str) {
        if result != "success" {
            bail!("汇率服务返回 {result}");
        }
    }
    if let Some(base) = value.get("base").and_then(Value::as_str) {
        if !base.eq_ignore_ascii_case("USD") {
            bail!("汇率基准不是 USD（{base}）");
        }
    }
    let Some(raw) = value.get("rates").and_then(Value::as_object) else {
        bail!("响应缺少 rates 字段");
    };
    let mut rates = BTreeMap::new();
    rates.insert("USD".to_string(), 1.0);
    for (currency, amount) in raw {
        let Some(rate) = amount
            .as_f64()
            .filter(|rate| rate.is_finite() && *rate > 0.0)
        else {
            continue;
        };
        let currency = crate::config::normalize_currency(currency);
        if currency.len() < 2 || currency.len() > 8 {
            continue;
        }
        rates.insert(currency, rate);
    }
    if rates.len() <= 1 {
        bail!("汇率响应不含有效币种");
    }
    if !rates.contains_key("CNY") {
        // The balance page always shows a CNY total; a missing CNY rate is a
        // hard failure so the UI can keep the previous table.
        return Err(anyhow!("汇率响应缺少 CNY"));
    }
    Ok(rates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_frankfurter_payload() {
        let value = json!({
            "amount": 1.0,
            "base": "USD",
            "date": "2026-10-22",
            "rates": {"CNY": 7.12, "EUR": 0.92, "BAD": 0.0}
        });
        let rates = parse_rates(&value).unwrap();
        assert_eq!(rates.get("USD"), Some(&1.0));
        assert_eq!(rates.get("CNY"), Some(&7.12));
        assert!(!rates.contains_key("BAD"));
    }

    #[test]
    fn parses_er_api_payload_and_rejects_bad_bases() {
        let value = json!({
            "result": "success",
            "base_code": "USD",
            "rates": {"USD": 1, "CNY": 7.2, "JPY": 150.2}
        });
        let rates = parse_rates(&value).unwrap();
        assert_eq!(rates.get("CNY"), Some(&7.2));
        assert_eq!(rates.get("JPY"), Some(&150.2));

        let wrong = json!({"base": "EUR", "rates": {"CNY": 7.2}});
        assert!(parse_rates(&wrong).is_err());
        let missing = json!({"result": "success", "rates": {"USD": 1}});
        assert!(parse_rates(&missing).is_err());
    }
}
