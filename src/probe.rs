//! Provider / account / route / local-key test probes.
//!
//! These are diagnostics only: they never write to the usage database.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::config::{now_ts, Account, Protocol, Provider};
use crate::gateway::{apply_auth, readable_reqwest_error, upstream_error_message, Gateway};
use futures_util::StreamExt;

use crate::protocol::{
    build_upstream_body, extract_nonstream_usage, route_supports_ingress, Ingress, SseDecoder,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestLevel {
    Success,
    Warning,
    AuthFailed,
    RateLimited,
    Unreachable,
    Error,
}

impl TestLevel {
    pub fn label(self) -> &'static str {
        match self {
            TestLevel::Success => "可用",
            TestLevel::Warning => "警告",
            TestLevel::AuthFailed => "鉴权失败",
            TestLevel::RateLimited => "限流",
            TestLevel::Unreachable => "不可达",
            TestLevel::Error => "失败",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TestResult {
    pub key: String,
    pub kind: String,
    pub title: String,
    pub level: TestLevel,
    pub status: Option<u16>,
    pub latency_ms: u64,
    pub message: String,
    pub detail: String,
    pub models: Vec<String>,
    pub tokens: Option<u64>,
    /// Measured decode throughput (completion tokens per second) for a speed
    /// test. `None` for ordinary connectivity probes.
    pub tokens_per_sec: Option<f64>,
    /// Time to first streamed chunk, in milliseconds.
    pub first_token_ms: Option<u64>,
    pub at: i64,
}

impl TestResult {
    fn new(key: &str, kind: &str, title: &str) -> Self {
        Self {
            key: key.to_string(),
            kind: kind.to_string(),
            title: title.to_string(),
            level: TestLevel::Error,
            status: None,
            latency_ms: 0,
            message: String::new(),
            detail: String::new(),
            models: Vec::new(),
            tokens: None,
            tokens_per_sec: None,
            first_token_ms: None,
            at: now_ts(),
        }
    }
}

struct ProbeResponse {
    status: Option<u16>,
    latency_ms: u64,
    body: String,
    error: Option<String>,
}

async fn send_probe(
    gw: &Gateway,
    provider: &Provider,
    key: &str,
    method: &str,
    url: &str,
    body: Option<&Value>,
    timeout_secs: u64,
) -> ProbeResponse {
    let started = Instant::now();
    // Both verbs must honour the loopback bypass; local model servers behind
    // the system proxy used to fail their POST probes.
    let client = gw.client_for(url);
    let mut req = if method == "GET" {
        client.get(url)
    } else {
        client.post(url)
    };
    req = req.header(reqwest::header::ACCEPT, "application/json");
    if let Some(body) = body {
        req = req
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(body);
    }
    req = match apply_auth(req, &provider.auth, key) {
        Ok(r) => r,
        Err(err) => {
            return ProbeResponse {
                status: None,
                latency_ms: started.elapsed().as_millis() as u64,
                body: String::new(),
                error: Some(format!("鉴权头配置无效：{err}")),
            }
        }
    };
    for h in &provider.extra_headers {
        if !h.name.trim().is_empty() {
            req = req.header(h.name.trim(), h.value.as_str());
        }
    }
    if !provider
        .extra_headers
        .iter()
        .any(|h| h.name.eq_ignore_ascii_case("anthropic-version"))
    {
        req = req.header("anthropic-version", "2023-06-01");
    }
    let resp = tokio::time::timeout(Duration::from_secs(timeout_secs.max(3)), req.send()).await;
    match resp {
        Err(_) => ProbeResponse {
            status: None,
            latency_ms: started.elapsed().as_millis() as u64,
            body: String::new(),
            error: Some(format!("连接超时（{timeout_secs}s）")),
        },
        Ok(Err(err)) => ProbeResponse {
            status: None,
            latency_ms: started.elapsed().as_millis() as u64,
            body: String::new(),
            error: Some(readable_reqwest_error(&err)),
        },
        Ok(Ok(resp)) => {
            let status = resp.status().as_u16();
            let body = tokio::time::timeout(Duration::from_secs(15), resp.text())
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or_default();
            ProbeResponse {
                status: Some(status),
                latency_ms: started.elapsed().as_millis() as u64,
                body,
                error: None,
            }
        }
    }
}

fn parse_models(body: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    value
        .get("data")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn models_endpoint(provider: &Provider) -> String {
    provider.models_url()
}

fn default_test_model(provider: &Provider) -> String {
    if let Some(m) = &provider.test_model {
        if !m.trim().is_empty() {
            return m.clone();
        }
    }
    match provider.static_protocol {
        Protocol::AnthropicMessages => "claude-3-5-haiku-20241022".into(),
        Protocol::OpenAiResponses => "gpt-4o-mini".into(),
        Protocol::OpenAiChat => "gpt-4o-mini".into(),
    }
}

fn minimal_body(protocol: Protocol, model: &str) -> Value {
    match protocol {
        Protocol::OpenAiChat => json!({
            "model": model,
            "messages": [{"role": "user", "content": "ping"}],
            "max_tokens": 1,
            "stream": false
        }),
        Protocol::AnthropicMessages => json!({
            "model": model,
            "messages": [{"role": "user", "content": "ping"}],
            "max_tokens": 1
        }),
        Protocol::OpenAiResponses => json!({
            "model": model,
            "input": "ping",
            "max_output_tokens": 16,
            "stream": false
        }),
    }
}

fn minimal_url(provider: &Provider, protocol: Protocol) -> String {
    let path = match protocol {
        Protocol::OpenAiChat => "chat/completions",
        Protocol::AnthropicMessages => "messages",
        Protocol::OpenAiResponses => "responses",
    };
    provider.join_url_for(protocol, path)
}

const AI_PRICING_SYSTEM_PROMPT: &str = r#"你是 YoyoRelay 的 LLM API 计费配置助手。根据用户的需求生成一个价格方案。
只返回一个 JSON 对象，不要 Markdown、不要解释、不要代码围栏。

JSON 结构：
{
  "name": "方案名称",
  "note": "对假设和未提及价格的说明",
  "currency": "CNY" 或 "USD",
  "base": {
    "input_per_mtok": 0,
    "cached_input_per_mtok": 0,
    "output_per_mtok": 0
  },
  "periods": [
    {
      "name": "高峰",
      "start_hour": 9,
      "end_hour": 18,
      "days": [0, 1, 2, 3, 4],
      "tier": {
        "input_per_mtok": 0,
        "cached_input_per_mtok": 0,
        "output_per_mtok": 0
      }
    }
  ]
}

规则：
1. base 是「未命中任何时段」时的默认价，通常就是用户说的低谷/空闲价。periods 只放用户明确说的高峰时段；不要为低谷时间创建时段。
2. 价格单位统一为「每百万 tokens」。用户说“每千 tokens”时乘以 1000，说“每 token”时乘以 1,000,000。
3. days 用 0=周一、1=周二……6=周日；空数组表示每天。只提到“工作日”时用 [0,1,2,3,4]。
4. start_hour 为 0–23，end_hour 为 0–24；end_hour 小于 start_hour 表示跨午夜。整天的时段用 start_hour=end_hour。
5. 用户明确给出的数字必须原样使用；没有提到的字段填 0，并在 note 中说明。
6. 如果用户只描述高峰价，没有描述低谷价，base 全部填 0，并在 note 中提示需要补充默认价。
7. 用户使用“元、¥、人民币”时 currency=CNY，否则 currency=USD。
8. 方案名称要简短，例如“工作日峰谷价”。"#;

/// Ask the route's upstream model to turn a natural-language pricing request
/// into the JSON shape consumed by `PricePlan`.
pub async fn generate_price_plan(
    gw: &Gateway,
    provider_id: &str,
    model: &str,
    prompt: &str,
) -> Result<Value, String> {
    let config = gw.config();
    let provider = config
        .provider(provider_id)
        .cloned()
        .ok_or_else(|| "供应商不存在".to_string())?;
    if provider.base_url.starts_with("plugin://") {
        return Err("临时供应商没有可直接调用的账号，请选择真实上游再生成。".into());
    }
    let account = config
        .accounts
        .iter()
        .find(|account| account.provider_id == provider_id && account.enabled)
        .cloned()
        .ok_or_else(|| "该供应商没有启用的账号，无法调用 AI。".to_string())?;
    let key = if account.api_key.trim().is_empty() {
        String::new()
    } else {
        gw.master
            .decrypt(&account.api_key)
            .map_err(|err| format!("读取账号 Key 失败：{err}"))?
    };
    let protocol = provider.protocol_for(model, None);
    let body = ai_pricing_body(protocol, model, prompt);
    let url = minimal_url(&provider, protocol);
    let timeout = provider.timeout_secs.unwrap_or(120).clamp(30, 300);
    let response = send_probe(gw, &provider, &key, "POST", &url, Some(&body), timeout).await;
    if let Some(err) = response.error {
        return Err(err);
    }
    let status = response.status.unwrap_or(0);
    if !(200..300).contains(&status) {
        let snippet: String = response.body.chars().take(300).collect();
        return Err(format!("上游返回 HTTP {status}：{snippet}"));
    }
    let text = ai_response_text(protocol, &response.body)?;
    extract_json_object(&text).ok_or_else(|| {
        format!(
            "AI 没有返回有效 JSON：{}",
            text.chars().take(200).collect::<String>()
        )
    })
}

fn ai_pricing_body(protocol: Protocol, model: &str, prompt: &str) -> Value {
    match protocol {
        Protocol::OpenAiChat => json!({
            "model": model,
            "messages": [
                {"role": "system", "content": AI_PRICING_SYSTEM_PROMPT},
                {"role": "user", "content": prompt}
            ],
            "temperature": 0.2,
            "max_tokens": 1600,
            "stream": false
        }),
        Protocol::AnthropicMessages => json!({
            "model": model,
            "max_tokens": 1600,
            "system": AI_PRICING_SYSTEM_PROMPT,
            "messages": [{"role": "user", "content": prompt}]
        }),
        Protocol::OpenAiResponses => json!({
            "model": model,
            "instructions": AI_PRICING_SYSTEM_PROMPT,
            "input": prompt,
            "max_output_tokens": 1600,
            "stream": false
        }),
    }
}

fn ai_response_text(protocol: Protocol, body: &str) -> Result<String, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|err| format!("解析上游响应失败：{err}"))?;
    let text = match protocol {
        Protocol::OpenAiChat => value
            .pointer("/choices/0/message/content")
            .and_then(|content| {
                content.as_str().map(str::to_string).or_else(|| {
                    content.as_array().map(|parts| {
                        parts
                            .iter()
                            .filter_map(|part| part.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("")
                    })
                })
            })
            .unwrap_or_default(),
        Protocol::AnthropicMessages => value
            .get("content")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default(),
        Protocol::OpenAiResponses => value
            .get("output_text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                value.get("output").and_then(Value::as_array).map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("content").and_then(Value::as_array))
                        .flatten()
                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("")
                })
            })
            .unwrap_or_default(),
    };
    if text.trim().is_empty() {
        return Err(format!(
            "AI 响应没有文本内容：{}",
            body.chars().take(200).collect::<String>()
        ));
    }
    Ok(text)
}

