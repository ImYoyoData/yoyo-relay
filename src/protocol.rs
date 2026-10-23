//! Protocol conversion between the client-facing API shapes and upstream
//! providers. Only the parts needed for a resilient chat relay are translated;
//! unknown fields are forwarded when the protocols match.

use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use crate::config::{ModelRoute, Protocol, RouteMode};

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ingress {
    Chat,
    Anthropic,
    Responses,
    Generic,
}

impl Ingress {
    pub fn protocol(self) -> Option<Protocol> {
        match self {
            Ingress::Chat => Some(Protocol::OpenAiChat),
            Ingress::Anthropic => Some(Protocol::AnthropicMessages),
            Ingress::Responses => Some(Protocol::OpenAiResponses),
            Ingress::Generic => None,
        }
    }

    pub fn from_path(path: &str) -> Ingress {
        let path = path.trim_end_matches('/');
        if path.ends_with("/chat/completions") {
            Ingress::Chat
        } else if path.ends_with("/messages") {
            Ingress::Anthropic
        } else if path.ends_with("/responses") {
            Ingress::Responses
        } else {
            Ingress::Generic
        }
    }

    #[allow(dead_code)]
    pub fn path(self) -> &'static str {
        match self {
            Ingress::Chat => "/v1/chat/completions",
            Ingress::Anthropic => "/v1/messages",
            Ingress::Responses => "/v1/responses",
            Ingress::Generic => "/v1",
        }
    }
}

/// Whether a route can serve the given ingress protocol.
pub fn route_supports_ingress(route: &ModelRoute, ingress: Ingress, upstream: Protocol) -> bool {
    let Some(client) = ingress.protocol() else {
        // Generic passthrough endpoints never convert protocols.
        return route.mode == RouteMode::Passthrough;
    };
    if route.mode == RouteMode::Passthrough {
        return client == upstream;
    }
    // Every ingress protocol can be adapted to every upstream protocol.
    true
}

#[derive(Default, Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct Usage {
    pub input_tokens: u64,
    /// Prompt tokens served from the provider's prompt cache. Billed at the
    /// route's cached-input rate when one is configured.
    pub cached_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub estimated: bool,
}

impl Usage {
    pub fn total(&self) -> u64 {
        if self.total_tokens > 0 {
            self.total_tokens
        } else {
            self.input_tokens.saturating_add(self.output_tokens)
        }
    }

    fn absorb_max(&mut self, other: &Usage) {
        self.input_tokens = self.input_tokens.max(other.input_tokens);
        self.cached_tokens = self.cached_tokens.max(other.cached_tokens);
        self.output_tokens = self.output_tokens.max(other.output_tokens);
        if other.total_tokens > 0 {
            self.total_tokens = self.total_tokens.max(other.total_tokens);
        }
    }

    pub fn openai_json(&self) -> Value {
        json!({
            "prompt_tokens": self.input_tokens,
            "completion_tokens": self.output_tokens,
            "total_tokens": self.total(),
        })
    }

    #[allow(dead_code)]
    pub fn anthropic_json(&self) -> Value {
        json!({
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
        })
    }
}

pub fn extract_client_model(body: &Value) -> Option<String> {
    body.get("model")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

pub fn is_stream(body: &Value) -> bool {
    body.get("stream").and_then(Value::as_bool).unwrap_or(false)
}

/// Build the upstream JSON body. `raw` requests (generic passthrough) are
/// handled by the caller and never reach this function.
pub fn build_upstream_body(
    ingress: Ingress,
    upstream: Protocol,
    body: &Value,
    upstream_model: &str,
) -> Result<Value> {
    let client = ingress
        .protocol()
        .ok_or_else(|| anyhow!("该端点不支持协议转换"))?;
    let mut out = if client == upstream {
        body.clone()
    } else {
        match (client, upstream) {
            (Protocol::OpenAiChat, Protocol::AnthropicMessages) => {
                chat_to_anthropic(body, upstream_model)?
            }
            (Protocol::AnthropicMessages, Protocol::OpenAiChat) => {
                anthropic_to_chat(body, upstream_model)?
            }
            (Protocol::OpenAiChat, Protocol::OpenAiResponses) => {
                chat_to_responses(body, upstream_model)?
            }
            (Protocol::AnthropicMessages, Protocol::OpenAiResponses) => {
                let chat = anthropic_to_chat(body, upstream_model)?;
                chat_to_responses(&chat, upstream_model)?
            }
            (Protocol::OpenAiResponses, Protocol::OpenAiChat) => {
                responses_to_chat(body, upstream_model)?
            }
            (Protocol::OpenAiResponses, Protocol::AnthropicMessages) => {
                let chat = responses_to_chat(body, upstream_model)?;
                chat_to_anthropic(&chat, upstream_model)?
            }
            _ => bail!("暂不支持从 {} 转换到 {}", client.label(), upstream.label()),
        }
    };
    if out.get("model").and_then(Value::as_str) != Some(upstream_model) {
        out["model"] = Value::String(upstream_model.to_string());
    }
    let stream = is_stream(body);
    if stream {
        out["stream"] = Value::Bool(true);
        if upstream == Protocol::OpenAiChat {
            // Make usage observable in the final chunk.
            let opts = out
                .as_object_mut()
                .ok_or_else(|| anyhow!("请求体不是 JSON 对象"))?
                .entry("stream_options")
                .or_insert_with(|| json!({}));
            if let Some(obj) = opts.as_object_mut() {
                obj.insert("include_usage".into(), Value::Bool(true));
            }
        }
    }
    Ok(out)
}

/// Convert a non-streaming upstream response into the client's protocol.
pub fn convert_nonstream_response(
    ingress: Ingress,
    upstream: Protocol,
    value: Value,
    client_model: &str,
) -> Result<Value> {
    let client = ingress
        .protocol()
        .ok_or_else(|| anyhow!("该端点不支持协议转换"))?;
    if client == upstream {
        return Ok(rewrite_model(value, upstream, client_model));
    }
    match (client, upstream) {
        (Protocol::OpenAiChat, Protocol::AnthropicMessages) => {
            Ok(anthropic_response_to_chat(&value, client_model))
        }
        (Protocol::AnthropicMessages, Protocol::OpenAiChat) => {
            Ok(chat_response_to_anthropic(&value, client_model))
        }
        (Protocol::OpenAiChat, Protocol::OpenAiResponses) => {
            Ok(responses_response_to_chat(&value, client_model))
        }
        (Protocol::AnthropicMessages, Protocol::OpenAiResponses) => {
            let chat = responses_response_to_chat(&value, client_model);
            Ok(chat_response_to_anthropic(&chat, client_model))
        }
        (Protocol::OpenAiResponses, Protocol::OpenAiChat) => {
            Ok(chat_response_to_responses(&value, client_model))
        }
        (Protocol::OpenAiResponses, Protocol::AnthropicMessages) => {
            let chat = anthropic_response_to_chat(&value, client_model);
            Ok(chat_response_to_responses(&chat, client_model))
        }
        _ => bail!("暂不支持从 {} 转换到 {}", upstream.label(), client.label()),
    }
}

pub fn extract_nonstream_usage(protocol: Protocol, value: &Value) -> Usage {
    value
        .get("usage")
        .and_then(Value::as_object)
        .map(|usage| usage_from_object(protocol, usage))
        .unwrap_or_default()
}

/// Read a `usage` object in the given protocol dialect, including prompt-cache
/// hit counts. Shared by the non-stream and SSE paths so both bill identically.
pub fn usage_from_object(protocol: Protocol, usage: &serde_json::Map<String, Value>) -> Usage {
    let num = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
    let mut u = Usage::default();
    match protocol {
        Protocol::OpenAiChat => {
            u.input_tokens = num("prompt_tokens");
            u.output_tokens = num("completion_tokens");
            u.total_tokens = num("total_tokens");
            // OpenAI reports `prompt_tokens_details.cached_tokens`, while
            // DeepSeek-style gateways use `prompt_cache_hit_tokens` and some
            // compatible servers use `cached_tokens` directly.
            u.cached_tokens = usage
                .get("prompt_tokens_details")
                .and_then(|details| {
                    details
                        .get("cached_tokens")
                        .or_else(|| details.get("cache_read_tokens"))
                })
                .and_then(Value::as_u64)
                .or_else(|| usage.get("prompt_cache_hit_tokens").and_then(Value::as_u64))
                .or_else(|| usage.get("cache_read_input_tokens").and_then(Value::as_u64))
                .or_else(|| usage.get("cached_tokens").and_then(Value::as_u64))
                .unwrap_or(0);
        }
        Protocol::AnthropicMessages => {
            // Anthropic reports cache reads and writes separately from
            // `input_tokens`; all three are prompt tokens for billing purposes.
            let fresh = num("input_tokens");
            let cache_read = num("cache_read_input_tokens");
            let cache_write = num("cache_creation_input_tokens");
            let cached = if num("cached_tokens") > 0 {
                num("cached_tokens")
            } else {
                cache_read
            };
            u.input_tokens = fresh + cache_read + cache_write;
            u.cached_tokens = cached;
            u.output_tokens = num("output_tokens");
            u.total_tokens = u.input_tokens + u.output_tokens;
        }
        Protocol::OpenAiResponses => {
            u.input_tokens = num("input_tokens");
            u.output_tokens = num("output_tokens");
            u.total_tokens = num("total_tokens");
            u.cached_tokens = usage
                .get("input_tokens_details")
                .and_then(|details| {
                    details
                        .get("cached_tokens")
                        .or_else(|| details.get("cache_read_tokens"))
                })
                .and_then(Value::as_u64)
                .or_else(|| usage.get("cached_tokens").and_then(Value::as_u64))
                .unwrap_or(0);
        }
    }
    u
}

fn rewrite_model(mut value: Value, protocol: Protocol, client_model: &str) -> Value {
    match protocol {
        Protocol::OpenAiChat | Protocol::OpenAiResponses | Protocol::AnthropicMessages => {
            if value.get("model").is_some()
                || value.get("object").is_some()
                || value.get("type").is_some()
            {
                value["model"] = Value::String(client_model.to_string());
            }
            // Anthropic streaming message_start carries model inside `message`.
            if let Some(message) = value.get_mut("message") {
                if message.get("model").is_some() {
                    message["model"] = Value::String(client_model.to_string());
                }
            }
        }
    }
    value
}

// ---------------------------------------------------------------------------
// Request conversion: chat -> anthropic
// ---------------------------------------------------------------------------

fn chat_to_anthropic(body: &Value, upstream_model: &str) -> Result<Value> {
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("messages 字段缺失"))?;
    let mut system_parts: Vec<String> = Vec::new();
    let mut out: Vec<(String, Vec<Value>)> = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("user");
        match role {
            "system" | "developer" => {
                let text = flatten_text(msg.get("content"));
                if !text.is_empty() {
                    system_parts.push(text);
                }
            }
            "tool" => {
                let call_id = msg
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let text = flatten_text(msg.get("content"));
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": call_id,
                    "content": text,
                });
                push_blocks(&mut out, "user", vec![block]);
            }
            "assistant" => {
                let mut blocks = Vec::new();
                if let Some(text) = opt_text(msg.get("content")) {
                    if !text.is_empty() {
                        blocks.push(json!({"type": "text", "text": text}));
                    }
                }
                if let Some(tool_calls) = msg.get("tool_calls").and_then(Value::as_array) {
                    for tc in tool_calls {
                        let name = tc
                            .pointer("/function/name")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let id = tc.get("id").and_then(Value::as_str).unwrap_or_default();
                        let args = tc
                            .pointer("/function/arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("{}");
                        let input =
                            serde_json::from_str::<Value>(args).unwrap_or_else(|_| json!({}));
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": input,
                        }));
                    }
                }
                if !blocks.is_empty() {
                    push_blocks(&mut out, "assistant", blocks);
                }
            }
            _ => {
                let blocks = content_to_anthropic(msg.get("content"))?;
                if !blocks.is_empty() {
                    push_blocks(&mut out, "user", blocks);
                }
            }
        }
    }

    if out.is_empty() {
        bail!("messages 为空");
    }

    let mut result = json!({
        "model": upstream_model,
        "messages": out.into_iter().map(|(role, blocks)| json!({"role": role, "content": blocks})).collect::<Vec<_>>(),
        "max_tokens": body
            .get("max_completion_tokens")
            .and_then(Value::as_u64)
            .or_else(|| body.get("max_tokens").and_then(Value::as_u64))
            .unwrap_or(4096),
    });

    if !system_parts.is_empty() {
        result["system"] = Value::String(system_parts.join("\n\n"));
    }
    for key in ["temperature", "top_p", "top_k"] {
        if let Some(v) = body.get(key) {
            result[key] = v.clone();
        }
    }
    if let Some(stop) = body.get("stop") {
        result["stop_sequences"] = match stop {
            Value::String(s) => json!([s]),
            Value::Array(_) => stop.clone(),
            _ => Value::Null,
        };
    }
    if let Some(user) = body.get("user").and_then(Value::as_str) {
        result["metadata"] = json!({"user_id": user});
    }

    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        let anthropic_tools: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                let f = t.get("function")?;
                Some(json!({
                    "name": f.get("name")?.as_str()?,
                    "description": f.get("description").cloned().unwrap_or(Value::Null),
                    "input_schema": f.get("parameters").cloned().unwrap_or_else(|| json!({"type":"object","properties":{}})),
                }))
            })
            .collect();
        if !anthropic_tools.is_empty() {
            result["tools"] = Value::Array(anthropic_tools);
            if let Some(choice) = body.get("tool_choice") {
                result["tool_choice"] = map_tool_choice_chat_to_anthropic(choice);
            }
            if let Some(parallel) = body.get("parallel_tool_calls").and_then(Value::as_bool) {
                result["disable_parallel_tool_use"] = Value::Bool(!parallel);
            }
        }
    }

    Ok(result)
}