fn extract_json_object(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    let without_fence = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```JSON"))
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|inner| inner.trim_start())
        .unwrap_or(trimmed)
        .trim_end_matches("```")
        .trim();
    if let Ok(value) = serde_json::from_str::<Value>(without_fence) {
        return Some(value);
    }
    let start = without_fence.find('{')?;
    let end = without_fence.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&without_fence[start..=end]).ok()
}

async fn probe_credentials(
    gw: &Gateway,
    provider: &Provider,
    key: &str,
    model_hint: Option<&str>,
    allow_models_endpoint: bool,
) -> ProbeResultData {
    if key.trim().is_empty() && provider.auth.mode != crate::config::AuthMode::None {
        // Still try: some local gateways don't require auth; report auth failure if rejected.
    }
    let timeout = provider.timeout_secs.unwrap_or(15).max(5);
    let mut detail = String::new();
    if allow_models_endpoint {
        let url = models_endpoint(provider);
        let resp = send_probe(gw, provider, key, "GET", &url, None, timeout).await;
        detail.push_str(&format!(
            "GET {url} → {}\n",
            resp.status
                .map(|s| s.to_string())
                .unwrap_or_else(|| "连接失败".into())
        ));
        if let Some(err) = &resp.error {
            return ProbeResultData {
                level: TestLevel::Unreachable,
                status: None,
                latency_ms: resp.latency_ms,
                message: err.clone(),
                detail,
                models: Vec::new(),
                tokens: None,
            };
        }
        match resp.status {
            Some(200) => {
                let models = parse_models(&resp.body);
                let message = if models.is_empty() {
                    "连接成功，鉴权有效（未返回模型列表）".to_string()
                } else {
                    format!("连接成功，鉴权有效，返回 {} 个模型", models.len())
                };
                detail.push_str(&format!("延迟 {}ms\n", resp.latency_ms));
                return ProbeResultData {
                    level: TestLevel::Success,
                    status: Some(200),
                    latency_ms: resp.latency_ms,
                    message,
                    detail,
                    models,
                    tokens: None,
                };
            }
            Some(401) | Some(403) => {
                return ProbeResultData {
                    level: TestLevel::AuthFailed,
                    status: resp.status,
                    latency_ms: resp.latency_ms,
                    message: "Key 无效或无权限".into(),
                    detail,
                    models: Vec::new(),
                    tokens: None,
                };
            }
            Some(429) => {
                return ProbeResultData {
                    level: TestLevel::RateLimited,
                    status: resp.status,
                    latency_ms: resp.latency_ms,
                    message: "当前被限流，冷却后可恢复".into(),
                    detail,
                    models: Vec::new(),
                    tokens: None,
                };
            }
            Some(404) | Some(405) | None => {
                detail.push_str("该端点不支持 /models，改用最小请求探测\n");
            }
            Some(status) if status >= 500 => {
                return ProbeResultData {
                    level: TestLevel::Error,
                    status: Some(status),
                    latency_ms: resp.latency_ms,
                    message: format!("上游服务器错误（HTTP {status}）"),
                    detail,
                    models: Vec::new(),
                    tokens: None,
                };
            }
            Some(_) => {
                detail.push_str("改用最小请求探测\n");
            }
        }
    }

    let model = model_hint
        .map(str::to_string)
        .unwrap_or_else(|| default_test_model(provider));
    let protocol = provider.protocol_for(&model, None);
    let body = minimal_body(protocol, &model);
    let url = minimal_url(provider, protocol);
    let resp = send_probe(gw, provider, key, "POST", &url, Some(&body), timeout).await;
    detail.push_str(&format!(
        "POST {url} → {}\n",
        resp.status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "连接失败".into())
    ));
    if let Some(err) = &resp.error {
        return ProbeResultData {
            level: TestLevel::Unreachable,
            status: None,
            latency_ms: resp.latency_ms,
            message: err.clone(),
            detail,
            models: Vec::new(),
            tokens: None,
        };
    }
    let status = resp.status.unwrap_or(0);
    let usage = serde_json::from_str::<Value>(&resp.body)
        .map(|v| extract_nonstream_usage(protocol, &v))
        .ok();
    match status {
        200..=299 => {
            detail.push_str(&format!(
                "延迟 {}ms\n协议 {}\n模型 {}\n",
                resp.latency_ms,
                protocol.label(),
                model
            ));
            if let Some(u) = usage {
                detail.push_str(&format!(
                    "消耗 {} tokens（测试请求，不计入额度）\n",
                    u.total()
                ));
            }
            ProbeResultData {
                level: TestLevel::Success,
                status: Some(status),
                latency_ms: resp.latency_ms,
                message: format!("可用（HTTP {status}）"),
                detail,
                models: Vec::new(),
                tokens: usage.map(|u| u.total()),
            }
        }
        401 | 403 => ProbeResultData {
            level: TestLevel::AuthFailed,
            status: Some(status),
            latency_ms: resp.latency_ms,
            message: "Key 无效或无权限".into(),
            detail,
            models: Vec::new(),
            tokens: None,
        },
        429 => ProbeResultData {
            level: TestLevel::RateLimited,
            status: Some(status),
            latency_ms: resp.latency_ms,
            message: "当前被限流，冷却后可恢复".into(),
            detail,
            models: Vec::new(),
            tokens: None,
        },
        404 => ProbeResultData {
            level: TestLevel::Error,
            status: Some(status),
            latency_ms: resp.latency_ms,
            message: format!("模型或路径不存在：{}", upstream_error_message(&resp.body)),
            detail,
            models: Vec::new(),
            tokens: None,
        },
        _ => ProbeResultData {
            level: TestLevel::Error,
            status: Some(status),
            latency_ms: resp.latency_ms,
            message: format!("HTTP {status}：{}", upstream_error_message(&resp.body)),
            detail,
            models: Vec::new(),
            tokens: None,
        },
    }
}

struct ProbeResultData {
    level: TestLevel,
    status: Option<u16>,
    latency_ms: u64,
    message: String,
    detail: String,
    models: Vec<String>,
    tokens: Option<u64>,
}

pub async fn test_provider(gw: &Gateway, provider_id: &str) -> TestResult {
    let cfg = gw.config();
    let mut result = TestResult::new(&format!("provider:{provider_id}"), "provider", "供应商测试");
    let Some(provider) = cfg.provider(provider_id).cloned() else {
        result.message = "供应商不存在".into();
        return result;
    };
    result.title = provider.name.clone();

    let mut accounts = cfg.accounts_of(&provider.id);
    accounts.retain(|a| a.enabled);
    accounts.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.created_at.cmp(&b.created_at))
    });
    let model_hint = provider.test_model.clone();
    let mut attempts: Vec<String> = Vec::new();
    let mut best: Option<ProbeResultData> = None;

    if accounts.is_empty() {
        let data = probe_credentials(gw, &provider, "", model_hint.as_deref(), true).await;
        attempts.push(format!("无账号凭据（无鉴权）：{}", data.message));
        best = Some(data);
    } else {
        for account in &accounts {
            let key = match gw.master.decrypt(&account.api_key) {
                Ok(k) => k,
                Err(err) => {
                    attempts.push(format!("账号「{}」：Key 无法解密（{err}）", account.name));
                    continue;
                }
            };
            let key_mask = crate::config::mask_secret(&key);
            let data = probe_credentials(gw, &provider, &key, model_hint.as_deref(), true).await;
            attempts.push(format!(
                "账号「{}」({key_mask})：{} — {}",
                account.name,
                data.level.label(),
                data.message
            ));
            if data.level == TestLevel::Success {
                best = Some(data);
                break;
            }
            let rank = |level: TestLevel| match level {
                TestLevel::Success => 5,
                TestLevel::Unreachable => 4,
                TestLevel::Error => 3,
                TestLevel::AuthFailed => 2,
                TestLevel::Warning | TestLevel::RateLimited => 1,
            };
            if best
                .as_ref()
                .map(|current| rank(data.level) > rank(current.level))
                .unwrap_or(true)
            {
                best = Some(data);
            }
        }
    }

    let Some(data) = best else {
        result.level = TestLevel::Error;
        result.message = "没有可用的测试凭据".into();
        result.detail = attempts.join("\n");
        return result;
    };
    result.level = data.level;
    result.status = data.status;
    result.latency_ms = data.latency_ms;
    result.message = data.message;
    result.models = data.models;
    result.tokens = data.tokens;
    result.detail = format!(
        "供应商: {}\nBase URL: {}\n尝试记录:\n{}\n{}\n延迟: {}ms",
        provider.name,
        provider.base_url,
        attempts.join("\n"),
        data.detail.trim_end(),
        data.latency_ms
    );
    result
}

pub async fn test_account(gw: &Gateway, account_id: &str) -> TestResult {
    let cfg = gw.config();
    let mut result = TestResult::new(&format!("account:{account_id}"), "account", "账号测试");
    let Some(account) = cfg.account(account_id).cloned() else {
        result.message = "账号不存在".into();
        return result;
    };
    result.title = account.name.clone();
    let Some(provider) = cfg.provider(&account.provider_id).cloned() else {
        result.message = "账号所属供应商不存在".into();
        return result;
    };
    let key = match gw.master.decrypt(&account.api_key) {
        Ok(k) => k,
        Err(err) => {
            result.message = format!("Key 无法解密：{err}");
            return result;
        }
    };
    let model_hint = cfg
        .routes
        .iter()
        .find(|r| r.provider_id == provider.id)
        .map(|r| r.upstream_model.clone())
        .or_else(|| provider.test_model.clone());
    let data = probe_credentials(gw, &provider, &key, model_hint.as_deref(), true).await;
    result.level = data.level;
    result.status = data.status;
    result.latency_ms = data.latency_ms;
    result.message = data.message;
    result.models = data.models;
    result.tokens = data.tokens;
    result.detail = format!(
        "供应商: {}\n账号: {}\nKey: {}\n{}\n延迟: {}ms",
        provider.name,
        account.name,
        crate::config::mask_secret(&key),
        data.detail.trim_end(),
        data.latency_ms
    );
    result
}

/// Test a not-yet-saved account draft. Nothing is persisted and no usage is recorded.
pub async fn test_account_draft(
    gw: &Gateway,
    provider_id: &str,
    name: &str,
    api_key: &str,
    model_hint: Option<String>,
) -> TestResult {
    let cfg = gw.config();
    let mut result = TestResult::new("draft:account", "draft", "账号预检");
    result.title = if name.trim().is_empty() {
        "未命名账号".to_string()
    } else {
        name.trim().to_string()
    };
    let Some(provider) = cfg.provider(provider_id).cloned() else {
        result.message = "账号所属供应商不存在".into();
        return result;
    };
    if api_key.trim().is_empty() && provider.auth.mode != crate::config::AuthMode::None {
        result.level = TestLevel::Error;
        result.message = "请先填写 API Key 再测试。".into();
        result.detail = format!(
            "供应商: {}
Key: 未填写",
            provider.name
        );
        return result;
    }
    let data = probe_credentials(gw, &provider, api_key.trim(), model_hint.as_deref(), true).await;
    result.level = data.level;
    result.status = data.status;
    result.latency_ms = data.latency_ms;
    result.message = data.message;
    result.models = data.models;
    result.tokens = data.tokens;
    result.detail = format!(
        "供应商: {}
账号: {}
Key: {}
{}
延迟: {}ms
（预检请求，不计入额度）",
        provider.name,
        result.title,
        if api_key.trim().is_empty() {
            "无".to_string()
        } else {
            crate::config::mask_secret(api_key.trim())
        },
        data.detail.trim_end(),
        data.latency_ms
    );
    result
}