fn content_to_anthropic(content: Option<&Value>) -> Result<Vec<Value>> {
    let Some(content) = content else {
        return Ok(Vec::new());
    };
    match content {
        Value::String(s) => Ok(if s.is_empty() {
            Vec::new()
        } else {
            vec![json!({"type": "text", "text": s})]
        }),
        Value::Array(parts) => {
            let mut out = Vec::new();
            for part in parts {
                let ty = part.get("type").and_then(Value::as_str).unwrap_or("text");
                match ty {
                    "text" => {
                        let text = part.get("text").and_then(Value::as_str).unwrap_or_default();
                        if !text.is_empty() {
                            out.push(json!({"type": "text", "text": text}));
                        }
                    }
                    "image_url" => {
                        let url = part
                            .pointer("/image_url/url")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        out.push(image_block(url)?);
                    }
                    "image" => {
                        // Already in a near-Anthropic shape.
                        out.push(part.clone());
                    }
                    other => bail!("不支持的消息内容类型：{other}"),
                }
            }
            Ok(out)
        }
        _ => Ok(Vec::new()),
    }
}

fn image_block(url: &str) -> Result<Value> {
    if let Some(rest) = url.strip_prefix("data:") {
        let (meta, data) = rest
            .split_once(',')
            .ok_or_else(|| anyhow!("图片 data URL 无效"))?;
        let media_type = meta.strip_suffix(";base64").unwrap_or(meta);
        return Ok(json!({
            "type": "image",
            "source": {"type": "base64", "media_type": media_type, "data": data}
        }));
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        // Anthropic supports URL image sources for remote images.
        return Ok(json!({
            "type": "image",
            "source": {"type": "url", "url": url}
        }));
    }
    bail!("无法识别的图片地址")
}

fn push_blocks(out: &mut Vec<(String, Vec<Value>)>, role: &str, blocks: Vec<Value>) {
    if blocks.is_empty() {
        return;
    }
    if let Some((last_role, last_blocks)) = out.last_mut() {
        if last_role == role {
            last_blocks.extend(blocks);
            return;
        }
    }
    out.push((role.to_string(), blocks));
}

fn map_tool_choice_chat_to_anthropic(choice: &Value) -> Value {
    match choice {
        Value::String(s) if s == "required" => json!({"type": "any"}),
        Value::String(s) if s == "auto" => json!({"type": "auto"}),
        Value::String(_) => json!({"type": "auto"}),
        Value::Object(obj) => {
            if let Some(name) = obj
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
            {
                json!({"type": "tool", "name": name})
            } else {
                json!({"type": "auto"})
            }
        }
        _ => json!({"type": "auto"}),
    }
}

// ---------------------------------------------------------------------------
// Request conversion: anthropic -> chat
// ---------------------------------------------------------------------------

fn anthropic_to_chat(body: &Value, upstream_model: &str) -> Result<Value> {
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("messages 字段缺失"))?;
    let mut chat_messages: Vec<Value> = Vec::new();

    if let Some(system) = body.get("system") {
        let text = flatten_text(Some(system));
        if !text.is_empty() {
            chat_messages.push(json!({"role": "system", "content": text}));
        }
    }

    for msg in messages {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("user");
        let content = msg.get("content");
        let mut text_parts: Vec<Value> = Vec::new();
        let mut tool_calls: Vec<Value> = Vec::new();
        let mut tool_results: Vec<Value> = Vec::new();

        match content {
            Some(Value::String(s)) => {
                if !s.is_empty() {
                    text_parts.push(json!({"type": "text", "text": s}));
                }
            }
            Some(Value::Array(blocks)) => {
                for block in blocks {
                    match block.get("type").and_then(Value::as_str).unwrap_or("text") {
                        "text" => {
                            if let Some(t) = block.get("text").and_then(Value::as_str) {
                                if !t.is_empty() {
                                    text_parts.push(json!({"type": "text", "text": t}));
                                }
                            }
                        }
                        "image" => {
                            if let Some(url) = anthropic_image_to_data_url(block) {
                                text_parts
                                    .push(json!({"type": "image_url", "image_url": {"url": url}}));
                            }
                        }
                        "tool_use" => {
                            let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
                            let name = block
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {"name": name, "arguments": input.to_string()}
                            }));
                        }
                        "tool_result" => {
                            let id = block
                                .get("tool_use_id")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let output = flatten_text(block.get("content"));
                            tool_results.push(json!({
                                "role": "tool",
                                "tool_call_id": id,
                                "content": output,
                            }));
                        }
                        "thinking" | "redacted_thinking" => {}
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        if role == "assistant" {
            let mut message = json!({"role": "assistant"});
            let content_value = chat_content_value(&text_parts);
            message["content"] = content_value;
            if !tool_calls.is_empty() {
                message["tool_calls"] = Value::Array(tool_calls);
            }
            if message
                .get("content")
                .map(|v| !v.is_null())
                .unwrap_or(false)
                || message.get("tool_calls").is_some()
            {
                chat_messages.push(message);
            }
        } else {
            for tr in tool_results {
                chat_messages.push(tr);
            }
            if !text_parts.is_empty() {
                chat_messages.push(json!({
                    "role": "user",
                    "content": chat_content_value(&text_parts)
                }));
            }
        }
    }

    let mut result = json!({
        "model": upstream_model,
        "messages": chat_messages,
        "max_tokens": body.get("max_tokens").and_then(Value::as_u64).unwrap_or(4096),
    });
    for key in ["temperature", "top_p"] {
        if let Some(v) = body.get(key) {
            result[key] = v.clone();
        }
    }
    if let Some(stop) = body.get("stop_sequences") {
        result["stop"] = stop.clone();
    }
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        let chat_tools: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                let name = t.get("name")?.as_str()?;
                Some(json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": t.get("description").cloned().unwrap_or(Value::Null),
                        "parameters": t.get("input_schema").cloned().unwrap_or_else(|| json!({"type":"object","properties":{}})),
                    }
                }))
            })
            .collect();
        if !chat_tools.is_empty() {
            result["tools"] = Value::Array(chat_tools);
            if let Some(choice) = body.get("tool_choice") {
                result["tool_choice"] = map_tool_choice_anthropic_to_chat(choice);
            }
            if body
                .get("disable_parallel_tool_use")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                result["parallel_tool_calls"] = Value::Bool(false);
            }
        }
    }
    Ok(result)
}

fn chat_content_value(parts: &[Value]) -> Value {
    if parts.is_empty() {
        return Value::Null;
    }
    if parts.len() == 1 && parts[0].get("type").and_then(Value::as_str) == Some("text") {
        return parts[0].get("text").cloned().unwrap_or(Value::Null);
    }
    Value::Array(parts.to_vec())
}

fn anthropic_image_to_data_url(block: &Value) -> Option<String> {
    let source = block.get("source")?;
    match source.get("type").and_then(Value::as_str) {
        Some("base64") => {
            let media = source
                .get("media_type")
                .and_then(Value::as_str)
                .unwrap_or("image/png");
            let data = source.get("data").and_then(Value::as_str)?;
            Some(format!("data:{media};base64,{data}"))
        }
        Some("url") => source
            .get("url")
            .and_then(Value::as_str)
            .map(|s| s.to_string()),
        _ => None,
    }
}

fn map_tool_choice_anthropic_to_chat(choice: &Value) -> Value {
    match choice.get("type").and_then(Value::as_str).unwrap_or("auto") {
        "any" => json!("required"),
        "none" => json!("none"),
        "tool" => {
            let name = choice
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            json!({"type": "function", "function": {"name": name}})
        }
        _ => json!("auto"),
    }
}