pub async fn test_route(gw: &Gateway, route_id: &str) -> TestResult {
    let cfg = gw.config();
    let mut result = TestResult::new(&format!("route:{route_id}"), "route", "路由测试");
    let Some(route) = cfg.route(route_id).cloned() else {
        result.message = "路由不存在".into();
        return result;
    };
    result.title = format!("{} → {}", route.client_model, route.upstream_model);
    let Some(provider) = cfg.provider(&route.provider_id).cloned() else {
        result.message = "路由引用的供应商不存在".into();
        return result;
    };
    let protocol = provider.protocol_for(&route.upstream_model, route.protocol);
    let ingress = match protocol {
        Protocol::AnthropicMessages => Ingress::Anthropic,
        Protocol::OpenAiResponses => Ingress::Responses,
        Protocol::OpenAiChat => Ingress::Chat,
    };
    let client_body = match ingress {
        Ingress::Chat => json!({
            "model": route.client_model,
            "messages": [{"role":"user","content":"ping"}],
            "max_tokens": 1,
            "stream": false
        }),
        Ingress::Anthropic => json!({
            "model": route.client_model,
            "messages": [{"role":"user","content":"ping"}],
            "max_tokens": 1
        }),
        Ingress::Responses => json!({
            "model": route.client_model,
            "input": "ping",
            "max_output_tokens": 16,
            "stream": false
        }),
        Ingress::Generic => json!({"model": route.client_model}),
    };
    let upstream_body =
        match build_upstream_body(ingress, protocol, &client_body, &route.upstream_model) {
            Ok(v) => v,
            Err(err) => {
                result.message = format!("协议转换失败：{err}");
                return result;
            }
        };
    let url = crate::gateway::upstream_url_public(&provider, protocol);
    let mut preview = format!(
        "客户端模型: {}\n→ 供应商: {}\n→ 上游模型: {}\n→ 协议: {}\n→ 路径: POST {}",
        route.client_model,
        provider.name,
        route.upstream_model,
        protocol.label(),
        url
    );
    if !route_supports_ingress(&route, ingress, protocol) {
        result.message = format!(
            "该路由的协议组合（{} → {}）当前不支持测试",
            ingress.protocol().map(|p| p.label()).unwrap_or("generic"),
            protocol.label()
        );
        result.detail = preview;
        return result;
    }

    let mut accounts = cfg.accounts_of(&provider.id);
    accounts.retain(|a| a.enabled);
    accounts.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.created_at.cmp(&b.created_at))
    });
    if accounts.is_empty() {
        result.message = "该供应商没有启用的账号".into();
        preview.push_str("\n→ 账号: 无");
        result.detail = preview;
        return result;
    }

    let timeout = provider.timeout_secs.unwrap_or(30).max(5);
    let mut last_failure: Option<ProbeResultData> = None;
    let mut hard_stop = false;
    for account in &accounts {
        let state = gw
            .scheduler
            .state_of(account, chrono::Utc::now().timestamp());
        let key = match gw.master.decrypt(&account.api_key) {
            Ok(k) => k,
            Err(err) => {
                preview.push_str(&format!(
                    "\n→ 账号「{}」：Key 无法解密（{err}）",
                    account.name
                ));
                continue;
            }
        };
        let started = Instant::now();
        let mut req = gw
            .client_for(&url)
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "application/json")
            .json(&upstream_body);
        req = match apply_auth(req, &provider.auth, &key) {
            Ok(r) => r,
            Err(err) => {
                preview.push_str(&format!(
                    "\n→ 账号「{}」：鉴权头配置无效（{err}）",
                    account.name
                ));
                continue;
            }
        };
        for h in &provider.extra_headers {
            if !h.name.trim().is_empty() {
                req = req.header(h.name.trim(), h.value.as_str());
            }
        }
        if protocol == Protocol::AnthropicMessages
            && !provider
                .extra_headers
                .iter()
                .any(|h| h.name.eq_ignore_ascii_case("anthropic-version"))
        {
            req = req.header("anthropic-version", "2023-06-01");
        }
        let sent = tokio::time::timeout(Duration::from_secs(timeout), req.send()).await;
        let (status, body, error, latency) = match sent {
            Err(_) => (
                None,
                String::new(),
                Some(format!("连接超时（{timeout}s）")),
                started.elapsed().as_millis() as u64,
            ),
            Ok(Err(err)) => (
                None,
                String::new(),
                Some(readable_reqwest_error(&err)),
                started.elapsed().as_millis() as u64,
            ),
            Ok(Ok(resp)) => {
                let status = resp.status().as_u16();
                let body = tokio::time::timeout(Duration::from_secs(30), resp.text())
                    .await
                    .ok()
                    .and_then(|r| r.ok())
                    .unwrap_or_default();
                (
                    Some(status),
                    body,
                    None,
                    started.elapsed().as_millis() as u64,
                )
            }
        };
        let label = format!("账号「{}」({})", account.name, state.badge());
        if let Some(err) = error {
            preview.push_str(&format!("\n→ {label}: ❌ {err}"));
            result.level = TestLevel::Unreachable;
            result.latency_ms = latency;
            result.message = err.clone();
            hard_stop = true;
            break;
        }
        let status = status.unwrap_or(0);
        let usage = serde_json::from_str::<Value>(&body)
            .ok()
            .map(|v| extract_nonstream_usage(protocol, &v));
        let token_text = usage
            .map(|u| format!(" · 消耗 {} tokens", u.total()))
            .unwrap_or_default();
        if (200..300).contains(&status) {
            preview.push_str(&format!(
                "\n→ {label}: ✅ HTTP {status} · 延迟 {latency}ms{token_text}"
            ));
            result.level = TestLevel::Success;
            result.status = Some(status);
            result.latency_ms = latency;
            result.tokens = usage.map(|u| u.total());
            result.message = format!("HTTP 200 · 延迟 {latency}ms{token_text}");
            preview.push_str("\n结果: ✅ 测试请求成功（不计入额度）");
            result.detail = preview;
            return result;
        }
        preview.push_str(&format!(
            "\n→ {label}: ❌ HTTP {status} · {}",
            upstream_error_message(&body)
        ));
        let data = ProbeResultData {
            level: match status {
                401 | 403 => TestLevel::AuthFailed,
                429 => TestLevel::RateLimited,
                _ => TestLevel::Error,
            },
            status: Some(status),
            latency_ms: latency,
            message: format!("HTTP {status}：{}", upstream_error_message(&body)),
            detail: String::new(),
            models: Vec::new(),
            tokens: usage.map(|u| u.total()),
        };
        result.status = Some(status);
        result.latency_ms = latency;
        result.message = data.message.clone();
        result.level = data.level;
        result.tokens = data.tokens;
        if status == 400 || status == 413 || status == 422 {
            hard_stop = true;
            break;
        }
        last_failure = Some(data);
    }
    if hard_stop {
        result.detail = preview;
        return result;
    }
    if let Some(data) = last_failure {
        result.level = data.level;
        result.status = data.status;
        result.latency_ms = data.latency_ms;
        result.message = data.message;
        result.tokens = data.tokens;
    }
    preview.push_str("\n结果: ❌ 所有可用账号的测试均失败");
    result.detail = preview;
    result
}

pub async fn test_local_key(gw: &Gateway, key_id: &str) -> TestResult {
    let cfg = gw.config();
    let mut result = TestResult::new(&format!("local_key:{key_id}"), "local_key", "本地 Key 测试");
    let Some(key) = cfg.local_key(key_id).cloned() else {
        result.message = "本地 Key 不存在".into();
        return result;
    };
    result.title = key.name.clone();
    let plain = match gw.master.decrypt(&key.key) {
        Ok(k) => k,
        Err(err) => {
            result.message = format!("Key 无法解密：{err}");
            return result;
        }
    };
    let endpoint = gw.models_url();
    let started = Instant::now();
    let resp = gw
        .client_for(&endpoint)
        .get(&endpoint)
        .bearer_auth(&plain)
        .timeout(Duration::from_secs(10))
        .send()
        .await;
    let resp = match resp {
        Ok(r) => r,
        Err(err) => {
            result.level = TestLevel::Unreachable;
            result.message = format!("代理未运行或端口未监听：{}", readable_reqwest_error(&err));
            result.detail = format!("GET {endpoint}\n{err}");
            return result;
        }
    };
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    let latency = started.elapsed().as_millis() as u64;
    result.status = Some(status);
    result.latency_ms = latency;
    if status == 401 {
        result.level = TestLevel::AuthFailed;
        result.message = "本地 Key 鉴权失败".into();
        result.detail = format!("GET {endpoint} → 401\n{body}");
        return result;
    }
    if status != 200 {
        result.level = TestLevel::Error;
        result.message = format!("/v1/models 返回 HTTP {status}");
        result.detail = format!("GET {endpoint} → {status}\n{body}");
        return result;
    }
    let models = parse_models(&body);
    let mut detail = format!(
        "GET {endpoint} → 200\n延迟: {latency}ms\n可见模型: {}\n",
        if models.is_empty() {
            "无".to_string()
        } else {
            models.join(", ")
        }
    );
    let mut level = TestLevel::Success;
    let mut message = format!("本地 Key 可用，可见 {} 个模型", models.len());
    if cfg.server.auth_enabled {
        if !key.allowed_models.is_empty() && !key.allowed_models.iter().any(|m| m == "*") {
            let probe_model = "__yoyo_disallowed_probe__";
            if key.allows_model(probe_model) {
                detail.push_str("白名单包含通配符，跳过拒绝校验\n");
            } else {
                let url = format!("{}/v1/chat/completions", gw.endpoint());
                let probe = gw
                    .client_for(&url)
                    .post(&url)
                    .bearer_auth(&plain)
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .json(&json!({
                        "model": probe_model,
                        "messages": [{"role":"user","content":"x"}],
                        "max_tokens": 1
                    }))
                    .timeout(Duration::from_secs(10))
                    .send()
                    .await;
                match probe {
                    Ok(r) if r.status().as_u16() == 403 => {
                        detail.push_str("白名单拒绝校验：✅ 403（未触达上游）\n");
                    }
                    Ok(r) => {
                        level = TestLevel::Warning;
                        message = format!("Key 可用，但白名单校验返回 HTTP {}", r.status());
                        detail.push_str(&format!(
                            "白名单拒绝校验：⚠️ 期望 403，实际 {}\n",
                            r.status()
                        ));
                    }
                    Err(err) => {
                        level = TestLevel::Warning;
                        message = format!("Key 可用，但白名单校验请求失败：{err}");
                        detail.push_str(&format!("白名单拒绝校验：⚠️ {err}\n"));
                    }
                }
            }
        }
        detail.push_str(&format!(
            "额度：{} tokens / {} 请求\n",
            key.max_tokens
                .map(|v| v.to_string())
                .unwrap_or_else(|| "不限".into()),
            key.max_requests
                .map(|v| v.to_string())
                .unwrap_or_else(|| "不限".into())
        ));
    } else {
        level = TestLevel::Warning;
        message = "代理未启用鉴权，Key 与白名单未参与校验".into();
        detail.push_str("代理鉴权开关：关闭\n");
    }
    result.level = level;
    result.message = message;
    result.detail = detail;
    result.models = models;
    result
}