// ---------------------------------------------------------------------------
// Request conversion: chat -> responses
// ---------------------------------------------------------------------------

fn chat_to_responses(body: &Value, upstream_model: &str) -> Result<Value> {
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("messages 字段缺失"))?;
    let mut instructions: Vec<String> = Vec::new();
    let mut input: Vec<Value> = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("user");
        match role {
            "system" | "developer" => {
                let text = flatten_text(msg.get("content"));
                if !text.is_empty() {
                    instructions.push(text);
                }
            }
            "tool" => {
                let call_id = msg
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": flatten_text(msg.get("content")),
                }));
            }
            "assistant" => {
                if let Some(text) = opt_text(msg.get("content")) {
                    if !text.is_empty() {
                        input.push(json!({"role": "assistant", "content": text}));
                    }
                }
                if let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) {
                    for tc in calls {
                        let id = tc.get("id").and_then(Value::as_str).unwrap_or_default();
                        let name = tc
                            .pointer("/function/name")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let args = tc
                            .pointer("/function/arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("{}");
                        input.push(json!({
                            "type": "function_call",
                            "call_id": id,
                            "name": name,
                            "arguments": args,
                        }));
                    }
                }
            }
            _ => {
                let content = responses_input_content(msg.get("content"))?;
                input.push(json!({"role": "user", "content": content}));
            }
        }
    }

    let mut result = json!({
        "model": upstream_model,
        "input": input,
        "max_output_tokens": body
            .get("max_completion_tokens")
            .and_then(Value::as_u64)
            .or_else(|| body.get("max_tokens").and_then(Value::as_u64))
            .unwrap_or(4096),
    });
    if !instructions.is_empty() {
        result["instructions"] = Value::String(instructions.join("\n\n"));
    }
    for key in ["temperature", "top_p", "parallel_tool_calls"] {
        if let Some(v) = body.get(key) {
            result[key] = v.clone();
        }
    }
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        let resp_tools: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                let f = t.get("function")?;
                Some(json!({
                    "type": "function",
                    "name": f.get("name")?.as_str()?,
                    "description": f.get("description").cloned().unwrap_or(Value::Null),
                    "parameters": f.get("parameters").cloned().unwrap_or_else(|| json!({"type":"object","properties":{}})),
                }))
            })
            .collect();
        if !resp_tools.is_empty() {
            result["tools"] = Value::Array(resp_tools);
            if let Some(choice) = body.get("tool_choice") {
                result["tool_choice"] = match choice {
                    Value::String(s) => Value::String(s.clone()),
                    Value::Object(obj) => {
                        if let Some(name) = obj
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(Value::as_str)
                        {
                            json!({"type": "function", "name": name})
                        } else {
                            json!("auto")
                        }
                    }
                    _ => json!("auto"),
                };
            }
        }
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Request conversion: responses -> chat
// ---------------------------------------------------------------------------

fn responses_to_chat(body: &Value, upstream_model: &str) -> Result<Value> {
    let mut messages: Vec<Value> = Vec::new();
    if let Some(instructions) = body.get("instructions").and_then(Value::as_str) {
        if !instructions.trim().is_empty() {
            messages.push(json!({"role": "system", "content": instructions}));
        }
    }

    match body.get("input") {
        Some(Value::String(text)) => {
            messages.push(json!({"role": "user", "content": text}));
        }
        Some(Value::Array(items)) => {
            for item in items {
                let ty = item.get("type").and_then(Value::as_str).unwrap_or("");
                match ty {
                    "function_call" => {
                        let call_id = item
                            .get("call_id")
                            .or_else(|| item.get("id"))
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                        let arguments = item
                            .get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("{}");
                        let call = json!({
                            "id": call_id,
                            "type": "function",
                            "function": {"name": name, "arguments": arguments}
                        });
                        if let Some(last) = messages.last_mut() {
                            if last.get("role").and_then(Value::as_str) == Some("assistant") {
                                match last.get_mut("tool_calls") {
                                    Some(Value::Array(calls)) => calls.push(call),
                                    _ => last["tool_calls"] = json!([call]),
                                }
                                continue;
                            }
                        }
                        messages.push(json!({
                            "role": "assistant",
                            "content": Value::Null,
                            "tool_calls": [call]
                        }));
                    }
                    "function_call_output" => {
                        let call_id = item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let output = match item.get("output") {
                            Some(Value::String(s)) => s.clone(),
                            Some(other) => flatten_text(Some(other)),
                            None => String::new(),
                        };
                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": output
                        }));
                    }
                    _ => {
                        let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
                        let content = responses_content_to_chat(item.get("content"))?;
                        let chat_role = if role == "developer" { "system" } else { role };
                        messages.push(json!({"role": chat_role, "content": content}));
                    }
                }
            }
        }
        None => {
            return Err(anyhow!("Responses 请求缺少 input 字段"));
        }
        _ => return Err(anyhow!("Responses 请求的 input 类型无效")),
    }

    if messages.is_empty() {
        return Err(anyhow!("Responses 请求没有可转换的消息"));
    }

    let mut result = json!({
        "model": upstream_model,
        "messages": messages,
        "max_tokens": body
            .get("max_output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(4096),
    });
    for key in ["temperature", "top_p", "parallel_tool_calls"] {
        if let Some(v) = body.get(key) {
            result[key] = v.clone();
        }
    }
    if let Some(effort) = body
        .pointer("/reasoning/effort")
        .and_then(Value::as_str)
        .filter(|effort| !effort.is_empty())
    {
        result["reasoning_effort"] = Value::String(effort.to_string());
    }
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        let chat_tools: Vec<Value> = tools
            .iter()
            .filter(|t| t.get("type").and_then(Value::as_str).unwrap_or("function") == "function")
            .filter_map(|t| {
                let name = t.get("name").and_then(Value::as_str)?;
                Some(json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": t.get("description").cloned().unwrap_or(Value::Null),
                        "parameters": t
                            .get("parameters")
                            .cloned()
                            .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
                    }
                }))
            })
            .collect();
        if !chat_tools.is_empty() {
            result["tools"] = Value::Array(chat_tools);
            if let Some(choice) = body.get("tool_choice") {
                result["tool_choice"] = match choice {
                    Value::String(s) => Value::String(s.clone()),
                    Value::Object(obj) => {
                        if let Some(name) = obj.get("name").and_then(Value::as_str) {
                            json!({"type": "function", "function": {"name": name}})
                        } else {
                            json!("auto")
                        }
                    }
                    _ => json!("auto"),
                };
            }
        }
    }
    Ok(result)
}

fn responses_content_to_chat(content: Option<&Value>) -> Result<Value> {
    match content {
        Some(Value::String(s)) => Ok(json!(s)),
        Some(Value::Array(parts)) => {
            let mut out = Vec::new();
            for p in parts {
                match p.get("type").and_then(Value::as_str).unwrap_or("") {
                    "input_text" | "output_text" | "text" => out.push(json!({
                        "type": "text",
                        "text": p.get("text").cloned().unwrap_or(Value::Null),
                    })),
                    "input_image" => {
                        let url = p
                            .get("image_url")
                            .and_then(|v| {
                                v.as_str().map(str::to_string).or_else(|| {
                                    p.pointer("/image_url/url")
                                        .and_then(Value::as_str)
                                        .map(str::to_string)
                                })
                            })
                            .unwrap_or_default();
                        out.push(json!({"type": "image_url", "image_url": {"url": url}}));
                    }
                    "" => {}
                    other => bail!("Responses 客户端内容类型 {other} 暂不支持"),
                }
            }
            if out.len() == 1 && out[0].get("type").and_then(Value::as_str) == Some("text") {
                return Ok(out[0].get("text").cloned().unwrap_or(Value::Null));
            }
            Ok(Value::Array(out))
        }
        Some(other) => Err(anyhow!("Responses 内容类型无效：{other}")),
        None => Ok(Value::Null),
    }
}

fn responses_input_content(content: Option<&Value>) -> Result<Value> {
    match content {
        Some(Value::String(s)) => Ok(json!(s)),
        Some(Value::Array(parts)) => {
            let mut out = Vec::new();
            for p in parts {
                match p.get("type").and_then(Value::as_str).unwrap_or("text") {
                    "text" => out.push(json!({
                        "type": "input_text",
                        "text": p.get("text").cloned().unwrap_or(Value::Null),
                    })),
                    "image_url" => out.push(json!({
                        "type": "input_image",
                        "image_url": p.pointer("/image_url/url").cloned().unwrap_or(Value::Null),
                    })),
                    other => bail!("Responses 上游不支持内容类型 {other}"),
                }
            }
            Ok(Value::Array(out))
        }
        _ => Ok(json!("")),
    }
}

// ---------------------------------------------------------------------------
// Response conversion
// ---------------------------------------------------------------------------

fn anthropic_response_to_chat(value: &Value, client_model: &str) -> Value {
    let content = value
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for block in &content {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(Value::as_str) {
                    text.push_str(t);
                }
            }
            Some("tool_use") => {
                tool_calls.push(json!({
                    "id": block.get("id").cloned().unwrap_or(Value::Null),
                    "type": "function",
                    "function": {
                        "name": block.get("name").cloned().unwrap_or(Value::Null),
                        "arguments": block.get("input").cloned().unwrap_or_else(|| json!({})).to_string(),
                    }
                }));
            }
            _ => {}
        }
    }
    let usage = value.get("usage").cloned().unwrap_or(Value::Null);
    let mut message = json!({
        "role": "assistant",
        "content": if text.is_empty() { Value::Null } else { Value::String(text) },
    });
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls);
    }
    let finish = match value.get("stop_reason").and_then(Value::as_str) {
        Some("max_tokens") => "length",
        Some("tool_use") => "tool_calls",
        Some("refusal") => "content_filter",
        _ => "stop",
    };
    json!({
        "id": value.get("id").cloned().unwrap_or_else(|| json!("chatcmpl-yoyo")),
        "object": "chat.completion",
        "created": value.get("created").and_then(Value::as_i64).unwrap_or_else(now_secs),
        "model": client_model,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish,
        }],
        "usage": Usage {
            input_tokens: usage.get("input_tokens").and_then(Value::as_u64).unwrap_or(0),
            cached_tokens: 0,
            output_tokens: usage.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
            total_tokens: 0,
            estimated: false,
        }.openai_json(),
    })
}