/// Metadata read from one upstream model's `/models` entry. Every field is
/// optional because most providers only return an id.
#[derive(Clone, Debug, Default)]
pub struct FetchedModel {
    pub ok: bool,
    pub message: String,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub input_modalities: Vec<String>,
    pub output_modalities: Vec<String>,
    pub reasoning_levels: Vec<String>,
    pub supports_tools: Option<bool>,
    pub supports_streaming: Option<bool>,
    pub models_seen: usize,
}

fn pointer_u64(value: &Value, pointers: &[&str]) -> Option<u64> {
    pointers.iter().find_map(|pointer| {
        value
            .pointer(pointer)
            .and_then(Value::as_u64)
            .filter(|n| *n > 0)
    })
}

fn string_array(value: &Value, pointers: &[&str]) -> Vec<String> {
    for pointer in pointers {
        let Some(entry) = value.pointer(pointer) else {
            continue;
        };
        let items = match entry {
            Value::Array(items) => items.clone(),
            Value::String(text) => text
                .split(',')
                .map(|part| Value::String(part.trim().to_string()))
                .collect(),
            _ => continue,
        };
        let mut out: Vec<String> = items
            .iter()
            .filter_map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .or_else(|| item.get("name").and_then(Value::as_str).map(str::to_string))
                    .or_else(|| {
                        item.get("effort")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
            })
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        out.dedup();
        if !out.is_empty() {
            return out;
        }
    }
    Vec::new()
}

fn pointer_bool(value: &Value, pointers: &[&str]) -> Option<bool> {
    pointers
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(Value::as_bool))
}

fn parse_modalities(item: &Value, direction: &str) -> Vec<String> {
    let mut out = if direction == "input" {
        string_array(
            item,
            &[
                "/input_modalities",
                "/architecture/input_modalities",
                "/modalities/input",
                "/capabilities/input_modalities",
            ],
        )
    } else {
        string_array(
            item,
            &[
                "/output_modalities",
                "/architecture/output_modalities",
                "/modalities/output",
                "/capabilities/output_modalities",
            ],
        )
    };
    // OpenRouter-style "text+image->text".
    if let Some(spec) = item
        .pointer("/architecture/modality")
        .and_then(Value::as_str)
    {
        let (input, output) = spec.split_once("->").unwrap_or((spec, spec));
        let side = if direction == "input" { input } else { output };
        for part in side.split('+') {
            let part = part.trim().to_ascii_lowercase();
            if !part.is_empty() && !out.contains(&part) {
                out.push(part);
            }
        }
    }
    if direction == "input"
        && pointer_bool(
            item,
            &[
                "/supports_vision",
                "/vision",
                "/capabilities/vision",
                "/supports_image_input",
            ],
        )
        .unwrap_or(false)
        && !out.iter().any(|m| m == "image")
    {
        out.push("image".to_string());
    }
    if out.is_empty() && direction == "input" {
        out.push("text".to_string());
    }
    out
}

fn extract_fetched_model(item: &Value) -> FetchedModel {
    let context_window = pointer_u64(
        item,
        &[
            "/context_window",
            "/context_length",
            "/max_context_tokens",
            "/max_context_length",
            "/max_input_tokens",
            "/limits/context_window",
            "/top_provider/context_length",
            "/metadata/context_window",
        ],
    );
    let max_output_tokens = pointer_u64(
        item,
        &[
            "/max_output_tokens",
            "/max_completion_tokens",
            "/max_tokens",
            "/limits/max_output_tokens",
            "/top_provider/max_completion_tokens",
            "/metadata/max_output_tokens",
        ],
    );
    let input_modalities = parse_modalities(item, "input");
    let output_modalities = parse_modalities(item, "output");
    let reasoning_levels = string_array(
        item,
        &[
            "/reasoning_levels",
            "/supported_reasoning_levels",
            "/reasoning/efforts",
            "/capabilities/reasoning_levels",
        ],
    );
    let supports_tools = pointer_bool(
        item,
        &[
            "/capabilities/tools",
            "/supports_tools",
            "/supports_function_calling",
            "/tools",
        ],
    );
    let supports_streaming =
        pointer_bool(item, &["/capabilities/streaming", "/supports_streaming"]);
    FetchedModel {
        ok: true,
        message: String::new(),
        context_window,
        max_output_tokens,
        input_modalities,
        output_modalities,
        reasoning_levels,
        supports_tools,
        supports_streaming,
        models_seen: 0,
    }
}

/// Query the provider's `/models` endpoint and read metadata for one model.
/// This is a diagnostics call and never writes usage records.
pub async fn fetch_model_info(
    gw: &Gateway,
    provider_id: &str,
    upstream_model: &str,
) -> FetchedModel {
    let items = match load_model_items(gw, provider_id).await {
        Ok(items) => items,
        Err(message) => {
            return FetchedModel {
                message,
                ..FetchedModel::default()
            }
        }
    };
    let wanted = upstream_model.trim().to_ascii_lowercase();
    let item = items
        .iter()
        .find(|item| {
            item.get("id")
                .and_then(Value::as_str)
                .map(|id| id.eq_ignore_ascii_case(&wanted))
                .unwrap_or(false)
        })
        .or_else(|| {
            items.iter().find(|item| {
                item.get("id")
                    .and_then(Value::as_str)
                    .map(|id| {
                        let id = id.to_ascii_lowercase();
                        id.contains(&wanted) || wanted.contains(&id)
                    })
                    .unwrap_or(false)
            })
        });
    let Some(item) = item else {
        return FetchedModel {
            message: format!(
                "上游返回 {} 个模型，但没有找到「{upstream_model}」。请核对模型名或供应商协议。",
                items.len()
            ),
            models_seen: items.len(),
            ..FetchedModel::default()
        };
    };
    let mut fetched = extract_fetched_model(item);
    fetched.models_seen = items.len();
    let found_any = fetched.context_window.is_some()
        || fetched.max_output_tokens.is_some()
        || !fetched.reasoning_levels.is_empty()
        || fetched.supports_tools.is_some()
        || fetched.input_modalities.iter().any(|m| m != "text");
    fetched.message = if found_any {
        format!("已从上游读取「{upstream_model}」的模型信息。")
    } else {
        format!("上游确认存在「{upstream_model}」，但没有返回上下文/输出等元数据，请手动填写。")
    };
    fetched
}

/// Cached list of model ids advertised by a provider.
#[derive(Clone, Debug, Default)]
pub struct FetchedModelList {
    pub ok: bool,
    pub message: String,
    pub models: Vec<String>,
}

/// Read every model id from the provider's `/models` endpoint. Used by the
/// route editor to offer a picker while still allowing free-form input.
pub async fn fetch_model_list(gw: &Gateway, provider_id: &str) -> FetchedModelList {
    let items = match load_model_items(gw, provider_id).await {
        Ok(items) => items,
        Err(message) => {
            return FetchedModelList {
                message,
                ..FetchedModelList::default()
            }
        }
    };
    let mut models: Vec<String> = items
        .iter()
        .filter_map(|item| {
            item.get("id")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
        })
        .collect();
    models.sort_by(|a, b| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()));
    models.dedup();
    FetchedModelList {
        ok: true,
        message: if models.is_empty() {
            "上游模型列表为空，请手动填写模型名。".into()
        } else {
            format!("已获取 {} 个上游模型。", models.len())
        },
        models,
    }
}

/// Fetch and unwrap `data` / `models` arrays from a provider's `/models`
/// endpoint, returning readable diagnostics on failure.
async fn load_model_items(gw: &Gateway, provider_id: &str) -> Result<Vec<Value>, String> {
    let cfg = gw.config();
    let provider = cfg
        .provider(provider_id)
        .cloned()
        .ok_or_else(|| "供应商不存在。".to_string())?;
    let accounts = cfg.accounts_of(provider_id);
    let key = accounts
        .iter()
        .find(|account| account.enabled && !account.api_key.is_empty())
        .or_else(|| accounts.iter().find(|account| !account.api_key.is_empty()))
        .and_then(|account| gw.master.decrypt(&account.api_key).ok())
        .unwrap_or_default();
    let url = models_endpoint(&provider);
    let timeout = provider.timeout_secs.unwrap_or(20).max(5);
    let resp = send_probe(gw, &provider, &key, "GET", &url, None, timeout).await;
    if let Some(err) = resp.error {
        return Err(format!("无法连接上游：{err}"));
    }
    let Some(status) = resp.status else {
        return Err("上游没有返回 HTTP 状态。".into());
    };
    if status == 401 || status == 403 {
        return Err(format!("上游鉴权失败（HTTP {status}），请先检查账号 Key。"));
    }
    if status != 200 {
        return Err(format!("上游模型列表返回 HTTP {status}。"));
    }
    let value: Value = serde_json::from_str(&resp.body)
        .map_err(|err| format!("上游模型列表不是合法 JSON：{err}"))?;
    let items = value
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| value.get("models").and_then(Value::as_array))
        .cloned()
        .or_else(|| value.as_array().cloned())
        .unwrap_or_default();
    Ok(items)
}