fn chat_response_to_anthropic(value: &Value, client_model: &str) -> Value {
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let message = choice.get("message").cloned().unwrap_or_else(|| json!({}));
    let mut content: Vec<Value> = Vec::new();
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        if !text.is_empty() {
            content.push(json!({"type": "text", "text": text}));
        }
    } else if let Some(parts) = message.get("content").and_then(Value::as_array) {
        for p in parts {
            if p.get("type").and_then(Value::as_str) == Some("text") {
                content.push(
                    json!({"type": "text", "text": p.get("text").cloned().unwrap_or(Value::Null)}),
                );
            }
        }
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for (idx, tc) in calls.iter().enumerate() {
            let name = tc
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = tc
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let input = serde_json::from_str::<Value>(args).unwrap_or_else(|_| json!({}));
            content.push(json!({
                "type": "tool_use",
                "id": tc.get("id").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("toolu_{idx}")),
                "name": name,
                "input": input,
            }));
        }
    }
    let finish = match choice.get("finish_reason").and_then(Value::as_str) {
        Some("length") => "max_tokens",
        Some("tool_calls") | Some("function_call") => "tool_use",
        Some("content_filter") => "refusal",
        _ => "end_turn",
    };
    let usage = value.get("usage").cloned().unwrap_or(Value::Null);
    json!({
        "id": value.get("id").cloned().unwrap_or_else(|| json!("msg_yoyo")),
        "type": "message",
        "role": "assistant",
        "model": client_model,
        "content": content,
        "stop_reason": finish,
        "stop_sequence": Value::Null,
        "usage": {
            "input_tokens": usage.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
            "output_tokens": usage.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0),
        }
    })
}

fn chat_response_to_responses(value: &Value, client_model: &str) -> Value {
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let message = choice.get("message").cloned().unwrap_or_else(|| json!({}));
    let text = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let finish = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .unwrap_or("stop");
    let id = responses_id(value.get("id").and_then(Value::as_str));
    let mut output = Vec::new();
    if !text.is_empty() {
        output.push(json!({
            "id": format!("msg_{}", short_id()),
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": text,
                "annotations": []
            }]
        }));
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for (index, tc) in calls.iter().enumerate() {
            let call_id = tc
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("call_{index}"));
            output.push(json!({
                "id": format!("fc_{}", short_id()),
                "type": "function_call",
                "status": "completed",
                "call_id": call_id,
                "name": tc
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                "arguments": tc
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}"),
            }));
        }
    }
    let usage = value.get("usage").cloned().unwrap_or(Value::Null);
    let input_tokens = usage
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(input_tokens + output_tokens);
    let incomplete = finish == "length";
    let mut response = json!({
        "id": id,
        "object": "response",
        "created_at": value
            .get("created")
            .and_then(Value::as_i64)
            .unwrap_or_else(now_secs),
        "status": if incomplete { "incomplete" } else { "completed" },
        "model": client_model,
        "output": output,
        "output_text": text,
        "usage": {
            "input_tokens": input_tokens,
            "input_tokens_details": {"cached_tokens": 0},
            "output_tokens": output_tokens,
            "output_tokens_details": {"reasoning_tokens": 0},
            "total_tokens": total_tokens,
        },
        "error": Value::Null,
    });
    if incomplete {
        response["incomplete_details"] = json!({"reason": "max_output_tokens"});
    }
    response
}

fn responses_id(id: Option<&str>) -> String {
    match id {
        Some(value) if value.starts_with("resp_") => value.to_string(),
        Some(value) if !value.is_empty() => format!("resp_{value}"),
        _ => format!("resp_{}", short_id()),
    }
}

fn responses_response_to_chat(value: &Value, client_model: &str) -> Value {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    if let Some(output) = value.get("output").and_then(Value::as_array) {
        for item in output {
            match item.get("type").and_then(Value::as_str) {
                Some("message") => {
                    if let Some(parts) = item.get("content").and_then(Value::as_array) {
                        for p in parts {
                            if p.get("type").and_then(Value::as_str) == Some("output_text") {
                                if let Some(t) = p.get("text").and_then(Value::as_str) {
                                    text.push_str(t);
                                }
                            }
                        }
                    }
                }
                Some("function_call") => {
                    tool_calls.push(json!({
                        "id": item.get("call_id").cloned().unwrap_or_else(|| item.get("id").cloned().unwrap_or(Value::Null)),
                        "type": "function",
                        "function": {
                            "name": item.get("name").cloned().unwrap_or(Value::Null),
                            "arguments": item.get("arguments").cloned().unwrap_or_else(|| json!("{}")),
                        }
                    }));
                }
                _ => {}
            }
        }
    }
    let finish = if !tool_calls.is_empty() {
        "tool_calls"
    } else if value
        .pointer("/incomplete_details/reason")
        .and_then(Value::as_str)
        == Some("max_output_tokens")
    {
        "length"
    } else {
        "stop"
    };
    let usage = value.get("usage").cloned().unwrap_or(Value::Null);
    let mut message = json!({
        "role": "assistant",
        "content": if text.is_empty() { Value::Null } else { Value::String(text) },
    });
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls);
    }
    json!({
        "id": value.get("id").cloned().unwrap_or_else(|| json!("chatcmpl-yoyo")),
        "object": "chat.completion",
        "created": value.get("created_at").and_then(Value::as_i64).unwrap_or_else(now_secs),
        "model": client_model,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish,
        }],
        "usage": Usage {
            input_tokens: usage.get("input_tokens").and_then(Value::as_u64).unwrap_or(0),
            cached_tokens: 0,
            output_tokens: usage.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
            total_tokens: usage.get("total_tokens").and_then(Value::as_u64).unwrap_or(0),
            estimated: false,
        }.openai_json(),
    })
}

// ---------------------------------------------------------------------------
// SSE decoding
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct SseDecoder {
    buf: Vec<u8>,
    current: SseEvent,
}

impl SseDecoder {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            let Some(pos) = self.buf.iter().position(|&b| b == b'\n') else {
                break;
            };
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line).to_string();
            if line.is_empty() {
                if !self.current.data.is_empty() || self.current.event.is_some() {
                    out.push(std::mem::take(&mut self.current));
                }
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("event:") {
                self.current.event = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("data:") {
                let rest = rest.strip_prefix(' ').unwrap_or(rest);
                if !self.current.data.is_empty() {
                    self.current.data.push('\n');
                }
                self.current.data.push_str(rest);
            }
        }
        out
    }

    pub fn flush(&mut self) -> Option<SseEvent> {
        if self.current.data.is_empty() && self.current.event.is_none() {
            None
        } else {
            Some(std::mem::take(&mut self.current))
        }
    }
}

// ---------------------------------------------------------------------------
// Streaming translation
// ---------------------------------------------------------------------------

#[derive(Default)]
struct RespToolState {
    item_id: String,
    output_index: usize,
    call_id: String,
    name: String,
    arguments: String,
}

pub struct StreamTranslator {
    ingress: Protocol,
    upstream: Protocol,
    client_model: String,
    usage: Usage,
    id: String,
    created: i64,
    finished: bool,
    chat_role_sent: bool,
    anthropic_started: bool,
    anthropic_next_block: usize,
    anthropic_text_block: Option<usize>,
    anthropic_open_blocks: Vec<usize>,
    anthropic_tool_by_openai: HashMap<u64, usize>,
    anthropic_stop_reason: Option<String>,
    openai_tool_started: bool,
    responses_tool_by_id: HashMap<String, usize>,
    responses_stop_reason: Option<String>,
    resp_started: bool,
    resp_text_item: Option<String>,
    resp_text_output: Option<usize>,
    resp_content_open: bool,
    resp_text: String,
    resp_tools: HashMap<u64, RespToolState>,
    resp_next_output: usize,
    /// Characters of assistant text the upstream has emitted so far. The
    /// gateway divides this to drive the live tokens-per-second meter.
    produced_chars: usize,
}

impl StreamTranslator {
    pub fn new(ingress: Protocol, upstream: Protocol, client_model: &str) -> Self {
        let id = match ingress {
            Protocol::OpenAiChat => format!("chatcmpl-{}", short_id()),
            Protocol::AnthropicMessages => format!("msg_{}", short_id()),
            Protocol::OpenAiResponses => format!("resp_{}", short_id()),
        };
        Self {
            ingress,
            upstream,
            client_model: client_model.to_string(),
            usage: Usage::default(),
            id,
            created: now_secs(),
            finished: false,
            chat_role_sent: false,
            anthropic_started: false,
            anthropic_next_block: 0,
            anthropic_text_block: None,
            anthropic_open_blocks: Vec::new(),
            anthropic_tool_by_openai: HashMap::new(),
            anthropic_stop_reason: None,
            openai_tool_started: false,
            responses_tool_by_id: HashMap::new(),
            responses_stop_reason: None,
            resp_started: false,
            resp_text_item: None,
            resp_text_output: None,
            resp_content_open: false,
            resp_text: String::new(),
            produced_chars: 0,
            resp_tools: HashMap::new(),
            resp_next_output: 0,
        }
    }

    pub fn usage(&self) -> Usage {
        self.usage
    }

    /// Characters of generated assistant text seen so far.
    pub fn produced_chars(&self) -> usize {
        self.produced_chars
    }

    /// Add upstream-generated text to the throughput counter. Called from the
    /// upstream-dialect parsers, before conversion to the client dialect.
    fn note_produced(&mut self, text: &str) {
        self.produced_chars = self.produced_chars.saturating_add(text.chars().count());
    }

    /// Count reasoning/thinking deltas in the live throughput meter.
    ///
    /// Reasoning tokens are generated at the same rate as the answer and
    /// usually arrive first, so ignoring them made the meter read 0 tok/s for
    /// the entire thinking phase of a reasoning model.
    fn note_reasoning(&mut self, value: &Value) {
        // Accept a whole SSE event (`{ delta }`, `{ choices: [{ delta }] }`)
        // or the delta object itself.
        let delta = value
            .get("delta")
            .or_else(|| value.pointer("/choices/0/delta"))
            .unwrap_or(value);
        // Responses-style reasoning emits a plain string delta.
        if delta.is_string() {
            if let Some(text) = delta.as_str() {
                self.note_produced(text);
            }
            return;
        }
        for key in ["reasoning_content", "reasoning", "thinking"] {
            if let Some(text) = delta.get(key).and_then(Value::as_str) {
                self.note_produced(text);
                return;
            }
        }
        if let Some(text) = value
            .pointer("/content_block/thinking")
            .and_then(Value::as_str)
        {
            self.note_produced(text);
        }
    }