/// Minimal side-effect-free probe used by the half-open recovery task.
pub async fn half_open_probe(
    gw: &Gateway,
    account: &Account,
    provider: &Provider,
    model_hint: Option<String>,
) -> bool {
    let key = match gw.master.decrypt(&account.api_key) {
        Ok(k) => k,
        Err(_) => return false,
    };
    let data = probe_credentials(gw, provider, &key, model_hint.as_deref(), true).await;
    matches!(data.level, TestLevel::Success)
}

// ---------------------------------------------------------------------------
// Speed test (tokens per second)
// ---------------------------------------------------------------------------

/// Generation budget for a speed test. Large enough that the first-token
/// latency does not dominate the measurement, small enough to stay cheap.
const SPEED_TEST_MAX_TOKENS: u32 = 256;

fn speed_prompt() -> &'static str {
    "Write a detailed technical explanation of how TCP congestion control works. \
     Cover slow start, congestion avoidance, fast retransmit and the difference \
     between Reno and CUBIC. Keep writing continuously."
}

fn speed_body(protocol: Protocol, model: &str) -> Value {
    match protocol {
        Protocol::OpenAiChat => json!({
            "model": model,
            "messages": [{"role": "user", "content": speed_prompt()}],
            "max_tokens": SPEED_TEST_MAX_TOKENS,
            "stream": true,
            "stream_options": {"include_usage": true}
        }),
        Protocol::AnthropicMessages => json!({
            "model": model,
            "messages": [{"role": "user", "content": speed_prompt()}],
            "max_tokens": SPEED_TEST_MAX_TOKENS,
            "stream": true
        }),
        Protocol::OpenAiResponses => json!({
            "model": model,
            "input": speed_prompt(),
            "max_output_tokens": SPEED_TEST_MAX_TOKENS,
            "stream": true
        }),
    }
}

/// Build and send one speed-test request, mapping failures to readable text.
#[allow(clippy::too_many_arguments)]
async fn send_speed_request(
    gw: &Gateway,
    provider: &Provider,
    key: &str,
    url: &str,
    protocol: Protocol,
    body: &Value,
    timeout_secs: u64,
) -> Result<reqwest::Response, String> {
    let mut req = gw
        .client_for(url)
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(
            reqwest::header::ACCEPT,
            "text/event-stream, application/json",
        )
        .json(body);
    req = apply_auth(req, &provider.auth, key).map_err(|err| format!("鉴权头配置无效：{err}"))?;
    for h in &provider.extra_headers {
        if !h.name.trim().is_empty() {
            req = req.header(h.name.trim(), h.value.as_str());
        }
    }
    if protocol == Protocol::AnthropicMessages
        && !provider
            .extra_headers
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case("anthropic-version"))
    {
        req = req.header("anthropic-version", "2023-06-01");
    }
    match tokio::time::timeout(Duration::from_secs(timeout_secs), req.send()).await {
        Err(_) => Err(format!("连接超时（{timeout_secs}s）")),
        Ok(Err(err)) => Err(readable_reqwest_error(&err)),
        Ok(Ok(resp)) => Ok(resp),
    }
}