    /// A terminal error frame in the client's own dialect.
    pub fn error_frame(&self, message: &str) -> String {
        let message = message.replace('"', "'");
        match self.ingress {
            Protocol::AnthropicMessages => anthropic_event_frame(
                "error",
                &json!({
                    "type": "error",
                    "error": {"type": "api_error", "message": message}
                }),
            ),
            Protocol::OpenAiResponses => json_frame(
                Some("response.failed"),
                &json!({
                    "type": "response.failed",
                    "response": {"status": "failed", "error": {"message": message}}
                }),
            ),
            Protocol::OpenAiChat => json_frame(
                None,
                &json!({"error": {"message": message, "type": "yoyo_relay_error"}}),
            ),
        }
    }

    pub fn translate(&mut self, ev: &SseEvent) -> Vec<String> {
        if self.ingress == self.upstream {
            return self.passthrough(ev);
        }
        match (self.ingress, self.upstream) {
            (Protocol::OpenAiChat, Protocol::AnthropicMessages) => self.chat_from_anthropic(ev),
            (Protocol::AnthropicMessages, Protocol::OpenAiChat) => self.anthropic_from_chat(ev),
            (Protocol::OpenAiChat, Protocol::OpenAiResponses) => self.chat_from_responses(ev),
            (Protocol::AnthropicMessages, Protocol::OpenAiResponses) => {
                self.anthropic_from_responses(ev)
            }
            (Protocol::OpenAiResponses, Protocol::OpenAiChat) => self.responses_from_chat(ev),
            (Protocol::OpenAiResponses, Protocol::AnthropicMessages) => {
                self.responses_from_anthropic(ev)
            }
            _ => Vec::new(),
        }
    }

    pub fn finish(&mut self) -> Vec<String> {
        if self.finished {
            return Vec::new();
        }
        if self.ingress == self.upstream {
            self.finished = true;
            if self.ingress == Protocol::OpenAiChat {
                return vec![done_frame()];
            }
            return Vec::new();
        }
        match (self.ingress, self.upstream) {
            (Protocol::OpenAiChat, Protocol::AnthropicMessages) => {
                self.finished = true;
                let finish = self
                    .anthropic_stop_reason
                    .clone()
                    .unwrap_or_else(|| "stop".into());
                let mut out = vec![self.chat_final_chunk(&finish)];
                out.push(done_frame());
                out
            }
            (Protocol::AnthropicMessages, Protocol::OpenAiChat) => {
                self.finished = true;
                self.anthropic_close_all()
            }
            (Protocol::OpenAiChat, Protocol::OpenAiResponses) => {
                self.finished = true;
                let finish = self
                    .responses_stop_reason
                    .clone()
                    .unwrap_or_else(|| "stop".into());
                vec![self.chat_final_chunk(&finish), done_frame()]
            }
            (Protocol::AnthropicMessages, Protocol::OpenAiResponses) => {
                self.finished = true;
                self.anthropic_close_all()
            }
            (Protocol::OpenAiResponses, _) => self.responses_complete(),
            _ => {
                self.finished = true;
                Vec::new()
            }
        }
    }

    // -- passthrough --------------------------------------------------------

    fn passthrough(&mut self, ev: &SseEvent) -> Vec<String> {
        let data = ev.data.trim();
        if data == "[DONE]" {
            self.finished = true;
            return if self.ingress == Protocol::OpenAiChat {
                vec![done_frame()]
            } else {
                Vec::new()
            };
        }
        let Ok(mut value) = serde_json::from_str::<Value>(data) else {
            return vec![raw_frame(ev.event.as_deref(), data)];
        };
        self.collect_usage_from_event(&value);
        // Passthrough streams never run through the dialect parsers, so count
        // generated text here to keep the live meter accurate.
        self.note_reasoning(&value);
        match self.ingress {
            Protocol::OpenAiChat => {
                if let Some(text) = value
                    .pointer("/choices/0/delta/content")
                    .and_then(Value::as_str)
                {
                    self.note_produced(text);
                }
            }
            Protocol::AnthropicMessages => {
                if value.get("type").and_then(Value::as_str) == Some("content_block_delta") {
                    if let Some(text) = value.pointer("/delta/text").and_then(Value::as_str) {
                        self.note_produced(text);
                    }
                }
            }
            Protocol::OpenAiResponses => {
                if value.get("type").and_then(Value::as_str) == Some("response.output_text.delta") {
                    if let Some(text) = value.get("delta").and_then(Value::as_str) {
                        self.note_produced(text);
                    }
                }
            }
        }
        if self.ingress == Protocol::OpenAiResponses
            && value.get("type").and_then(Value::as_str) == Some("response.completed")
        {
            self.finished = true;
        }
        if value.get("model").is_some() {
            value["model"] = Value::String(self.client_model.clone());
        }
        if let Some(message) = value.get_mut("message") {
            if message.get("model").is_some() {
                message["model"] = Value::String(self.client_model.clone());
            }
        }
        if let Some(response) = value.get_mut("response") {
            if response.get("model").is_some() {
                response["model"] = Value::String(self.client_model.clone());
            }
        }
        vec![json_frame(ev.event.as_deref(), &value)]
    }

    fn collect_usage_from_event(&mut self, value: &Value) {
        let usage = match self.upstream {
            Protocol::OpenAiChat => value.get("usage"),
            Protocol::AnthropicMessages => value
                .pointer("/message/usage")
                .or_else(|| value.get("usage")),
            Protocol::OpenAiResponses => value
                .pointer("/response/usage")
                .or_else(|| value.get("usage")),
        };
        if let Some(usage) = usage.and_then(Value::as_object) {
            let u = usage_from_object(self.upstream, usage);
            self.usage.absorb_max(&u);
        }
    }

    // -- chat ingress <- anthropic upstream --------------------------------

    fn chat_from_anthropic(&mut self, ev: &SseEvent) -> Vec<String> {
        let event_type = ev
            .event
            .clone()
            .or_else(|| {
                serde_json::from_str::<Value>(&ev.data)
                    .ok()
                    .and_then(|v| v.get("type").and_then(Value::as_str).map(str::to_string))
            })
            .unwrap_or_default();
        let data: Value = serde_json::from_str(&ev.data).unwrap_or_else(|_| json!({}));
        let mut out = Vec::new();
        out.push(self.chat_role_chunk());
        match event_type.as_str() {
            "message_start" => {
                self.usage.input_tokens = data
                    .pointer("/message/usage/input_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                if self.ingress == Protocol::OpenAiChat {
                    if let Some(id) = data.pointer("/message/id").and_then(Value::as_str) {
                        self.id = format!("chatcmpl-{id}");
                    }
                }
                out.push(self.chat_role_chunk());
            }
            "content_block_start" => {
                let block = data
                    .get("content_block")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    let block_index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
                    let tool_index = self.anthropic_tool_by_openai.len() as u64;
                    self.anthropic_tool_by_openai
                        .insert(block_index, tool_index as usize);
                    out.push(
                        self.chat_tool_start_chunk(
                            tool_index,
                            block.get("id").and_then(Value::as_str).unwrap_or_default(),
                            block
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        ),
                    );
                }
            }
            "content_block_delta" => {
                let delta = data.get("delta").cloned().unwrap_or_else(|| json!({}));
                self.note_reasoning(&delta);
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            self.note_produced(text);
                            if !text.is_empty() {
                                out.push(self.chat_content_chunk(text));
                            }
                        }
                    }
                    Some("input_json_delta") => {
                        let block_index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
                        let tool_index = self
                            .anthropic_tool_by_openai
                            .get(&block_index)
                            .copied()
                            .unwrap_or(0) as u64;
                        let partial = delta
                            .get("partial_json")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        if !partial.is_empty() {
                            out.push(self.chat_tool_args_chunk(tool_index, partial));
                        }
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(reason) = data.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.anthropic_stop_reason = Some(map_stop_anthropic_to_chat(reason).into());
                }
                if let Some(out_tokens) =
                    data.pointer("/usage/output_tokens").and_then(Value::as_u64)
                {
                    self.usage.output_tokens = out_tokens;
                }
            }
            "message_stop" => {
                let finish = self
                    .anthropic_stop_reason
                    .clone()
                    .unwrap_or_else(|| "stop".into());
                out.push(self.chat_final_chunk(&finish));
                out.push(done_frame());
                self.finished = true;
            }
            "error" => {
                out.push(json_frame(
                    None,
                    &json!({"error": data.get("error").cloned().unwrap_or(data)}),
                ));
                self.finished = true;
            }
            _ => {}
        }
        out
    }

    // -- anthropic ingress <- chat upstream --------------------------------

    fn anthropic_from_chat(&mut self, ev: &SseEvent) -> Vec<String> {
        let data = ev.data.trim();
        let mut out = Vec::new();
        if data == "[DONE]" {
            out.extend(self.anthropic_close_all());
            self.finished = true;
            return out;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return Vec::new();
        };
        self.collect_usage_from_event(&value);
        out.extend(self.anthropic_ensure_started());
        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
        else {
            return out;
        };
        let delta = choice.get("delta").cloned().unwrap_or_else(|| json!({}));
        self.note_reasoning(&delta);
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                self.note_produced(text);
                out.extend(self.anthropic_text_delta(text));
            }
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for tc in calls {
                let openai_index = tc.get("index").and_then(Value::as_u64).unwrap_or(0);
                let block_index = if let Some(idx) =
                    self.anthropic_tool_by_openai.get(&openai_index)
                {
                    *idx
                } else {
                    let idx = self.anthropic_next_block;
                    self.anthropic_next_block += 1;
                    self.anthropic_tool_by_openai.insert(openai_index, idx);
                    self.anthropic_open_blocks.push(idx);
                    let id = tc
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("toolu_{openai_index}"));
                    let name = tc
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    out.push(anthropic_event_frame(
                        "content_block_start",
                        &json!({
                            "type": "content_block_start",
                            "index": idx,
                            "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}
                        }),
                    ));
                    idx
                };
                if let Some(args) = tc.pointer("/function/arguments").and_then(Value::as_str) {
                    if !args.is_empty() {
                        out.push(anthropic_event_frame(
                            "content_block_delta",
                            &json!({
                                "type": "content_block_delta",
                                "index": block_index,
                                "delta": {"type": "input_json_delta", "partial_json": args}
                            }),
                        ));
                    }
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.anthropic_stop_reason = Some(map_stop_chat_to_anthropic(reason).into());
        }
        out
    }

    fn anthropic_ensure_started(&mut self) -> Vec<String> {
        if self.anthropic_started {
            return Vec::new();
        }
        self.anthropic_started = true;
        vec![anthropic_event_frame(
            "message_start",
            &json!({
                "type": "message_start",
                "message": {
                    "id": self.id,
                    "type": "message",
                    "role": "assistant",
                    "model": self.client_model,
                    "content": [],
                    "stop_reason": Value::Null,
                    "stop_sequence": Value::Null,
                    "usage": {"input_tokens": self.usage.input_tokens, "output_tokens": 0}
                }
            }),
        )]
    }

    fn anthropic_text_delta(&mut self, text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let idx = if let Some(idx) = self.anthropic_text_block {
            idx
        } else {
            let idx = self.anthropic_next_block;
            self.anthropic_next_block += 1;
            self.anthropic_text_block = Some(idx);
            self.anthropic_open_blocks.push(idx);
            out.push(anthropic_event_frame(
                "content_block_start",
                &json!({
                    "type": "content_block_start",
                    "index": idx,
                    "content_block": {"type": "text", "text": ""}
                }),
            ));
            idx
        };
        out.push(anthropic_event_frame(
            "content_block_delta",
            &json!({
                "type": "content_block_delta",
                "index": idx,
                "delta": {"type": "text_delta", "text": text}
            }),
        ));
        out
    }

    fn anthropic_close_all(&mut self) -> Vec<String> {
        if !self.anthropic_started {
            self.anthropic_started = true;
            let mut out = vec![anthropic_event_frame(
                "message_start",
                &json!({
                    "type": "message_start",
                    "message": {
                        "id": self.id,
                        "type": "message",
                        "role": "assistant",
                        "model": self.client_model,
                        "content": [],
                        "stop_reason": Value::Null,
                        "stop_sequence": Value::Null,
                        "usage": {"input_tokens": self.usage.input_tokens, "output_tokens": 0}
                    }
                }),
            )];
            out.extend(self.anthropic_finish_frames());
            return out;
        }
        let mut out = Vec::new();
        for idx in self.anthropic_open_blocks.clone() {
            out.push(anthropic_event_frame(
                "content_block_stop",
                &json!({"type": "content_block_stop", "index": idx}),
            ));
        }
        self.anthropic_open_blocks.clear();
        out.extend(self.anthropic_finish_frames());
        out
    }

    fn anthropic_finish_frames(&mut self) -> Vec<String> {
        let stop = self
            .anthropic_stop_reason
            .clone()
            .unwrap_or_else(|| "end_turn".into());
        vec![
            anthropic_event_frame(
                "message_delta",
                &json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": stop, "stop_sequence": Value::Null},
                    "usage": {
                        "input_tokens": self.usage.input_tokens,
                        "output_tokens": self.usage.output_tokens
                    }
                }),
            ),
            anthropic_event_frame("message_stop", &json!({"type": "message_stop"})),
        ]
    }

    // -- chat ingress <- responses upstream --------------------------------

    fn chat_from_responses(&mut self, ev: &SseEvent) -> Vec<String> {
        let event_type = ev
            .event
            .clone()
            .or_else(|| {
                serde_json::from_str::<Value>(&ev.data)
                    .ok()
                    .and_then(|v| v.get("type").and_then(Value::as_str).map(str::to_string))
            })
            .unwrap_or_default();
        let data: Value = serde_json::from_str(&ev.data).unwrap_or_else(|_| json!({}));
        let mut out = Vec::new();
        out.push(self.chat_role_chunk());
        match event_type.as_str() {
            "response.created" => {
                out.push(self.chat_role_chunk());
            }
            "response.output_text.delta" => {
                if let Some(text) = data.get("delta").and_then(Value::as_str) {
                    self.note_produced(text);
                    if !text.is_empty() {
                        out.push(self.chat_content_chunk(text));
                    }
                }
            }
            // Reasoning summaries are generated tokens too; count them for
            // the live meter even though the client does not see them.
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                self.note_reasoning(&data);
            }
            "response.output_item.added" => {
                if let Some(item) = data.get("item") {
                    if item.get("type").and_then(Value::as_str) == Some("function_call") {
                        let item_id = item
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let index = self.responses_tool_by_id.len() as u64;
                        self.responses_tool_by_id.insert(item_id, index as usize);
                        self.openai_tool_started = true;
                        out.push(
                            self.chat_tool_start_chunk(
                                index,
                                item.get("call_id")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default(),
                                item.get("name").and_then(Value::as_str).unwrap_or_default(),
                            ),
                        );
                    }
                }
            }
            "response.function_call_arguments.delta" => {
                let item_id = data
                    .get("item_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if let Some(index) = self.responses_tool_by_id.get(item_id) {
                    let delta = data
                        .get("delta")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !delta.is_empty() {
                        out.push(self.chat_tool_args_chunk(*index as u64, delta));
                    }
                }
            }
            "response.completed" => {
                self.collect_usage_from_event(&data);
                let finish = if self.openai_tool_started {
                    "tool_calls"
                } else {
                    "stop"
                };
                out.push(self.chat_final_chunk(finish));
                out.push(done_frame());
                self.finished = true;
            }
            "response.incomplete" => {
                let finish = if self.pointer_reason(&data) == Some("max_output_tokens") {
                    "length"
                } else {
                    "stop"
                };
                out.push(self.chat_final_chunk(finish));
                out.push(done_frame());
                self.finished = true;
            }
            "response.failed" | "error" => {
                out.push(json_frame(
                    None,
                    &json!({"error": data.get("error").cloned().unwrap_or(data.clone())}),
                ));
                self.finished = true;
            }
            _ => {}
        }
        out
    }

    // -- anthropic ingress <- responses upstream ---------------------------

    fn anthropic_from_responses(&mut self, ev: &SseEvent) -> Vec<String> {
        let frames = self.chat_from_responses(ev);
        let mut out = Vec::new();
        for frame in frames {
            if let Some(chat_event) = parse_sse_frame(&frame) {
                out.extend(self.anthropic_from_chat(&chat_event));
            }
        }
        out
    }

    // -- responses ingress <- chat upstream --------------------------------

    fn responses_from_chat(&mut self, ev: &SseEvent) -> Vec<String> {
        let data = ev.data.trim();
        if data == "[DONE]" {
            return self.responses_complete();
        }
        match serde_json::from_str::<Value>(data) {
            Ok(value) => self.responses_ingest_chunk(&value),
            Err(_) => Vec::new(),
        }
    }

    // -- responses ingress <- anthropic upstream ---------------------------

    fn responses_from_anthropic(&mut self, ev: &SseEvent) -> Vec<String> {
        let frames = self.chat_from_anthropic(ev);
        // The chat bridge marks the stream finished on `message_stop`; the
        // Responses completion still has to be emitted from the trailing
        // [DONE] frame, so clear the flag before closing.
        if frames.iter().any(|frame| frame.contains("data: [DONE]")) {
            self.finished = false;
        }
        let mut out = Vec::new();
        for frame in frames {
            if let Some(chat_event) = parse_sse_frame(&frame) {
                if chat_event.data.trim() == "[DONE]" {
                    out.extend(self.responses_complete());
                } else if let Ok(value) = serde_json::from_str::<Value>(&chat_event.data) {
                    out.extend(self.responses_ingest_chunk(&value));
                }
            }
        }
        out
    }

    fn responses_ensure_started(&mut self) -> Vec<String> {
        if self.resp_started {
            return Vec::new();
        }
        self.resp_started = true;
        vec![json_frame(
            Some("response.created"),
            &json!({
                "type": "response.created",
                "response": {
                    "id": self.id,
                    "object": "response",
                    "created_at": self.created,
                    "status": "in_progress",
                    "model": self.client_model,
                    "output": [],
                }
            }),
        )]
    }

    fn responses_ingest_chunk(&mut self, value: &Value) -> Vec<String> {
        let mut out = self.responses_ensure_started();
        if let Some(usage) = value.get("usage").and_then(Value::as_object) {
            let u = Usage {
                input_tokens: usage
                    .get("prompt_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                cached_tokens: 0,
                output_tokens: usage
                    .get("completion_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                total_tokens: usage
                    .get("total_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                estimated: false,
            };
            self.usage.absorb_max(&u);
        }
        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
        else {
            return out;
        };
        let delta = choice.get("delta").cloned().unwrap_or_else(|| json!({}));
        self.note_reasoning(&delta);
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            if !text.is_empty() {
                self.note_produced(text);
                if self.resp_text_item.is_none() {
                    let item_id = format!("msg_{}", short_id());
                    let output_index = self.resp_next_output;
                    self.resp_next_output += 1;
                    self.resp_text_item = Some(item_id.clone());
                    self.resp_text_output = Some(output_index);
                    self.resp_content_open = true;
                    out.push(json_frame(
                        Some("response.output_item.added"),
                        &json!({
                            "type": "response.output_item.added",
                            "output_index": output_index,
                            "item": {
                                "id": item_id,
                                "type": "message",
                                "status": "in_progress",
                                "role": "assistant",
                                "content": [],
                            }
                        }),
                    ));
                    out.push(json_frame(
                        Some("response.content_part.added"),
                        &json!({
                            "type": "response.content_part.added",
                            "item_id": item_id,
                            "output_index": output_index,
                            "content_index": 0,
                            "part": {"type": "output_text", "text": "", "annotations": []}
                        }),
                    ));
                }
                self.resp_text.push_str(text);
                let item_id = self.resp_text_item.clone().unwrap_or_default();
                let output_index = self.resp_text_output.unwrap_or(0);
                out.push(json_frame(
                    Some("response.output_text.delta"),
                    &json!({
                        "type": "response.output_text.delta",
                        "item_id": item_id,
                        "output_index": output_index,
                        "content_index": 0,
                        "delta": text,
                    }),
                ));
            }
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for tc in calls {
                let tool_index = tc.get("index").and_then(Value::as_u64).unwrap_or(0);
                if !self.resp_tools.contains_key(&tool_index) {
                    let output_index = self.resp_next_output;
                    self.resp_next_output += 1;
                    let item_id = format!("fc_{}", short_id());
                    let call_id = tc
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("call_{tool_index}"));
                    let name = tc
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    self.resp_tools.insert(
                        tool_index,
                        RespToolState {
                            item_id: item_id.clone(),
                            output_index,
                            call_id: call_id.clone(),
                            name: name.clone(),
                            arguments: String::new(),
                        },
                    );
                    out.push(json_frame(
                        Some("response.output_item.added"),
                        &json!({
                            "type": "response.output_item.added",
                            "output_index": output_index,
                            "item": {
                                "id": item_id,
                                "type": "function_call",
                                "status": "in_progress",
                                "call_id": call_id,
                                "name": name,
                                "arguments": "",
                            }
                        }),
                    ));
                }
                if let Some(args) = tc.pointer("/function/arguments").and_then(Value::as_str) {
                    if !args.is_empty() {
                        let (item_id, output_index) = {
                            let state = self.resp_tools.get_mut(&tool_index).unwrap();
                            state.arguments.push_str(args);
                            (state.item_id.clone(), state.output_index)
                        };
                        out.push(json_frame(
                            Some("response.function_call_arguments.delta"),
                            &json!({
                                "type": "response.function_call_arguments.delta",
                                "item_id": item_id,
                                "output_index": output_index,
                                "delta": args,
                            }),
                        ));
                    }
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.responses_stop_reason = Some(reason.to_string());
        }
        out
    }

    fn responses_complete(&mut self) -> Vec<String> {
        if self.finished {
            return Vec::new();
        }
        let mut out = self.responses_ensure_started();
        let mut outputs: Vec<(usize, Value)> = Vec::new();
        if self.resp_content_open {
            let item_id = self.resp_text_item.clone().unwrap_or_default();
            let output_index = self.resp_text_output.unwrap_or(0);
            let part = json!({
                "type": "output_text",
                "text": self.resp_text,
                "annotations": []
            });
            out.push(json_frame(
                Some("response.output_text.done"),
                &json!({
                    "type": "response.output_text.done",
                    "item_id": item_id,
                    "output_index": output_index,
                    "content_index": 0,
                    "text": self.resp_text,
                }),
            ));
            out.push(json_frame(
                Some("response.content_part.done"),
                &json!({
                    "type": "response.content_part.done",
                    "item_id": item_id,
                    "output_index": output_index,
                    "content_index": 0,
                    "part": part,
                }),
            ));
            let item = json!({
                "id": item_id,
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": self.resp_text,
                    "annotations": []
                }]
            });
            out.push(json_frame(
                Some("response.output_item.done"),
                &json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": item,
                }),
            ));
            outputs.push((output_index, item));
            self.resp_content_open = false;
        }
        let mut tools: Vec<&RespToolState> = self.resp_tools.values().collect();
        tools.sort_by_key(|state| state.output_index);
        for state in tools {
            out.push(json_frame(
                Some("response.function_call_arguments.done"),
                &json!({
                    "type": "response.function_call_arguments.done",
                    "item_id": state.item_id,
                    "output_index": state.output_index,
                    "arguments": state.arguments,
                }),
            ));
            let item = json!({
                "id": state.item_id,
                "type": "function_call",
                "status": "completed",
                "call_id": state.call_id,
                "name": state.name,
                "arguments": state.arguments,
            });
            out.push(json_frame(
                Some("response.output_item.done"),
                &json!({
                    "type": "response.output_item.done",
                    "output_index": state.output_index,
                    "item": item,
                }),
            ));
            outputs.push((state.output_index, item));
        }
        outputs.sort_by_key(|(index, _)| *index);
        let output: Vec<Value> = outputs.into_iter().map(|(_, item)| item).collect();
        let incomplete = self.responses_stop_reason.as_deref() == Some("length");
        let mut response = json!({
            "id": self.id,
            "object": "response",
            "created_at": self.created,
            "status": if incomplete { "incomplete" } else { "completed" },
            "model": self.client_model,
            "output": output,
            "output_text": self.resp_text,
            "usage": {
                "input_tokens": self.usage.input_tokens,
                "input_tokens_details": {"cached_tokens": 0},
                "output_tokens": self.usage.output_tokens,
                "output_tokens_details": {"reasoning_tokens": 0},
                "total_tokens": self.usage.total_tokens.max(self.usage.input_tokens + self.usage.output_tokens),
            },
            "error": Value::Null,
        });
        if incomplete {
            response["incomplete_details"] = json!({"reason": "max_output_tokens"});
        }
        out.push(json_frame(
            Some("response.completed"),
            &json!({"type": "response.completed", "response": response}),
        ));
        self.finished = true;
        out
    }

    fn pointer_reason<'a>(&self, data: &'a Value) -> Option<&'a str> {
        data.pointer("/response/incomplete_details/reason")
            .or_else(|| data.pointer("/incomplete_details/reason"))
            .and_then(Value::as_str)
    }

    // -- chat chunk builders ------------------------------------------------

    fn chat_chunk(&self, delta: Value, finish: Option<&str>, include_usage: bool) -> String {
        let mut value = json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.client_model,
            "choices": [{
                "index": 0,
                "delta": delta,
                "finish_reason": finish,
            }],
        });
        if include_usage {
            value["usage"] = self.usage.openai_json();
        }
        json_frame(None, &value)
    }

    fn chat_role_chunk(&mut self) -> String {
        if self.chat_role_sent {
            return String::new();
        }
        self.chat_role_sent = true;
        self.chat_chunk(json!({"role": "assistant"}), None, false)
    }

    fn chat_content_chunk(&self, text: &str) -> String {
        self.chat_chunk(json!({"content": text}), None, false)
    }

    fn chat_tool_start_chunk(&self, index: u64, id: &str, name: &str) -> String {
        self.chat_chunk(
            json!({
                "tool_calls": [{
                    "index": index,
                    "id": id,
                    "type": "function",
                    "function": {"name": name, "arguments": ""}
                }]
            }),
            None,
            false,
        )
    }

    fn chat_tool_args_chunk(&self, index: u64, args: &str) -> String {
        self.chat_chunk(
            json!({
                "tool_calls": [{
                    "index": index,
                    "function": {"arguments": args}
                }]
            }),
            None,
            false,
        )
    }

    fn chat_final_chunk(&mut self, finish: &str) -> String {
        self.chat_chunk(json!({}), Some(finish), true)
    }
}