/// Length of assistant text in a non-streaming response body.
fn nonstream_text_chars(protocol: Protocol, value: &Value) -> usize {
    let text = match protocol {
        Protocol::OpenAiChat => value
            .pointer("/choices/0/message/content")
            .or_else(|| value.pointer("/choices/0/text")),
        // Anthropic returns an array of content blocks; every text block is
        // generated output and should count toward the estimate.
        Protocol::AnthropicMessages => value
            .get("content")
            .filter(|content| content.is_array())
            .or_else(|| value.get("completion")),
        Protocol::OpenAiResponses => value
            .pointer("/output_text")
            .or_else(|| value.pointer("/output"))
            .or_else(|| value.pointer("/response/output")),
    };
    let chars = match text {
        Some(Value::String(text)) => text.chars().count(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .map(|text| text.chars().count())
            .sum(),
        _ => 0,
    };
    if chars > 0 {
        return chars;
    }
    // Dialect mismatch fallback: count any normal content field.
    generic_stream_text(value)
        .map(|text| text.chars().count())
        .unwrap_or(0)
}

/// Measure decode throughput against one account.
///
/// The request asks for a bounded amount of streaming output, then reports
/// completion tokens divided by the time between the first token and the last.
/// Time to first token (TTFT) is reported separately, because it reflects queue
/// and prefill latency rather than generation speed.
pub async fn test_speed_account(
    gw: &Gateway,
    account_id: &str,
    model_hint: Option<String>,
) -> TestResult {
    let cfg = gw.config();
    let mut result = TestResult::new(&format!("speed:{account_id}"), "speed", "账号测速");
    let Some(account) = cfg.account(account_id).cloned() else {
        result.level = TestLevel::Error;
        result.message = "账号不存在".into();
        return result;
    };
    result.title = account.name.clone();
    let Some(provider) = cfg.provider(&account.provider_id).cloned() else {
        result.level = TestLevel::Error;
        result.message = "账号所属供应商不存在".into();
        return result;
    };
    let key = match gw.master.decrypt(&account.api_key) {
        Ok(k) => k,
        Err(err) => {
            result.level = TestLevel::Error;
            result.message = format!("Key 无法解密：{err}");
            return result;
        }
    };
    let model = model_hint
        .filter(|m| !m.trim().is_empty())
        .or_else(|| {
            cfg.routes
                .iter()
                .find(|r| r.provider_id == provider.id)
                .map(|r| r.upstream_model.clone())
        })
        .or_else(|| provider.test_model.clone())
        .unwrap_or_else(|| default_test_model(&provider));
    let protocol = provider.protocol_for(&model, None);
    let url = minimal_url(&provider, protocol);
    let body = speed_body(protocol, &model);

    let timeout = provider.timeout_secs.unwrap_or(120).max(20);
    let mut payload = body;
    let mut started = Instant::now();
    let mut resp =
        match send_speed_request(gw, &provider, &key, &url, protocol, &payload, timeout).await {
            Ok(resp) => resp,
            Err(message) => {
                result.level = TestLevel::Unreachable;
                result.message = message;
                result.latency_ms = started.elapsed().as_millis() as u64;
                return result;
            }
        };
    let mut status = resp.status().as_u16();
    // Some OpenAI-compatible gateways reject `stream_options`; retry once
    // without it rather than failing the speed test.
    if status == 400 && protocol == Protocol::OpenAiChat && payload.get("stream_options").is_some()
    {
        if let Some(object) = payload.as_object_mut() {
            object.remove("stream_options");
        }
        started = Instant::now();
        resp = match send_speed_request(gw, &provider, &key, &url, protocol, &payload, timeout)
            .await
        {
            Ok(resp) => resp,
            Err(message) => {
                result.level = TestLevel::Unreachable;
                result.message = message;
                return result;
            }
        };
        status = resp.status().as_u16();
    }
    result.status = Some(status);
    if status == 401 || status == 403 {
        result.level = TestLevel::AuthFailed;
        result.message = "Key 无效或无权限".into();
        result.latency_ms = started.elapsed().as_millis() as u64;
        return result;
    }
    if status == 429 {
        result.level = TestLevel::RateLimited;
        result.message = "当前被限流，无法测速".into();
        result.latency_ms = started.elapsed().as_millis() as u64;
        return result;
    }
    if !(200..300).contains(&status) {
        let text = tokio::time::timeout(Duration::from_secs(10), resp.text())
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or_default();
        result.level = TestLevel::Error;
        result.message = format!("HTTP {status}：{}", upstream_error_message(&text));
        result.latency_ms = started.elapsed().as_millis() as u64;
        return result;
    }

    let mut stream: std::pin::Pin<
        Box<dyn futures_util::Stream<Item = reqwest::Result<bytes::Bytes>> + Send>,
    > = Box::pin(resp.bytes_stream());
    let mut decoder = SseDecoder::default();
    let mut raw: Vec<u8> = Vec::new();
    let mut first_token_at: Option<Duration> = None;
    let mut last_token_at = started.elapsed();
    let mut usage = crate::protocol::Usage::default();
    let mut produced_chars = 0usize;
    let idle = Duration::from_secs(30);
    loop {
        if started.elapsed() > Duration::from_secs(timeout) {
            break;
        }
        match tokio::time::timeout(idle, stream.next()).await {
            Err(_) => break,
            Ok(None) => break,
            Ok(Some(Err(_))) => break,
            Ok(Some(Ok(chunk))) => {
                if raw.len() < 512 * 1024 {
                    raw.extend_from_slice(&chunk);
                }
                for event in decoder.push(&chunk) {
                    note_speed_event(
                        protocol,
                        &event,
                        started,
                        &mut first_token_at,
                        &mut last_token_at,
                        &mut produced_chars,
                        &mut usage,
                    );
                }
            }
        }
        if usage.output_tokens > 0 && first_token_at.is_some() {
            break;
        }
    }
    if let Some(event) = decoder.flush() {
        note_speed_event(
            protocol,
            &event,
            started,
            &mut first_token_at,
            &mut last_token_at,
            &mut produced_chars,
            &mut usage,
        );
    }

    let elapsed = started.elapsed();
    result.latency_ms = elapsed.as_millis() as u64;
    result.first_token_ms = first_token_at.map(|d| d.as_millis() as u64);
    if first_token_at.is_none() && usage.output_tokens > 0 {
        // No delta text was recognised, but the upstream did report output
        // tokens. Throughput over the whole request is still a useful number.
        let tokens = usage.output_tokens;
        let measured = elapsed.as_secs_f64().max(0.05);
        let tps = tokens as f64 / measured;
        result.tokens = Some(tokens);
        result.tokens_per_sec = Some(tps);
        result.level = TestLevel::Success;
        result.message = format!("{:.1} tok/s · {} tokens（按上游 usage 计时）", tps, tokens);
        result.detail = format!(
            "POST {url}\n协议: {}\n模型: {}\nHTTP {status}\n\
             未捕获增量文本，使用上游 usage 的 output_tokens 计时\n\
             生成速度: {:.2} tokens/s\n生成 token: {}\n总耗时: {}ms",
            protocol.label(),
            model,
            tps,
            tokens,
            elapsed.as_millis()
        );
        return result;
    }
    let Some(first_at) = first_token_at else {
        // The upstream may have ignored `stream` and returned a plain JSON
        // body; measure that instead of reporting a failure.
        if let Ok(value) = serde_json::from_slice::<Value>(&raw) {
            let usage = crate::protocol::extract_nonstream_usage(protocol, &value);
            let chars = nonstream_text_chars(protocol, &value);
            let tokens = if usage.output_tokens > 0 {
                usage.output_tokens
            } else {
                (chars / 4).max(1) as u64
            };
            if tokens > 0 && elapsed.as_secs_f64() > 0.05 {
                let tps = tokens as f64 / elapsed.as_secs_f64();
                result.status = Some(status);
                result.tokens = Some(tokens);
                result.tokens_per_sec = Some(tps);
                result.level = TestLevel::Success;
                result.message = format!("{:.1} tok/s · {} tokens（非流式）", tps, tokens);
                result.detail = format!(
                    "POST {url}\n协议: {}\n模型: {}\nHTTP {status}\n\
                     上游未返回 SSE，按非流式响应计时\n\
                     生成速度: {:.2} tokens/s\n生成 token: {}\n总耗时: {}ms",
                    protocol.label(),
                    model,
                    tps,
                    tokens,
                    elapsed.as_millis()
                );
                return result;
            }
        }
        result.level = TestLevel::Warning;
        result.message = "上游接受了请求，但没有返回可测量的文本，无法测速".into();
        result.detail = format!(
            "POST {url}\n协议: {}\n模型: {}\nHTTP {status}\n总耗时: {}ms",
            protocol.label(),
            model,
            elapsed.as_millis()
        );
        return result;
    };
    let tokens = if usage.output_tokens > 0 {
        usage.output_tokens
    } else {
        (produced_chars / 4).max(1) as u64
    };
    let decode_secs = last_token_at.saturating_sub(first_at).as_secs_f64();
    let measured = if usage.output_tokens > 0 && decode_secs > 0.05 {
        decode_secs
    } else {
        // No usage block: the stream keeps flowing until the cap, so subtract
        // TTFT from the total wall time.
        elapsed.saturating_sub(first_at).as_secs_f64().max(0.05)
    };
    let tps = tokens as f64 / measured;
    result.tokens = Some(tokens);
    result.tokens_per_sec = Some(tps);
    result.level = TestLevel::Success;
    result.message = format!(
        "{:.1} tok/s · TTFT {}ms · {} tokens",
        tps,
        result.first_token_ms.unwrap_or(0),
        tokens
    );
    result.detail = format!(
        "POST {url}\n协议: {}\n模型: {}\nHTTP {status}\n\
         生成速度: {:.2} tokens/s\n首字延迟: {}ms\n生成 token: {}{}\n总耗时: {}ms",
        protocol.label(),
        model,
        tps,
        result.first_token_ms.unwrap_or(0),
        tokens,
        if usage.output_tokens == 0 {
            "（按字符估算）"
        } else {
            ""
        },
        elapsed.as_millis()
    );
    result
}

/// Speed-test every enabled account of a provider, sequentially, and summarize.
pub async fn test_speed_provider(gw: &Gateway, provider_id: &str) -> TestResult {
    let cfg = gw.config();
    let mut result = TestResult::new(&format!("speed:{provider_id}"), "speed", "供应商测速");
    let Some(provider) = cfg.provider(provider_id).cloned() else {
        result.level = TestLevel::Error;
        result.message = "供应商不存在".into();
        return result;
    };
    result.title = provider.name.clone();
    let mut accounts = cfg.accounts_of(&provider.id);
    accounts.retain(|a| a.enabled);
    accounts.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.created_at.cmp(&b.created_at))
    });
    if accounts.is_empty() {
        result.level = TestLevel::Error;
        result.message = "该供应商没有启用的账号".into();
        return result;
    }
    let model_hint = cfg
        .routes
        .iter()
        .find(|r| r.provider_id == provider.id)
        .map(|r| r.upstream_model.clone())
        .or_else(|| provider.test_model.clone());

    let mut lines: Vec<String> = Vec::new();
    let mut best: Option<(String, f64)> = None;
    let mut measured = 0usize;
    for account in &accounts {
        let one = test_speed_account(gw, &account.id, model_hint.clone()).await;
        match one.tokens_per_sec {
            Some(tps) => {
                measured += 1;
                lines.push(format!(
                    "账号「{}」：{:.1} tok/s · TTFT {}ms · {} tokens",
                    account.name,
                    tps,
                    one.first_token_ms.unwrap_or(0),
                    one.tokens.unwrap_or(0)
                ));
                if best
                    .as_ref()
                    .map(|(_, current)| tps > *current)
                    .unwrap_or(true)
                {
                    best = Some((account.name.clone(), tps));
                }
            }
            None => lines.push(format!(
                "账号「{}」：{} — {}",
                account.name,
                one.level.label(),
                one.message
            )),
        }
    }
    result.detail = format!(
        "供应商: {}\nBase URL: {}\n账号数: {}\n\n{}",
        provider.name,
        provider.base_url,
        accounts.len(),
        lines.join("\n")
    );
    match best {
        Some((name, tps)) => {
            result.level = TestLevel::Success;
            result.tokens_per_sec = Some(tps);
            result.message = format!(
                "最快账号「{name}」{tps:.1} tok/s（{measured}/{} 个账号测速成功）",
                accounts.len()
            );
        }
        None => {
            result.level = TestLevel::Warning;
            result.message = format!("{} 个账号均未能完成测速", accounts.len());
        }
    }
    result
}