fn map_stop_anthropic_to_chat(reason: &str) -> &'static str {
    match reason {
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        "refusal" => "content_filter",
        _ => "stop",
    }
}

fn map_stop_chat_to_anthropic(reason: &str) -> &'static str {
    match reason {
        "length" => "max_tokens",
        "tool_calls" | "function_call" => "tool_use",
        "content_filter" => "refusal",
        _ => "end_turn",
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

pub fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn short_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..24].to_string()
}

fn opt_text(content: Option<&Value>) -> Option<String> {
    match content? {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let text: String = parts
                .iter()
                .filter_map(|p| {
                    if p.get("type").and_then(Value::as_str) == Some("text") {
                        p.get("text").and_then(Value::as_str)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("");
            Some(text)
        }
        _ => None,
    }
}

fn flatten_text(content: Option<&Value>) -> String {
    match content {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| {
                p.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| p.get("content").and_then(Value::as_str))
            })
            .collect::<Vec<_>>()
            .join(""),
        Some(other) => other.to_string(),
    }
}

fn json_frame(event: Option<&str>, value: &Value) -> String {
    let mut out = String::new();
    if let Some(event) = event {
        out.push_str("event: ");
        out.push_str(event);
        out.push('\n');
    }
    out.push_str("data: ");
    out.push_str(&value.to_string());
    out.push_str("\n\n");
    out
}

fn anthropic_event_frame(event: &str, value: &Value) -> String {
    json_frame(Some(event), value)
}

fn raw_frame(event: Option<&str>, data: &str) -> String {
    let mut out = String::new();
    if let Some(event) = event {
        out.push_str("event: ");
        out.push_str(event);
        out.push('\n');
    }
    out.push_str("data: ");
    out.push_str(data);
    out.push_str("\n\n");
    out
}

fn done_frame() -> String {
    "data: [DONE]\n\n".to_string()
}

/// Parse a frame produced by `json_frame` / `anthropic_event_frame`.
fn parse_sse_frame(frame: &str) -> Option<SseEvent> {
    let mut event = None;
    let mut data = String::new();
    for line in frame.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest);
        }
    }
    if data.is_empty() {
        None
    } else {
        Some(SseEvent { event, data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_openai_cache_hits() {
        let value = json!({
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 50,
                "total_tokens": 1050,
                "prompt_tokens_details": {"cached_tokens": 800}
            }
        });
        let usage = extract_nonstream_usage(Protocol::OpenAiChat, &value);
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.cached_tokens, 800);
        assert_eq!(usage.total(), 1050);
    }

    #[test]
    fn extracts_deepseek_style_cache_hits() {
        let value = json!({
            "usage": {
                "prompt_tokens": 1000,
                "prompt_cache_hit_tokens": 700,
                "prompt_cache_miss_tokens": 300,
                "completion_tokens": 10,
                "total_tokens": 1010
            }
        });
        let usage = extract_nonstream_usage(Protocol::OpenAiChat, &value);
        assert_eq!(usage.cached_tokens, 700);
        assert_eq!(usage.input_tokens, 1000);
    }

    #[test]
    fn anthropic_cache_reads_count_as_prompt_tokens() {
        // Anthropic reports cache reads outside `input_tokens`; both are billed
        // prompt tokens, so they must be summed for accurate costing.
        let value = json!({
            "usage": {
                "input_tokens": 120,
                "cache_read_input_tokens": 880,
                "cache_creation_input_tokens": 0,
                "output_tokens": 40
            }
        });
        let usage = extract_nonstream_usage(Protocol::AnthropicMessages, &value);
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.cached_tokens, 880);
        assert_eq!(usage.output_tokens, 40);
        assert_eq!(usage.total(), 1040);
    }

    #[test]
    fn responses_usage_reads_cached_input_details() {
        let value = json!({
            "usage": {
                "input_tokens": 500,
                "output_tokens": 25,
                "total_tokens": 525,
                "input_tokens_details": {"cached_tokens": 400}
            }
        });
        let usage = extract_nonstream_usage(Protocol::OpenAiResponses, &value);
        assert_eq!(usage.cached_tokens, 400);
        assert_eq!(usage.total(), 525);
    }

    #[test]
    fn every_ingress_upstream_pair_converts() {
        let protocols = [
            Protocol::OpenAiChat,
            Protocol::AnthropicMessages,
            Protocol::OpenAiResponses,
        ];
        let ingress_body = |protocol: Protocol| match protocol {
            Protocol::OpenAiChat => json!({
                "model": "client",
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 16
            }),
            Protocol::AnthropicMessages => json!({
                "model": "client",
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 16
            }),
            Protocol::OpenAiResponses => json!({
                "model": "client",
                "input": "hi",
                "max_output_tokens": 16
            }),
        };
        let ingress = |protocol: Protocol| match protocol {
            Protocol::OpenAiChat => Ingress::Chat,
            Protocol::AnthropicMessages => Ingress::Anthropic,
            Protocol::OpenAiResponses => Ingress::Responses,
        };
        let upstream_response = |protocol: Protocol| match protocol {
            Protocol::OpenAiChat => json!({
                "id": "chatcmpl-x",
                "object": "chat.completion",
                "model": "up",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "hi"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
            }),
            Protocol::AnthropicMessages => json!({
                "id": "msg_x",
                "type": "message",
                "role": "assistant",
                "model": "up",
                "content": [{"type": "text", "text": "hi"}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 1, "output_tokens": 1}
            }),
            Protocol::OpenAiResponses => json!({
                "id": "resp_x",
                "object": "response",
                "model": "up",
                "status": "completed",
                "output": [{
                    "type": "message",
                    "id": "msg_x",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "hi"}]
                }],
                "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}
            }),
        };

        for client in protocols {
            for upstream in protocols {
                let body = ingress_body(client);
                let converted = build_upstream_body(ingress(client), upstream, &body, "up")
                    .unwrap_or_else(|err| {
                        panic!(
                            "request conversion {} -> {} failed: {err}",
                            client.label(),
                            upstream.label()
                        )
                    });
                assert_eq!(converted["model"], "up");

                let response = upstream_response(upstream);
                convert_nonstream_response(ingress(client), upstream, response, "client")
                    .unwrap_or_else(|err| {
                        panic!(
                            "response conversion {} -> {} failed: {err}",
                            upstream.label(),
                            client.label()
                        )
                    });
            }
        }
    }

    #[test]
    fn chat_stream_from_responses_events() {
        let mut translator =
            StreamTranslator::new(Protocol::OpenAiChat, Protocol::OpenAiResponses, "client");
        let start = SseEvent {
            event: Some("response.created".into()),
            data:
                json!({"type":"response.created","response":{"id":"resp_1","status":"in_progress"}})
                    .to_string(),
        };
        translator.translate(&start);
        let delta = SseEvent {
            event: Some("response.output_text.delta".into()),
            data: json!({
                "type": "response.output_text.delta",
                "delta": "hello"
            })
            .to_string(),
        };
        let frames = translator.translate(&delta);
        assert!(frames.iter().any(|frame| frame.contains("hello")));
    }

    #[test]
    fn chat_to_anthropic_keeps_system_and_tools() {
        let body = json!({
            "model": "claude-sonnet",
            "messages": [
                {"role": "system", "content": "be brief"},
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "{\"q\":\"x\"}"}}
                ]},
                {"role": "tool", "tool_call_id": "call_1", "content": "result"}
            ],
            "tools": [{"type": "function", "function": {"name": "lookup", "parameters": {"type": "object"}}}],
            "max_tokens": 100
        });
        let out = chat_to_anthropic(&body, "claude-sonnet-5").unwrap();
        assert_eq!(out["model"], "claude-sonnet-5");
        assert_eq!(out["system"], "be brief");
        assert_eq!(out["max_tokens"], 100);
        assert_eq!(out["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(out["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(out["tools"][0]["name"], "lookup");
    }

    #[test]
    fn anthropic_to_chat_extracts_system_and_tool_results() {
        let body = json!({
            "model": "claude",
            "system": "sys",
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "hello"}]},
                {"role": "assistant", "content": [
                    {"type": "text", "text": "checking"},
                    {"type": "tool_use", "id": "t1", "name": "lookup", "input": {"q": 1}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "ok"},
                    {"type": "text", "text": "thanks"}
                ]}
            ],
            "max_tokens": 64
        });
        let out = anthropic_to_chat(&body, "gpt-4o").unwrap();
        assert_eq!(out["messages"][0]["role"], "system");
        assert_eq!(out["messages"][2]["tool_calls"][0]["id"], "t1");
        assert_eq!(out["messages"][3]["role"], "tool");
        assert_eq!(out["messages"][4]["content"], "thanks");
    }

    #[test]
    fn sse_decoder_handles_partial_and_multiline() {
        let mut d = SseDecoder::default();
        let mut events = d.push(b"event: message_start\ndata: {\"a\":");
        assert!(events.is_empty());
        events.extend(d.push(b"1}\n\n"));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.as_deref(), Some("message_start"));
        assert_eq!(events[0].data, "{\"a\":1}");
        events = d.push(b"data: one\ndata: two\n\n");
        assert_eq!(events[0].data, "one\ntwo");
    }

    #[test]
    fn chat_stream_from_anthropic_events() {
        let mut t = StreamTranslator::new(
            Protocol::OpenAiChat,
            Protocol::AnthropicMessages,
            "claude-sonnet",
        );
        let start = SseEvent {
            event: Some("message_start".into()),
            data: json!({"type":"message_start","message":{"id":"m1","usage":{"input_tokens":9}}})
                .to_string(),
        };
        let out = t.translate(&start);
        assert!(out[0].contains("\"role\":\"assistant\""));
        let delta = SseEvent {
            event: Some("content_block_delta".into()),
            data: json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}).to_string(),
        };
        let out = t.translate(&delta);
        assert!(out.iter().any(|f| f.contains("\"content\":\"hi\"")));
        let stop = SseEvent {
            event: Some("message_delta".into()),
            data: json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}).to_string(),
        };
        t.translate(&stop);
        let out = t.translate(&SseEvent {
            event: Some("message_stop".into()),
            data: "{\"type\":\"message_stop\"}".into(),
        });
        assert!(out.last().unwrap().contains("[DONE]"));
        assert_eq!(t.usage().input_tokens, 9);
        assert_eq!(t.usage().output_tokens, 3);
    }

    #[test]
    fn anthropic_stream_from_chat_events() {
        let mut t = StreamTranslator::new(
            Protocol::AnthropicMessages,
            Protocol::OpenAiChat,
            "claude-sonnet",
        );
        let chunk = SseEvent {
            event: None,
            data: json!({"choices":[{"delta":{"content":"hel"}}],"usage":null}).to_string(),
        };
        let out = t.translate(&chunk);
        assert!(out.iter().any(|f| f.contains("message_start")));
        assert!(out.iter().any(|f| f.contains("text_delta")));
        let done = SseEvent {
            event: None,
            data: "[DONE]".into(),
        };
        let out = t.translate(&done);
        assert!(out.iter().any(|f| f.contains("message_stop")));
    }
    #[test]
    fn responses_request_converts_to_chat_and_anthropic() {
        let body = json!({
            "model": "client-model",
            "instructions": "be brief",
            "input": [
                {"role": "user", "content": [{"type": "input_text", "text": "hi"}]},
                {"type": "function_call", "call_id": "c1", "name": "lookup", "arguments": "{\"q\":1}"},
                {"type": "function_call_output", "call_id": "c1", "output": "ok"}
            ],
            "max_output_tokens": 128
        });
        let chat = responses_to_chat(&body, "gpt-4o").unwrap();
        assert_eq!(chat["messages"][0]["role"], "system");
        assert_eq!(chat["messages"][1]["content"], "hi");
        assert_eq!(chat["messages"][2]["tool_calls"][0]["id"], "c1");
        assert_eq!(chat["messages"][3]["role"], "tool");
        assert_eq!(chat["max_tokens"], 128);
        let anthropic = build_upstream_body(
            Ingress::Responses,
            Protocol::AnthropicMessages,
            &body,
            "claude-sonnet",
        )
        .unwrap();
        assert_eq!(anthropic["model"], "claude-sonnet");
        assert_eq!(anthropic["system"], "be brief");
        assert_eq!(anthropic["messages"][1]["content"][0]["type"], "tool_use");
    }

    #[test]
    fn chat_response_converts_to_responses() {
        let chat = json!({
            "id": "chatcmpl-1",
            "created": 1_700_000_000,
            "model": "gpt-4o",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "hello",
                    "tool_calls": [{
                        "id": "c1",
                        "type": "function",
                        "function": {"name": "lookup", "arguments": "{\"q\":1}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 4, "completion_tokens": 2, "total_tokens": 6}
        });
        let response = chat_response_to_responses(&chat, "client-model");
        assert_eq!(response["object"], "response");
        assert_eq!(response["status"], "completed");
        assert_eq!(response["output"][0]["content"][0]["text"], "hello");
        assert_eq!(response["output"][1]["type"], "function_call");
        assert_eq!(response["usage"]["input_tokens"], 4);
        assert_eq!(response["model"], "client-model");
    }

    #[test]
    fn responses_stream_from_chat_events() {
        let mut t =
            StreamTranslator::new(Protocol::OpenAiResponses, Protocol::OpenAiChat, "gpt-4o");
        let created = t.translate(&SseEvent {
            event: None,
            data: json!({"choices":[{"delta":{"role":"assistant"},"finish_reason":null}]})
                .to_string(),
        });
        assert!(created
            .iter()
            .any(|frame| frame.contains("response.created")));
        let delta = t.translate(&SseEvent {
            event: None,
            data: json!({"choices":[{"delta":{"content":"hi"},"finish_reason":null}]}).to_string(),
        });
        assert!(delta
            .iter()
            .any(|frame| frame.contains("response.output_text.delta")));
        let done = t.translate(&SseEvent {
            event: None,
            data: "[DONE]".into(),
        });
        assert!(done
            .iter()
            .any(|frame| frame.contains("response.output_text.done")));
        assert!(done
            .iter()
            .any(|frame| frame.contains("response.completed")));
        assert!(done.iter().any(|frame| frame.contains("\"text\":\"hi\"")));
    }

    #[test]
    fn anthropic_stream_from_responses_events() {
        let mut t = StreamTranslator::new(
            Protocol::AnthropicMessages,
            Protocol::OpenAiResponses,
            "claude-sonnet",
        );
        let mut out = t.translate(&SseEvent {
            event: Some("response.created".into()),
            data: json!({"type":"response.created","response":{"id":"resp_1","model":"gpt"}})
                .to_string(),
        });
        out.extend(t.translate(&SseEvent {
            event: Some("response.output_text.delta".into()),
            data: json!({"type":"response.output_text.delta","delta":"hi"}).to_string(),
        }));
        out.extend(
            t.translate(&SseEvent {
                event: Some("response.completed".into()),
                data: json!({
                    "type":"response.completed",
                    "response":{"usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}}
                })
                .to_string(),
            }),
        );
        assert!(out.iter().any(|frame| frame.contains("message_start")));
        assert!(out.iter().any(|frame| frame.contains("text_delta")));
        assert!(out.iter().any(|frame| frame.contains("message_stop")));
    }

    #[test]
    fn responses_stream_from_anthropic_events() {
        let mut t = StreamTranslator::new(
            Protocol::OpenAiResponses,
            Protocol::AnthropicMessages,
            "gpt-4o",
        );
        let mut out = t.translate(&SseEvent {
            event: Some("message_start".into()),
            data: json!({"type":"message_start","message":{"id":"m1","usage":{"input_tokens":2}}})
                .to_string(),
        });
        out.extend(t.translate(&SseEvent {
            event: Some("content_block_delta".into()),
            data: json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}})
                .to_string(),
        }));
        out.extend(t.translate(&SseEvent {
            event: Some("message_stop".into()),
            data: "{\"type\":\"message_stop\"}".into(),
        }));
        assert!(out.iter().any(|frame| frame.contains("response.created")));
        assert!(out
            .iter()
            .any(|frame| frame.contains("response.output_text.delta")));
        assert!(out.iter().any(|frame| frame.contains("response.completed")));
    }
}