/// Extract generated text from a streaming event. All three dialects have
/// several variants in the wild (reasoning deltas, tool arguments, gateways
/// that send a whole message instead of a delta), and any of them proves the
/// model is producing tokens, which is all the speed test needs.
fn text_delta(protocol: Protocol, value: &Value) -> Option<&str> {
    match protocol {
        Protocol::OpenAiChat => value
            .pointer("/choices/0/delta/content")
            .or_else(|| value.pointer("/choices/0/delta/reasoning_content"))
            .or_else(|| value.pointer("/choices/0/delta/reasoning"))
            .or_else(|| value.pointer("/choices/0/message/content"))
            .or_else(|| value.pointer("/choices/0/text"))
            .or_else(|| value.pointer("/choices/0/delta/tool_calls/0/function/arguments"))
            .and_then(as_text),
        Protocol::AnthropicMessages => {
            let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
            match kind {
                "content_block_delta" => value
                    .pointer("/delta/text")
                    .or_else(|| value.pointer("/delta/thinking"))
                    .or_else(|| value.pointer("/delta/partial_json"))
                    .and_then(as_text),
                "content_block_start" => value.pointer("/content_block/text").and_then(as_text),
                _ => None,
            }
        }
        Protocol::OpenAiResponses => {
            let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
            match kind {
                "response.output_text.delta"
                | "response.reasoning_text.delta"
                | "response.function_call_arguments.delta" => value.get("delta").and_then(as_text),
                "response.output_text.done" => value.get("text").and_then(as_text),
                _ => None,
            }
        }
    }
    // Last resort: providers sometimes speak a slightly different dialect than
    // configured (or wrap text in extra fields). Any generated text is enough
    // to measure speed.
    .or_else(|| generic_stream_text(value))
}

/// Depth-limited search for a generated-text field. Only used when the
/// protocol-specific extractor finds nothing, and only keys that are almost
/// always assistant output are considered.
fn generic_stream_text(value: &Value) -> Option<&str> {
    const KEYS: [&str; 8] = [
        "output_text",
        "reasoning_content",
        "partial_json",
        "arguments",
        "content",
        "thinking",
        "text",
        "delta",
    ];
    for key in KEYS {
        if let Some(found) = find_key(value, key, 5) {
            if let Some(text) = as_text(found) {
                if !text.trim().is_empty() {
                    return Some(text);
                }
            }
        }
    }
    None
}

fn find_key<'a>(value: &'a Value, key: &str, depth: u8) -> Option<&'a Value> {
    if depth == 0 {
        return None;
    }
    match value {
        Value::Object(map) => {
            if let Some(found) = map.get(key) {
                return Some(found);
            }
            map.values()
                .find_map(|child| find_key(child, key, depth - 1))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_key(child, key, depth - 1)),
        _ => None,
    }
}

/// Accept a plain string or an array of content parts.
fn as_text(value: &Value) -> Option<&str> {
    if let Some(text) = value.as_str() {
        return Some(text);
    }
    value
        .as_array()
        .and_then(|parts| parts.iter().find_map(|part| part.get("text")?.as_str()))
}

/// Feed one SSE event into the running speed measurement.
fn note_speed_event(
    protocol: Protocol,
    event: &crate::protocol::SseEvent,
    started: Instant,
    first_token_at: &mut Option<Duration>,
    last_token_at: &mut Duration,
    produced_chars: &mut usize,
    usage: &mut crate::protocol::Usage,
) {
    let Ok(value) = serde_json::from_str::<Value>(&event.data) else {
        return;
    };
    if let Some(delta) = text_delta(protocol, &value) {
        if !delta.is_empty() {
            if first_token_at.is_none() {
                *first_token_at = Some(started.elapsed());
            }
            *last_token_at = started.elapsed();
            *produced_chars += delta.chars().count();
        }
    }
    *usage = merge_stream_usage(protocol, &value, *usage);
}

fn merge_stream_usage(
    protocol: Protocol,
    value: &Value,
    current: crate::protocol::Usage,
) -> crate::protocol::Usage {
    let usage = match protocol {
        Protocol::OpenAiChat => value.get("usage"),
        Protocol::AnthropicMessages => value
            .pointer("/message/usage")
            .or_else(|| value.get("usage")),
        Protocol::OpenAiResponses => value
            .pointer("/response/usage")
            .or_else(|| value.get("usage")),
    };
    match usage.and_then(Value::as_object) {
        Some(object) => {
            let next = crate::protocol::usage_from_object(protocol, object);
            crate::protocol::Usage {
                input_tokens: current.input_tokens.max(next.input_tokens),
                cached_tokens: current.cached_tokens.max(next.cached_tokens),
                output_tokens: current.output_tokens.max(next.output_tokens),
                total_tokens: current.total_tokens.max(next.total_tokens),
                estimated: current.estimated,
            }
        }
        None => current,
    }
}

#[cfg(test)]
mod speed_probe_tests {
    use super::*;

    #[test]
    fn reads_text_deltas_from_each_protocol() {
        let chat = json!({"choices":[{"delta":{"content":"hello"}}]});
        assert_eq!(text_delta(Protocol::OpenAiChat, &chat), Some("hello"));

        let anthropic =
            json!({"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}});
        assert_eq!(
            text_delta(Protocol::AnthropicMessages, &anthropic),
            Some("hi")
        );

        let responses = json!({"type":"response.output_text.delta","delta":"yo"});
        assert_eq!(
            text_delta(Protocol::OpenAiResponses, &responses),
            Some("yo")
        );
        assert_eq!(text_delta(Protocol::OpenAiChat, &json!({})), None);
    }

    #[test]
    fn merges_stream_usage_upwards() {
        let first = json!({"usage":{"prompt_tokens":10,"completion_tokens":1,"total_tokens":11}});
        let second = json!({"usage":{"prompt_tokens":10,"completion_tokens":9,"total_tokens":19}});
        let usage = merge_stream_usage(
            Protocol::OpenAiChat,
            &first,
            crate::protocol::Usage::default(),
        );
        let usage = merge_stream_usage(Protocol::OpenAiChat, &second, usage);
        assert_eq!(usage.output_tokens, 9);
        assert_eq!(usage.total_tokens, 19);
    }

    #[test]
    fn counts_nonstream_text_for_estimates() {
        let chat = json!({"choices":[{"message":{"content":"four words here"}}]});
        assert_eq!(
            nonstream_text_chars(Protocol::OpenAiChat, &chat),
            "four words here".chars().count()
        );
        let anthropic =
            json!({"content":[{"type":"text","text":"abc"},{"type":"text","text":"de"}]});
        assert_eq!(
            nonstream_text_chars(Protocol::AnthropicMessages, &anthropic),
            5
        );
        let responses = json!({"output_text":"hello"});
        assert_eq!(
            nonstream_text_chars(Protocol::OpenAiResponses, &responses),
            5
        );
    }
}

#[cfg(test)]
mod ai_pricing_tests {
    use super::*;

    #[test]
    fn extracts_json_from_fenced_and_chatty_replies() {
        let fenced = r#"```json
{"name":"峰谷","currency":"CNY","base":{"input_per_mtok":1.0}}
```"#;
        let value = extract_json_object(fenced).expect("fenced json");
        assert_eq!(value.get("name").and_then(Value::as_str), Some("峰谷"));

        let chatty = "Here you go:\n{\"name\":\"x\",\"periods\":[]}\nThanks";
        let value = extract_json_object(chatty).expect("embedded json");
        assert_eq!(value.get("name").and_then(Value::as_str), Some("x"));
    }

    #[test]
    fn reads_text_from_each_protocol_response() {
        let chat = json!({"choices":[{"message":{"content":"{\"base\":{}}"}}]}).to_string();
        assert_eq!(
            ai_response_text(Protocol::OpenAiChat, &chat).unwrap(),
            "{\"base\":{}}"
        );

        let anthropic =
            json!({"content":[{"type":"text","text":"abc"},{"type":"text","text":"de"}]})
                .to_string();
        assert_eq!(
            ai_response_text(Protocol::AnthropicMessages, &anthropic).unwrap(),
            "abcde"
        );

        let responses = json!({"output":[{"content":[{"text":"hi"},{"text":"!"}]}]}).to_string();
        assert_eq!(
            ai_response_text(Protocol::OpenAiResponses, &responses).unwrap(),
            "hi!"
        );
    }

    #[test]
    fn builds_protocol_specific_pricing_requests() {
        let prompt = "工作日 9-18 点高峰";
        for protocol in [
            Protocol::OpenAiChat,
            Protocol::AnthropicMessages,
            Protocol::OpenAiResponses,
        ] {
            let body = ai_pricing_body(protocol, "model-x", prompt);
            assert_eq!(body.get("model").and_then(Value::as_str), Some("model-x"));
            match protocol {
                Protocol::OpenAiChat => assert!(body.get("messages").is_some()),
                Protocol::AnthropicMessages => assert!(body.get("system").is_some()),
                Protocol::OpenAiResponses => assert!(body.get("instructions").is_some()),
            }
        }
    }
}
