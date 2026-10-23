use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{Stream, StreamExt};
use serde_json::{json, Value};
use tower_http::cors::{Any, CorsLayer};

use crate::config::{
    now_ts, AppConfig, AuthConfig, AuthMode, LocalKey, ModelRoute, Protocol, Provider,
};
use crate::db::{Db, UsageRecord};
use crate::plugins::DynamicEntries;
use crate::plugins::PluginHost;
use crate::protocol::{
    self, build_upstream_body, convert_nonstream_response, extract_nonstream_usage, is_stream,
    Ingress, SseDecoder, StreamTranslator, Usage,
};
use crate::scheduler::{AccountSlot, AccountState, FailureKind, Scheduler};

// ---------------------------------------------------------------------------
// Concurrency gate (queue with bounded capacity)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateError {
    QueueFull,
    TimedOut,
}

pub struct ConcurrencyGate {
    limit: AtomicUsize,
    active: AtomicUsize,
    waiters: AtomicUsize,
}

pub struct GatePermit {
    gate: Arc<ConcurrencyGate>,
}

impl Drop for GatePermit {
    fn drop(&mut self) {
        self.gate.active.fetch_sub(1, Ordering::SeqCst);
    }
}

struct WaiterGuard {
    gate: Arc<ConcurrencyGate>,
    armed: bool,
}

impl WaiterGuard {
    fn disarm(mut self) {
        if self.armed {
            self.gate.waiters.fetch_sub(1, Ordering::SeqCst);
            self.armed = false;
        }
    }
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        if self.armed {
            self.gate.waiters.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl ConcurrencyGate {
    fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit: AtomicUsize::new(limit.max(1)),
            active: AtomicUsize::new(0),
            waiters: AtomicUsize::new(0),
        })
    }

    pub fn set_limit(&self, limit: usize) {
        self.limit.store(limit.max(1), Ordering::SeqCst);
    }

    pub fn active(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }

    pub fn waiters(&self) -> usize {
        self.waiters.load(Ordering::SeqCst)
    }

    pub async fn acquire(
        self: &Arc<Self>,
        queue_capacity: usize,
        timeout_secs: u64,
    ) -> Result<GatePermit, GateError> {
        let deadline = Instant::now() + Duration::from_secs(timeout_secs.max(1));
        let mut waiter: Option<WaiterGuard> = None;
        loop {
            let limit = self.limit.load(Ordering::SeqCst).max(1);
            let active = self.active.load(Ordering::SeqCst);
            if active < limit
                && self
                    .active
                    .compare_exchange(active, active + 1, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                if let Some(guard) = waiter.take() {
                    guard.disarm();
                }
                return Ok(GatePermit { gate: self.clone() });
            }
            if waiter.is_none() {
                if queue_capacity == 0 {
                    return Err(GateError::QueueFull);
                }
                let n = self.waiters.fetch_add(1, Ordering::SeqCst) + 1;
                if n > queue_capacity {
                    self.waiters.fetch_sub(1, Ordering::SeqCst);
                    return Err(GateError::QueueFull);
                }
                waiter = Some(WaiterGuard {
                    gate: self.clone(),
                    armed: true,
                });
            }
            if Instant::now() >= deadline {
                return Err(GateError::TimedOut);
            }
            tokio::time::sleep(Duration::from_millis(8)).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Live throughput meter
// ---------------------------------------------------------------------------

/// Rolling window of completion tokens, sampled as streams produce output.
/// The taskbar badge and overview read this to show tokens per second.
pub struct SpeedMeter {
    samples: Mutex<VecDeque<(Instant, u64)>>,
}

impl SpeedMeter {
    fn new() -> Self {
        Self {
            samples: Mutex::new(VecDeque::new()),
        }
    }

    /// Record `tokens` newly generated completion tokens.
    pub fn record(&self, tokens: u64) {
        if tokens == 0 {
            return;
        }
        let now = Instant::now();
        let mut samples = self.samples.lock().unwrap_or_else(|e| e.into_inner());
        samples.push_back((now, tokens));
        while let Some((at, _)) = samples.front() {
            if now.duration_since(*at) > Duration::from_secs(8) {
                samples.pop_front();
            } else {
                break;
            }
        }
    }

    /// Spread a completed non-streaming response across the time it took, so a
    /// single large reply does not register as an artificial burst.
    pub fn record_span(&self, tokens: u64, elapsed: Duration) {
        if tokens == 0 {
            return;
        }
        let now = Instant::now();
        let span = elapsed.min(Duration::from_secs(8));
        if span < Duration::from_millis(200) {
            self.insert_samples(vec![(now, tokens)]);
            return;
        }
        let steps = 8u64.min(tokens.max(1));
        let per = tokens / steps;
        let remainder = tokens % steps;
        let mut samples = Vec::with_capacity(steps as usize);
        for i in 0..steps {
            let back = span.mul_f64((steps - i) as f64 / steps as f64);
            let portion = per + u64::from(i + 1 == steps) * remainder;
            samples.push((now - back, portion));
        }
        self.insert_samples(samples);
    }

    fn insert_samples(&self, samples: Vec<(Instant, u64)>) {
        let now = Instant::now();
        let mut guard = self.samples.lock().unwrap_or_else(|e| e.into_inner());
        guard.extend(samples);
        guard.make_contiguous().sort_by_key(|(at, _)| *at);
        while let Some((at, _)) = guard.front() {
            if now.duration_since(*at) > Duration::from_secs(8) {
                guard.pop_front();
            } else {
                break;
            }
        }
    }

    /// Completion tokens per second over the last three seconds. Returns 0.0
    /// after two seconds of silence so the badge drops back to zero instead of
    /// showing a stale average.
    pub fn tokens_per_sec(&self) -> f64 {
        const WINDOW: f64 = 3.0;
        let now = Instant::now();
        let samples = self.samples.lock().unwrap_or_else(|e| e.into_inner());
        let Some((last_at, _)) = samples.back() else {
            return 0.0;
        };
        if now.duration_since(*last_at) > Duration::from_secs(2) {
            return 0.0;
        }
        let total: u64 = samples
            .iter()
            .filter(|(at, _)| now.duration_since(*at) <= Duration::from_secs_f64(WINDOW))
            .map(|(_, tokens)| *tokens)
            .sum();
        total as f64 / WINDOW
    }
}

// ---------------------------------------------------------------------------
// Gateway
// ---------------------------------------------------------------------------

pub struct Gateway {
    /// Merged config (user config + runtime plugin providers). This is what
    /// every scheduler, UI snapshot and request path reads.
    config: RwLock<Arc<AppConfig>>,
    /// User configuration without dynamic plugin entries, used for saving.
    base_config: RwLock<Arc<AppConfig>>,
    /// Temporary providers/accounts/routes contributed by running plugins.
    dynamic: RwLock<crate::plugins::DynamicEntries>,
    pub(crate) master: Arc<crate::config::secret::MasterKey>,
    pub db: Arc<Db>,
    pub scheduler: Arc<Scheduler>,
    pub plugins: Arc<PluginHost>,
    pub(crate) http: reqwest::Client,
    pub(crate) http_local: reqwest::Client,
    gate: Arc<ConcurrencyGate>,
    key_quota_cache: Mutex<HashMap<String, KeyQuotaCache>>,
    pub speed: Arc<SpeedMeter>,
    pub started_at: i64,
}

#[derive(Default)]
struct KeyQuotaCache {
    checked_at: i64,
    tokens: u64,
    requests: u64,
}

impl Gateway {
    pub fn new(
        config: Arc<AppConfig>,
        master: Arc<crate::config::secret::MasterKey>,
        db: Arc<Db>,
        scheduler: Arc<Scheduler>,
        plugins: Arc<PluginHost>,
    ) -> Arc<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .pool_max_idle_per_host(16)
            .tcp_nodelay(true)
            .user_agent(concat!("YoyoRelay/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let http_local = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .pool_max_idle_per_host(16)
            .tcp_nodelay(true)
            .no_proxy()
            .user_agent(concat!("YoyoRelay/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let gate = ConcurrencyGate::new(config.server.max_global_concurrency);
        Arc::new(Self {
            config: RwLock::new(config.clone()),
            base_config: RwLock::new(config),
            dynamic: RwLock::new(crate::plugins::DynamicEntries::default()),
            master,
            db,
            scheduler,
            plugins,
            http,
            http_local,
            gate,
            key_quota_cache: Mutex::new(HashMap::new()),
            speed: Arc::new(SpeedMeter::new()),
            started_at: now_ts(),
        })
    }

    pub fn config(&self) -> Arc<AppConfig> {
        match self.config.read() {
            Ok(g) => g.clone(),
            Err(e) => Arc::clone(&e.into_inner()),
        }
    }

    pub fn update_config(&self, config: Arc<AppConfig>) {
        self.gate.set_limit(config.server.max_global_concurrency);
        if let Ok(mut guard) = self.base_config.write() {
            *guard = config;
        }
        self.rebuild_config();
        self.key_quota_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// User configuration without runtime plugin entries.
    pub fn base_config(&self) -> Arc<AppConfig> {
        self.base_config
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|e| Arc::clone(&e.into_inner()))
    }

    /// Install the plugin-provided providers/accounts/routes.
    pub fn set_dynamic(&self, entries: DynamicEntries) {
        if let Ok(mut guard) = self.dynamic.write() {
            *guard = entries;
        }
        self.rebuild_config();
        self.key_quota_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    fn rebuild_config(&self) {
        let base = self.base_config();
        let dynamic = self
            .dynamic
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        let merged = if dynamic.providers.is_empty()
            && dynamic.accounts.is_empty()
            && dynamic.routes.is_empty()
        {
            base
        } else {
            let mut cfg = (*base).clone();
            cfg.providers.extend(dynamic.providers);
            cfg.accounts.extend(dynamic.accounts);
            cfg.routes.extend(dynamic.routes);
            Arc::new(cfg)
        };
        if let Ok(mut guard) = self.config.write() {
            *guard = merged;
        }
    }

    pub fn plugin_for_provider(&self, provider_id: &str) -> Option<String> {
        self.dynamic
            .read()
            .ok()
            .and_then(|dynamic| dynamic.provider_plugins.get(provider_id).cloned())
    }

    pub fn plugin_provider_streams(&self, provider_id: &str) -> bool {
        self.dynamic
            .read()
            .ok()
            .map(|dynamic| dynamic.streaming.contains(provider_id))
            .unwrap_or(true)
    }

    pub fn queue_depth(&self) -> (usize, usize) {
        (self.gate.active(), self.gate.waiters())
    }

    /// Loopback upstreams (local mock servers, LM Studio, Ollama, ...) must not
    /// be sent through the system HTTP proxy.
    pub(crate) fn client_for(&self, url: &str) -> &reqwest::Client {
        if is_loopback_url(url) {
            &self.http_local
        } else {
            &self.http
        }
    }

    /// A client that always bypasses the system HTTP proxy.
    pub(crate) fn direct_client(&self) -> &reqwest::Client {
        &self.http_local
    }

    pub fn endpoint(&self) -> String {
        let cfg = self.config();
        let host = if cfg.server.bind == "0.0.0.0" || cfg.server.bind == "::" {
            "127.0.0.1"
        } else {
            cfg.server.bind.as_str()
        };
        format!("http://{host}:{}", cfg.server.port)
    }

    /// API base clients should configure (OpenAI SDK `base_url`, Anthropic
    /// `base_url`, and Responses clients all use this prefix).
    pub fn api_base(&self) -> String {
        format!("{}/v1", self.endpoint())
    }

    pub fn models_url(&self) -> String {
        format!("{}/models", self.api_base())
    }

    pub fn router(self: &Arc<Self>) -> Router {
        let cfg = self.config();
        let mut router = Router::new()
            .route("/health", get(health_handler))
            .route("/v1/models", get(models_handler))
            .route("/v1/chat/completions", post(chat_handler))
            .route("/v1/messages", post(chat_handler))
            .route("/v1/responses", post(chat_handler))
            .fallback(generic_handler)
            .with_state(self.clone())
            .layer(DefaultBodyLimit::max(32 * 1024 * 1024));
        if cfg.server.cors {
            router = router.layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            );
        }
        router
    }

    // -- authentication -----------------------------------------------------

    fn authenticate(&self, headers: &HeaderMap) -> Result<AuthContext, Response> {
        let cfg = self.config();
        if !cfg.server.auth_enabled {
            return Ok(AuthContext { key: None });
        }
        let Some(token) = extract_token(headers) else {
            return Err(self.error_response(
                Ingress::Chat,
                StatusCode::UNAUTHORIZED,
                "missing_api_key",
                "缺少本地 Key。请在请求头中提供 Authorization: Bearer yr-... 或 x-api-key。",
            ));
        };
        for key in &cfg.local_keys {
            if !key.enabled {
                continue;
            }
            let Ok(plain) = self.master.decrypt(&key.key) else {
                continue;
            };
            if constant_time_eq(plain.as_bytes(), token.as_bytes()) {
                if self.local_key_quota_exceeded(key) {
                    return Err(self.error_response(
                        Ingress::Chat,
                        StatusCode::TOO_MANY_REQUESTS,
                        "local_key_quota_exceeded",
                        "该本地 Key 的额度已用尽。",
                    ));
                }
                return Ok(AuthContext {
                    key: Some(key.clone()),
                });
            }
        }
        Err(self.error_response(
            Ingress::Chat,
            StatusCode::UNAUTHORIZED,
            "invalid_api_key",
            "本地 Key 无效或已禁用。",
        ))
    }

    fn local_key_quota_exceeded(&self, key: &LocalKey) -> bool {
        if key.max_tokens.is_none() && key.max_requests.is_none() {
            return false;
        }
        let now = now_ts();
        let mut cache = self
            .key_quota_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let entry = cache.entry(key.id.clone()).or_default();
        if entry.checked_at == 0 || now - entry.checked_at > 5 {
            match self.db.sum_local_key_since(&key.id, 0) {
                Ok((tokens, requests)) => {
                    entry.tokens = tokens;
                    entry.requests = requests;
                    entry.checked_at = now;
                }
                Err(err) => {
                    tracing::warn!("local key quota query failed: {err}");
                    return false;
                }
            }
        }
        key.max_tokens.map(|l| entry.tokens >= l).unwrap_or(false)
            || key
                .max_requests
                .map(|l| entry.requests >= l)
                .unwrap_or(false)
    }

    fn note_local_key_usage(&self, key_id: &str, tokens: u64, requests: u64) {
        let mut cache = self
            .key_quota_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cache.get_mut(key_id) {
            entry.tokens = entry.tokens.saturating_add(tokens);
            entry.requests = entry.requests.saturating_add(requests);
        }
    }

    // -- usage recording ----------------------------------------------------

    fn record_usage(&self, rec: UsageRecord) {
        if let Err(err) = self.db.record_usage(&rec) {
            tracing::error!("failed to record usage: {err}");
        }
        self.scheduler
            .note_account_usage(&rec.account_id, rec.total_tokens, 1);
        if let Some(key) = rec.local_key_id.as_deref() {
            self.note_local_key_usage(key, rec.total_tokens, 1);
        }
    }

    fn error_response(
        &self,
        ingress: Ingress,
        status: StatusCode,
        code: &str,
        message: &str,
    ) -> Response {
        let body = match ingress {
            Ingress::Anthropic => json!({
                "type": "error",
                "error": {"type": code, "message": message}
            }),
            _ => json!({
                "error": {"message": message, "type": code, "code": code}
            }),
        };
        (status, Json(body)).into_response()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct AuthContext {
    key: Option<LocalKey>,
}

fn extract_token(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        let v = v.trim();
        if let Some(rest) = v
            .strip_prefix("Bearer ")
            .or_else(|| v.strip_prefix("bearer "))
        {
            if !rest.trim().is_empty() {
                return Some(rest.trim().to_string());
            }
        }
    }
    if let Some(v) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        if !v.trim().is_empty() {
            return Some(v.trim().to_string());
        }
    }
    None
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn upstream_path(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenAiChat => "/chat/completions",
        Protocol::AnthropicMessages => "/messages",
        Protocol::OpenAiResponses => "/responses",
    }
}

fn upstream_url(provider: &Provider, protocol: Protocol) -> String {
    provider.join_url(upstream_path(protocol))
}

pub(crate) fn upstream_url_public(provider: &Provider, protocol: Protocol) -> String {
    upstream_url(provider, protocol)
}

/// Is this URL a loopback/LAN-local endpoint that must bypass the proxy?
pub(crate) fn is_loopback_url(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .map(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .map(|ip| ip.is_loopback())
                    .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// Build one upstream request. Used twice when the proxy attempt fails and a
/// direct retry is enabled, so both attempts share identical headers/body.
fn build_upstream_request(
    client: &reqwest::Client,
    provider: &Provider,
    api_key: &str,
    url: &str,
    body: &Value,
    stream_requested: bool,
    upstream_protocol: Protocol,
) -> Result<reqwest::RequestBuilder> {
    let mut req = client
        .post(url)
        .header(header::CONTENT_TYPE, "application/json")
        .json(body);
    if stream_requested {
        req = req.header(header::ACCEPT, "text/event-stream");
    }
    req = apply_auth(req, &provider.auth, api_key)?;
    for h in &provider.extra_headers {
        if !h.name.trim().is_empty() {
            req = req.header(h.name.trim(), h.value.as_str());
        }
    }
    if upstream_protocol == Protocol::AnthropicMessages
        && !has_header(&provider.extra_headers, "anthropic-version")
    {
        req = req.header("anthropic-version", "2023-06-01");
    }
    Ok(req)
}

pub(crate) fn apply_auth(
    mut req: reqwest::RequestBuilder,
    auth: &AuthConfig,
    key: &str,
) -> Result<reqwest::RequestBuilder> {
    match auth.mode {
        AuthMode::Bearer => {
            if !key.is_empty() {
                req = req.bearer_auth(key);
            }
        }
        AuthMode::XApiKey => {
            if !key.is_empty() {
                req = req.header("x-api-key", key);
            }
        }
        AuthMode::Custom => {
            if !auth.header_name.trim().is_empty() && !key.is_empty() {
                let value = if auth.scheme.trim().is_empty() {
                    key.to_string()
                } else {
                    format!("{} {}", auth.scheme.trim(), key)
                };
                req = req.header(auth.header_name.trim(), value);
            }
        }
        AuthMode::None => {}
    }
    Ok(req)
}

fn has_header(headers: &[crate::config::HeaderPair], name: &str) -> bool {
    headers
        .iter()
        .any(|h| h.name.eq_ignore_ascii_case(name) && !h.value.trim().is_empty())
}

pub(crate) fn readable_reqwest_error(err: &reqwest::Error) -> String {
    let mut detail = err.to_string();
    let mut source: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(err);
    while let Some(cause) = source {
        detail.push_str(" · ");
        detail.push_str(&cause.to_string());
        source = cause.source();
    }
    if err.is_timeout() {
        format!("连接超时（{detail}）")
    } else if err.is_connect() {
        format!("无法连接上游：{detail}")
    } else if err.is_request() {
        format!("请求构造失败：{detail}")
    } else {
        format!("上游请求失败：{detail}")
    }
}

enum FirstChunkError {
    Slow,
    Closed,
    Network(String),
}

/// Wait for the first body chunk (or first SSE chunk) with a dedicated
/// deadline. Returns the chunk plus the response so the rest can be streamed
/// without buffering the whole body.
async fn read_first_chunk(
    mut resp: reqwest::Response,
    deadline_secs: u64,
) -> std::result::Result<(Bytes, reqwest::Response), FirstChunkError> {
    match tokio::time::timeout(Duration::from_secs(deadline_secs.max(1)), resp.chunk()).await {
        Err(_) => Err(FirstChunkError::Slow),
        Ok(Err(err)) => Err(FirstChunkError::Network(readable_reqwest_error(&err))),
        Ok(Ok(None)) => Err(FirstChunkError::Closed),
        Ok(Ok(Some(bytes))) => Ok((bytes, resp)),
    }
}

fn truncate_text(text: &str, max: usize) -> String {
    let mut out = String::new();
    for (i, ch) in text.chars().enumerate() {
        if i >= max {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

pub(crate) fn upstream_error_message(text: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        for pointer in [
            "/error/message",
            "/error/msg",
            "/message",
            "/detail",
            "/error",
        ] {
            if let Some(s) = value.pointer(pointer).and_then(Value::as_str) {
                return truncate_text(s, 400);
            }
        }
    }
    truncate_text(text.trim(), 400)
}

fn classify_429(text: &str) -> FailureKind {
    let lower = text.to_ascii_lowercase();
    if lower.contains("concurren")
        || lower.contains("parallel")
        || lower.contains("simultaneous")
        || lower.contains("too many requests in flight")
    {
        FailureKind::ConcurrencyLimit
    } else {
        FailureKind::RateLimited
    }
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

async fn health_handler(State(gw): State<Arc<Gateway>>) -> Response {
    let cfg = gw.config();
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": now_ts() - gw.started_at,
        "models": cfg.servable_models(),
    }))
    .into_response()
}

async fn models_handler(State(gw): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    let auth = match gw.authenticate(&headers) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let cfg = gw.config();
    let models: Vec<String> = cfg
        .servable_models()
        .into_iter()
        .filter(|m| auth.key.as_ref().map(|k| k.allows_model(m)).unwrap_or(true))
        .collect();
    let data: Vec<Value> = models
        .iter()
        .map(|model| model_object(&cfg, model, gw.started_at))
        .collect();
    Json(json!({
        "object": "list",
        "data": data,
        "has_more": false,
        "first_id": models.first(),
        "last_id": models.last(),
    }))
    .into_response()
}

/// OpenAI `models` entry plus the metadata gateways commonly expose
/// (Anthropic-compatible `type` / `created_at` and capability fields).
fn model_object(cfg: &AppConfig, model: &str, fallback_created: i64) -> Value {
    let routes = cfg.routes_for(model);
    let profile = cfg.model_profile(model);
    let created = profile
        .map(|p| p.created_at)
        .filter(|ts| *ts > 0)
        .or_else(|| {
            routes
                .iter()
                .map(|r| r.created_at)
                .filter(|ts| *ts > 0)
                .min()
        })
        .unwrap_or(fallback_created);
    let created_at = chrono::DateTime::from_timestamp(created, 0)
        .map(|d| d.to_rfc3339())
        .unwrap_or_default();
    let context = profile
        .and_then(|p| p.context_window)
        .or_else(|| routes.iter().filter_map(|r| r.max_context_tokens).max());
    let max_output = profile.and_then(|p| p.max_output_tokens);
    let mut input_modalities = profile
        .map(|p| p.input_modalities.clone())
        .unwrap_or_else(|| vec!["text".to_string()]);
    if input_modalities.is_empty() {
        input_modalities.push("text".to_string());
    }
    let route_vision = routes.iter().any(|r| {
        r.capabilities
            .iter()
            .any(|c| matches!(c.as_str(), "vision" | "image" | "images" | "multimodal"))
    });
    if route_vision && !input_modalities.iter().any(|m| m == "image") {
        input_modalities.push("image".to_string());
    }
    let output_modalities = profile
        .map(|p| p.output_modalities.clone())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| vec!["text".to_string()]);
    let reasoning_levels = profile
        .map(|p| p.reasoning_levels.clone())
        .unwrap_or_default();
    let route_tools = routes.iter().any(|r| {
        r.capabilities
            .iter()
            .any(|c| matches!(c.as_str(), "tools" | "tool" | "function_calling"))
    });
    let supports_tools = profile.map(|p| p.supports_tools).unwrap_or(true) || route_tools;
    let supports_streaming = profile.map(|p| p.supports_streaming).unwrap_or(true);
    let vision = input_modalities.iter().any(|m| m == "image");
    let display_name = profile
        .map(|p| p.display_name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| model.to_string());
    let description = profile.map(|p| p.description.clone()).unwrap_or_default();
    let owned_by = profile
        .map(|p| p.owned_by.clone())
        .filter(|owner| !owner.trim().is_empty())
        .unwrap_or_else(|| "yoyo-relay".to_string());
    json!({
        "id": model,
        "object": "model",
        "type": "model",
        "created": created,
        "created_at": created_at,
        "owned_by": owned_by,
        "display_name": display_name,
        "description": description,
        "context_window": context,
        "context_length": context,
        "max_output_tokens": max_output,
        "max_tokens": max_output,
        "input_modalities": input_modalities,
        "output_modalities": output_modalities,
        "architecture": {
            "input_modalities": input_modalities.clone(),
            "output_modalities": output_modalities.clone(),
        },
        "capabilities": {
            "chat": true,
            "streaming": supports_streaming,
            "tools": supports_tools,
            "vision": vision,
            "reasoning": !reasoning_levels.is_empty(),
        },
        "reasoning_levels": reasoning_levels.clone(),
        "supported_reasoning_levels": reasoning_levels,
    })
}

async fn chat_handler(
    State(gw): State<Arc<Gateway>>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    body: Bytes,
) -> Response {
    let ingress = Ingress::from_path(uri.path());
    let auth = match gw.authenticate(&headers) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let mut payload: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(err) => {
            return gw.error_response(
                ingress,
                StatusCode::BAD_REQUEST,
                "invalid_json",
                &format!("请求体不是有效 JSON：{err}"),
            )
        }
    };
    let Some(mut client_model) = protocol::extract_client_model(&payload) else {
        return gw.error_response(
            ingress,
            StatusCode::BAD_REQUEST,
            "missing_model",
            "请求体缺少 model 字段。",
        );
    };
    if !model_allowed(&auth, &client_model) {
        return gw.error_response(
            ingress,
            StatusCode::FORBIDDEN,
            "model_not_allowed",
            &format!("本地 Key 无权访问模型 {client_model}。"),
        );
    }

    // Plugins see the request before it reaches the scheduler and can modify
    // the payload or answer it directly.
    let stream_requested = is_stream(&payload);
    let plugin_meta = serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "model": client_model,
        "stream": stream_requested,
        "protocol": ingress.protocol().map(|p| p.label()).unwrap_or("generic"),
        "path": uri.path(),
        "localKey": auth.key.as_ref().map(|key| serde_json::json!({
            "id": key.id,
            "name": key.name,
        })),
        "payload": payload.clone(),
    });
    let plugin_outcome = gw.plugins.before_request(plugin_meta.clone()).await;
    if let Some(block) = plugin_outcome.block {
        let status = StatusCode::from_u16(block.status).unwrap_or(StatusCode::FORBIDDEN);
        return (status, Json(block.body)).into_response();
    }
    if let Some(modified) = plugin_outcome.payload {
        payload = modified;
        let Some(model) = protocol::extract_client_model(&payload) else {
            return gw.error_response(
                ingress,
                StatusCode::BAD_REQUEST,
                "plugin_removed_model",
                "插件修改后的请求体缺少 model 字段。",
            );
        };
        if !model_allowed(&auth, &model) {
            return gw.error_response(
                ingress,
                StatusCode::FORBIDDEN,
                "model_not_allowed",
                &format!("本地 Key 无权访问模型 {model}。"),
            );
        }
        client_model = model;
    }

    let cfg = gw.config();
    let now = now_ts();
    if !gw.scheduler.breaker.allow(&cfg.scheduler, now) {
        let retry = gw.scheduler.breaker.retry_after(now).max(1);
        let mut resp = gw.error_response(
            ingress,
            StatusCode::SERVICE_UNAVAILABLE,
            "relay_breaker_open",
            &format!("全局熔断已触发，将在 {retry} 秒后恢复。"),
        );
        resp.headers_mut().insert(
            "retry-after",
            retry
                .to_string()
                .parse()
                .unwrap_or_else(|_| "30".parse().unwrap()),
        );
        return resp;
    }

    let permit = match gw
        .gate
        .acquire(cfg.server.queue_capacity, cfg.server.queue_timeout_secs)
        .await
    {
        Ok(p) => p,
        Err(GateError::QueueFull) => {
            let mut resp = gw.error_response(
                ingress,
                StatusCode::TOO_MANY_REQUESTS,
                "queue_full",
                "本地代理排队已满，请稍后重试。",
            );
            resp.headers_mut()
                .insert("retry-after", "3".parse().unwrap());
            return resp;
        }
        Err(GateError::TimedOut) => {
            let mut resp = gw.error_response(
                ingress,
                StatusCode::TOO_MANY_REQUESTS,
                "queue_timeout",
                "等待代理并发槽位超时，请稍后重试。",
            );
            resp.headers_mut()
                .insert("retry-after", "3".parse().unwrap());
            return resp;
        }
    };

    let started = Instant::now();
    let request_id = uuid::Uuid::new_v4().to_string();
    let stream_requested = is_stream(&payload);
    let chain = gw.scheduler.build_chain(&cfg, &client_model, ingress, true);
    if chain.attempts.is_empty() {
        let notes = chain.notes.join("；");
        let message = if notes.is_empty() {
            format!("模型 {client_model} 没有可用的路由或账号。")
        } else {
            format!("模型 {client_model} 暂不可用：{notes}")
        };
        return gw.error_response(
            ingress,
            StatusCode::SERVICE_UNAVAILABLE,
            "no_available_route",
            &message,
        );
    }

    let total_attempts = chain.attempts.len();
    let mut errors: Vec<String> = chain.notes.clone();
    let mut attempts_made = 0usize;
    let mut last_failure: Option<(String, String)> = None;
    let mut last_status = StatusCode::BAD_GATEWAY;
    // Request-shape errors (400/404/413/422) are route-specific. Skip the rest
    // of that upstream model but keep walking the chain so the next configured
    // upstream still gets a chance.
    let mut skip_routes: std::collections::HashSet<String> = std::collections::HashSet::new();

    for attempt in &chain.attempts {
        if skip_routes.contains(&attempt.route.id) {
            continue;
        }
        attempts_made += 1;
        gw.plugins.notify(
            "onRouteSelected",
            serde_json::json!({
                "event": "onRouteSelected",
                "model": client_model,
                "account": attempt.account.name,
                "accountId": attempt.account.id,
                "provider": attempt.provider.name,
                "providerId": attempt.provider.id,
                "upstreamModel": attempt.route.upstream_model,
                "protocol": attempt.upstream_protocol.label(),
            }),
        );
        // The chain is built before the first request. If an earlier attempt
        // in this same request cooled the account, do not try it again for a
        // second route — move on to the next account/provider.
        let attempt_state = gw.scheduler.state_of(&attempt.account, now_ts());
        if !matches!(attempt_state, AccountState::Healthy) {
            errors.push(format!(
                "账号「{}」当前不可用（{}），已跳过",
                attempt.account.name,
                attempt_state.badge()
            ));
            continue;
        }
        let Some(slot) = gw
            .scheduler
            .try_acquire_slot(&attempt.account.id, attempt.account.max_concurrency)
        else {
            errors.push(format!("账号「{}」并发已满", attempt.account.name));
            continue;
        };
        if let Some(plugin_id) = gw.plugin_for_provider(&attempt.provider.id) {
            let upstream_body = match build_upstream_body(
                ingress,
                attempt.upstream_protocol,
                &payload,
                &attempt.route.upstream_model,
            ) {
                Ok(value) => value,
                Err(err) => {
                    errors.push(format!("插件供应商协议转换失败：{err}"));
                    continue;
                }
            };
            let supports_streaming = gw.plugin_provider_streams(&attempt.provider.id);
            let wants_stream = stream_requested && supports_streaming;
            let timeout_secs = attempt
                .provider
                .timeout_secs
                .unwrap_or(cfg.server.upstream_timeout_secs)
                .max(10);
            let event = serde_json::json!({
                "event": "onProviderRequest",
                "provider": {
                    "id": attempt.provider.id,
                    "name": attempt.provider.name,
                    "model": attempt.route.upstream_model,
                    "streaming": supports_streaming,
                },
                "request": {
                    "payload": upstream_body,
                    "model": attempt.route.upstream_model,
                    "clientModel": client_model,
                    "stream": stream_requested,
                    "protocol": attempt.upstream_protocol.label(),
                }
            });

            if wants_stream {
                match gw.plugins.call_provider_stream(&plugin_id, event).await {
                    Ok(chunks) => {
                        let upstream = async_stream::stream! {
                            let mut rx = chunks;
                            while let Some(item) = rx.recv().await {
                                yield item;
                            }
                        };
                        let translator = StreamTranslator::new(
                            ingress.protocol().unwrap_or(Protocol::OpenAiChat),
                            attempt.upstream_protocol,
                            &client_model,
                        );
                        let usage_cell = Arc::new(Mutex::new(Usage::default()));
                        let finalizer = StreamFinalizer {
                            db: gw.db.clone(),
                            scheduler: gw.scheduler.clone(),
                            account: attempt.account.clone(),
                            route: attempt.route.clone(),
                            provider: attempt.provider.clone(),
                            client_model: client_model.clone(),
                            local_key_id: auth.key.as_ref().map(|k| k.id.clone()),
                            request_id: request_id.clone(),
                            usage: usage_cell.clone(),
                            plugins: gw.plugins.clone(),
                            account_name: attempt.account.name.clone(),
                            provider_name: attempt.provider.name.clone(),
                            fx_units_per_usd: cfg
                                .ui
                                .fx_rate(attempt.route.effective_pricing().currency.code()),
                            _slot: Some(slot),
                            _permit: Some(permit),
                            started,
                            done: false,
                            completed: false,
                            failed: false,
                        };
                        let body = build_sse_body(
                            upstream,
                            translator,
                            finalizer,
                            usage_cell,
                            gw.speed.clone(),
                            Duration::from_secs(cfg.server.stream_idle_timeout_secs.max(5)),
                        );
                        let mut response = Response::new(body);
                        *response.status_mut() = StatusCode::OK;
                        let h = response.headers_mut();
                        h.insert(
                            header::CONTENT_TYPE,
                            "text/event-stream; charset=utf-8".parse().unwrap(),
                        );
                        h.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
                        h.insert(
                            "x-yoyo-attempts",
                            attempts_made.to_string().parse().unwrap(),
                        );
                        gw.scheduler.on_success(&attempt.account.id);
                        gw.scheduler.breaker.record_success();
                        return response;
                    }
                    Err(err) => {
                        errors.push(format!("插件供应商流式失败：{err}"));
                        last_failure =
                            Some((attempt.account.id.clone(), attempt.provider.id.clone()));
                        last_status = StatusCode::BAD_GATEWAY;
                        continue;
                    }
                }
            }

            match gw
                .plugins
                .call_provider(&plugin_id, event, Duration::from_secs(timeout_secs))
                .await
            {
                Ok((status, body)) => {
                    if !(200..300).contains(&status) {
                        last_status =
                            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
                        last_failure =
                            Some((attempt.account.id.clone(), attempt.provider.id.clone()));
                        errors.push(format!(
                            "插件供应商返回 HTTP {status}：{}",
                            upstream_error_message(&body.to_string())
                        ));
                        continue;
                    }
                    let usage = extract_nonstream_usage(attempt.upstream_protocol, &body);
                    let converted = convert_nonstream_response(
                        ingress,
                        attempt.upstream_protocol,
                        body,
                        &client_model,
                    );
                    let latency = started.elapsed().as_millis() as u64;
                    match converted {
                        Ok(mut value) => {
                            if let Some(model) = value.get_mut("model") {
                                *model = Value::String(client_model.clone());
                            }
                            gw.scheduler.on_success(&attempt.account.id);
                            gw.scheduler.breaker.record_success();
                            let cost_micros = attempt.route.cost_micros_with_fx(
                                usage.input_tokens,
                                usage.cached_tokens,
                                usage.output_tokens,
                                chrono::Local::now(),
                                cfg.ui
                                    .fx_rate(attempt.route.effective_pricing().currency.code()),
                            );
                            gw.record_usage(UsageRecord {
                                ts: now_ts(),
                                request_id: request_id.clone(),
                                account_id: attempt.account.id.clone(),
                                provider_id: attempt.provider.id.clone(),
                                local_key_id: auth.key.as_ref().map(|k| k.id.clone()),
                                client_model: client_model.clone(),
                                upstream_model: attempt.route.upstream_model.clone(),
                                protocol: attempt.upstream_protocol.label().to_string(),
                                status: 200,
                                ok: true,
                                stream: false,
                                cancelled: false,
                                prompt_tokens: usage.input_tokens,
                                cached_tokens: usage.cached_tokens,
                                completion_tokens: usage.output_tokens,
                                total_tokens: usage.total(),
                                cost_micros,
                                latency_ms: latency,
                                kind: "chat".into(),
                            });
                            gw.plugins.notify(
                                "onRequestCompleted",
                                serde_json::json!({
                                    "event": "onRequestCompleted",
                                    "request": {
                                        "id": request_id,
                                        "model": client_model,
                                        "upstreamModel": attempt.route.upstream_model,
                                        "account": attempt.account.name,
                                        "provider": attempt.provider.name,
                                        "protocol": attempt.upstream_protocol.label(),
                                        "status": 200,
                                        "ok": true,
                                        "stream": false,
                                    },
                                    "usage": {
                                        "promptTokens": usage.input_tokens,
                                        "cachedTokens": usage.cached_tokens,
                                        "completionTokens": usage.output_tokens,
                                        "totalTokens": usage.total(),
                                        "costMicros": cost_micros,
                                        "latencyMs": latency,
                                    }
                                }),
                            );
                            if let Some(modified) = gw
                                .plugins
                                .after_response(plugin_meta.clone(), value.clone())
                                .await
                            {
                                value = modified;
                            }
                            let mut response = Json(value).into_response();
                            let h = response.headers_mut();
                            h.insert(
                                "x-yoyo-attempts",
                                attempts_made.to_string().parse().unwrap(),
                            );
                            h.insert("x-yoyo-plugin-provider", plugin_id.parse().unwrap());
                            gw.speed.record_span(usage.output_tokens, started.elapsed());
                            return response;
                        }
                        Err(err) => {
                            errors.push(format!("插件供应商响应协议转换失败：{err}"));
                            last_status = StatusCode::BAD_GATEWAY;
                            continue;
                        }
                    }
                }
                Err(err) => {
                    errors.push(format!("插件供应商失败：{err}"));
                    last_failure = Some((attempt.account.id.clone(), attempt.provider.id.clone()));
                    last_status = StatusCode::BAD_GATEWAY;
                    continue;
                }
            }
        }
        let api_key = match gw.master.decrypt(&attempt.account.api_key) {
            Ok(k) => k,
            Err(err) => {
                errors.push(format!(
                    "账号「{}」的 Key 无法解密：{err}",
                    attempt.account.name
                ));
                gw.scheduler
                    .on_failure(&attempt.account, FailureKind::Auth, &cfg.scheduler);
                continue;
            }
        };
        let upstream_body = match build_upstream_body(
            ingress,
            attempt.upstream_protocol,
            &payload,
            &attempt.route.upstream_model,
        ) {
            Ok(v) => v,
            Err(err) => {
                errors.push(format!(
                    "路由「{}」协议转换失败：{err}",
                    attempt.route.client_model
                ));
                continue;
            }
        };
        let url = upstream_url(&attempt.provider, attempt.upstream_protocol);
        let timeout_secs = attempt
            .provider
            .timeout_secs
            .unwrap_or(cfg.server.upstream_timeout_secs)
            .max(5);
        let slow_secs = if cfg.scheduler.slow_switch {
            cfg.scheduler.slow_first_byte_secs.clamp(3, timeout_secs)
        } else {
            0
        };
        let request = match build_upstream_request(
            gw.client_for(&url),
            &attempt.provider,
            &api_key,
            &url,
            &upstream_body,
            stream_requested,
            attempt.upstream_protocol,
        ) {
            Ok(r) => r,
            Err(err) => {
                errors.push(format!(
                    "账号「{}」鉴权头配置无效：{err}",
                    attempt.account.name
                ));
                continue;
            }
        };
        let mut send_result =
            tokio::time::timeout(Duration::from_secs(timeout_secs), request.send()).await;
        let transport_failed = match &send_result {
            // Overall request timeout: do not double the wait with a direct
            // retry, the slow-switch path handles it.
            Err(_) => false,
            Ok(Err(err)) => err.is_connect() || err.is_request(),
            Ok(Ok(_)) => false,
        };
        // If the system proxy cannot reach the upstream, retry the same
        // account directly before burning a switch.
        if transport_failed && cfg.server.direct_fallback && !is_loopback_url(&url) {
            if let Ok(direct_req) = build_upstream_request(
                gw.direct_client(),
                &attempt.provider,
                &api_key,
                &url,
                &upstream_body,
                stream_requested,
                attempt.upstream_protocol,
            ) {
                errors.push(format!(
                    "账号「{}」代理连接失败，已尝试直连",
                    attempt.account.name
                ));
                send_result =
                    tokio::time::timeout(Duration::from_secs(timeout_secs), direct_req.send())
                        .await;
            }
        }

        let resp = match send_result {
            Err(_) => {
                errors.push(format!(
                    "账号「{}」连接上游超时（{timeout_secs}s）",
                    attempt.account.name
                ));
                gw.scheduler
                    .on_failure(&attempt.account, FailureKind::Timeout, &cfg.scheduler);
                last_failure = Some((attempt.account.id.clone(), attempt.provider.id.clone()));
                last_status = StatusCode::GATEWAY_TIMEOUT;
                continue;
            }
            Ok(Err(err)) => {
                errors.push(format!(
                    "账号「{}」{}",
                    attempt.account.name,
                    readable_reqwest_error(&err)
                ));
                gw.scheduler
                    .on_failure(&attempt.account, FailureKind::Network, &cfg.scheduler);
                last_failure = Some((attempt.account.id.clone(), attempt.provider.id.clone()));
                last_status = StatusCode::BAD_GATEWAY;
                continue;
            }
            Ok(Ok(r)) => r,
        };

        let status = resp.status();
        if status.is_success() {
            let content_type = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let is_sse = content_type.contains("text/event-stream");
            if stream_requested && is_sse {
                let first_deadline = if slow_secs > 0 {
                    slow_secs
                } else {
                    timeout_secs
                };
                let (first, resp) = match read_first_chunk(resp, first_deadline).await {
                    Ok(value) => value,
                    Err(FirstChunkError::Slow) => {
                        errors.push(format!(
                            "账号「{}」{}超过 {}s，自动切换",
                            attempt.account.name,
                            if slow_secs > 0 {
                                "首包"
                            } else {
                                "读取首包"
                            },
                            first_deadline
                        ));
                        gw.scheduler.on_failure(
                            &attempt.account,
                            if slow_secs > 0 {
                                FailureKind::Slow
                            } else {
                                FailureKind::Timeout
                            },
                            &cfg.scheduler,
                        );
                        last_failure =
                            Some((attempt.account.id.clone(), attempt.provider.id.clone()));
                        last_status = StatusCode::GATEWAY_TIMEOUT;
                        continue;
                    }
                    Err(FirstChunkError::Closed) => {
                        errors.push(format!(
                            "账号「{}」在返回首包前关闭了连接",
                            attempt.account.name
                        ));
                        gw.scheduler.on_failure(
                            &attempt.account,
                            FailureKind::Server,
                            &cfg.scheduler,
                        );
                        last_failure =
                            Some((attempt.account.id.clone(), attempt.provider.id.clone()));
                        last_status = StatusCode::BAD_GATEWAY;
                        continue;
                    }
                    Err(FirstChunkError::Network(err)) => {
                        errors.push(format!(
                            "账号「{}」读取首包失败：{err}",
                            attempt.account.name
                        ));
                        gw.scheduler.on_failure(
                            &attempt.account,
                            FailureKind::Network,
                            &cfg.scheduler,
                        );
                        last_failure =
                            Some((attempt.account.id.clone(), attempt.provider.id.clone()));
                        last_status = StatusCode::BAD_GATEWAY;
                        continue;
                    }
                };
                // Re-attach the already-read chunk so the translator sees the
                // complete stream without dropping the first SSE event.
                let upstream = async_stream::stream! {
                    yield Ok::<Bytes, String>(first);
                    let mut resp = resp;
                    loop {
                        match resp.chunk().await {
                            Ok(Some(chunk)) => yield Ok(chunk),
                            Ok(None) => break,
                            Err(err) => {
                                yield Err(err.to_string());
                                break;
                            }
                        }
                    }
                };
                let translator = StreamTranslator::new(
                    ingress.protocol().unwrap_or(Protocol::OpenAiChat),
                    attempt.upstream_protocol,
                    &client_model,
                );
                let usage_cell = Arc::new(Mutex::new(Usage::default()));
                let finalizer = StreamFinalizer {
                    db: gw.db.clone(),
                    scheduler: gw.scheduler.clone(),
                    account: attempt.account.clone(),
                    route: attempt.route.clone(),
                    provider: attempt.provider.clone(),
                    client_model: client_model.clone(),
                    local_key_id: auth.key.as_ref().map(|k| k.id.clone()),
                    request_id: request_id.clone(),
                    usage: usage_cell.clone(),
                    plugins: gw.plugins.clone(),
                    account_name: attempt.account.name.clone(),
                    provider_name: attempt.provider.name.clone(),
                    fx_units_per_usd: cfg
                        .ui
                        .fx_rate(attempt.route.effective_pricing().currency.code()),
                    _slot: Some(slot),
                    _permit: Some(permit),
                    started,
                    done: false,
                    completed: false,
                    failed: false,
                };
                let body = build_sse_body(
                    upstream,
                    translator,
                    finalizer,
                    usage_cell,
                    gw.speed.clone(),
                    Duration::from_secs(cfg.server.stream_idle_timeout_secs.max(5)),
                );
                let mut response = Response::new(body);
                *response.status_mut() = StatusCode::OK;
                let h = response.headers_mut();
                h.insert(
                    header::CONTENT_TYPE,
                    "text/event-stream; charset=utf-8".parse().unwrap(),
                );
                h.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
                h.insert(
                    "x-yoyo-attempts",
                    attempts_made.to_string().parse().unwrap(),
                );
                h.insert(
                    "x-yoyo-upstream-model",
                    attempt
                        .route
                        .upstream_model
                        .parse()
                        .unwrap_or_else(|_| "upstream".parse().unwrap()),
                );
                gw.scheduler.on_success(&attempt.account.id);
                gw.scheduler.breaker.record_success();
                return response;
            }

            // Non-streaming (or the upstream ignored `stream`).
            let first_deadline = if slow_secs > 0 {
                slow_secs
            } else {
                timeout_secs
            };
            let (first, resp) = match read_first_chunk(resp, first_deadline).await {
                Ok(value) => value,
                Err(FirstChunkError::Slow) => {
                    errors.push(format!(
                        "账号「{}」{}超过 {}s，自动切换",
                        attempt.account.name,
                        if slow_secs > 0 {
                            "首包"
                        } else {
                            "读取首包"
                        },
                        first_deadline
                    ));
                    gw.scheduler.on_failure(
                        &attempt.account,
                        if slow_secs > 0 {
                            FailureKind::Slow
                        } else {
                            FailureKind::Timeout
                        },
                        &cfg.scheduler,
                    );
                    last_failure = Some((attempt.account.id.clone(), attempt.provider.id.clone()));
                    last_status = StatusCode::GATEWAY_TIMEOUT;
                    continue;
                }
                Err(FirstChunkError::Closed) => {
                    errors.push(format!(
                        "账号「{}」返回了空响应（首包前关闭）",
                        attempt.account.name
                    ));
                    gw.scheduler
                        .on_failure(&attempt.account, FailureKind::Server, &cfg.scheduler);
                    last_failure = Some((attempt.account.id.clone(), attempt.provider.id.clone()));
                    last_status = StatusCode::BAD_GATEWAY;
                    continue;
                }
                Err(FirstChunkError::Network(err)) => {
                    errors.push(format!(
                        "账号「{}」读取首包失败：{err}",
                        attempt.account.name
                    ));
                    gw.scheduler
                        .on_failure(&attempt.account, FailureKind::Network, &cfg.scheduler);
                    last_failure = Some((attempt.account.id.clone(), attempt.provider.id.clone()));
                    last_status = StatusCode::BAD_GATEWAY;
                    continue;
                }
            };
            let read = tokio::time::timeout(Duration::from_secs(timeout_secs), resp.bytes()).await;
            let mut bytes = first.to_vec();
            match read {
                Err(_) => {
                    errors.push(format!("账号「{}」读取响应超时", attempt.account.name));
                    gw.scheduler
                        .on_failure(&attempt.account, FailureKind::Timeout, &cfg.scheduler);
                    last_status = StatusCode::GATEWAY_TIMEOUT;
                    continue;
                }
                Ok(Err(err)) => {
                    errors.push(format!(
                        "账号「{}」读取响应失败：{err}",
                        attempt.account.name
                    ));
                    gw.scheduler
                        .on_failure(&attempt.account, FailureKind::Network, &cfg.scheduler);
                    last_status = StatusCode::BAD_GATEWAY;
                    continue;
                }
                Ok(Ok(rest)) => bytes.extend_from_slice(&rest),
            }
            let upstream_json: Value = match serde_json::from_slice(&bytes) {
                Ok(v) => v,
                Err(err) => {
                    errors.push(format!(
                        "账号「{}」返回了无法解析的响应：{err}",
                        attempt.account.name
                    ));
                    last_status = StatusCode::BAD_GATEWAY;
                    continue;
                }
            };
            let usage = extract_nonstream_usage(attempt.upstream_protocol, &upstream_json);
            let converted = convert_nonstream_response(
                ingress,
                attempt.upstream_protocol,
                upstream_json,
                &client_model,
            );
            let latency = started.elapsed().as_millis() as u64;
            match converted {
                Ok(mut value) => {
                    if let Some(model) = value.get_mut("model") {
                        *model = Value::String(client_model.clone());
                    }
                    gw.scheduler.on_success(&attempt.account.id);
                    gw.scheduler.breaker.record_success();
                    let cost_micros = attempt.route.cost_micros_with_fx(
                        usage.input_tokens,
                        usage.cached_tokens,
                        usage.output_tokens,
                        chrono::Local::now(),
                        cfg.ui
                            .fx_rate(attempt.route.effective_pricing().currency.code()),
                    );
                    gw.record_usage(UsageRecord {
                        ts: now_ts(),
                        request_id: request_id.clone(),
                        account_id: attempt.account.id.clone(),
                        provider_id: attempt.provider.id.clone(),
                        local_key_id: auth.key.as_ref().map(|k| k.id.clone()),
                        client_model: client_model.clone(),
                        upstream_model: attempt.route.upstream_model.clone(),
                        protocol: attempt.upstream_protocol.label().to_string(),
                        status: 200,
                        ok: true,
                        stream: false,
                        cancelled: false,
                        prompt_tokens: usage.input_tokens,
                        cached_tokens: usage.cached_tokens,
                        completion_tokens: usage.output_tokens,
                        total_tokens: usage.total(),
                        cost_micros,
                        latency_ms: latency,
                        kind: "chat".into(),
                    });
                    gw.plugins.notify(
                        "onRequestCompleted",
                        serde_json::json!({
                            "event": "onRequestCompleted",
                            "request": {
                                "id": request_id,
                                "model": client_model,
                                "upstreamModel": attempt.route.upstream_model,
                                "account": attempt.account.name,
                                "provider": attempt.provider.name,
                                "protocol": attempt.upstream_protocol.label(),
                                "status": 200,
                                "ok": true,
                                "stream": false,
                            },
                            "usage": {
                                "promptTokens": usage.input_tokens,
                                "cachedTokens": usage.cached_tokens,
                                "completionTokens": usage.output_tokens,
                                "totalTokens": usage.total(),
                                "costMicros": cost_micros,
                                "latencyMs": latency,
                            }
                        }),
                    );
                    if let Some(modified) = gw
                        .plugins
                        .after_response(plugin_meta.clone(), value.clone())
                        .await
                    {
                        value = modified;
                    }
                    let mut response = Json(value).into_response();
                    let h = response.headers_mut();
                    h.insert(
                        "x-yoyo-attempts",
                        attempts_made.to_string().parse().unwrap(),
                    );
                    h.insert("x-yoyo-latency-ms", latency.to_string().parse().unwrap());
                    gw.speed.record_span(usage.output_tokens, started.elapsed());
                    return response;
                }
                Err(err) => {
                    gw.scheduler
                        .on_failure(&attempt.account, FailureKind::Server, &cfg.scheduler);
                    errors.push(format!(
                        "账号「{}」响应协议转换失败：{err}",
                        attempt.account.name
                    ));
                    last_status = StatusCode::BAD_GATEWAY;
                    continue;
                }
            }
        }

        // Upstream returned a non-2xx status.
        last_status = status;
        last_failure = Some((attempt.account.id.clone(), attempt.provider.id.clone()));
        let err_bytes = tokio::time::timeout(Duration::from_secs(10), resp.bytes())
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or_default();
        let err_text = String::from_utf8_lossy(&err_bytes).to_string();
        let summary = upstream_error_message(&err_text);
        match status.as_u16() {
            401 | 403 => {
                gw.scheduler
                    .on_failure(&attempt.account, FailureKind::Auth, &cfg.scheduler);
                notify_account_failure(&gw, attempt, status.as_u16(), "auth", &summary);
                errors.push(format!(
                    "账号「{}」鉴权失败（HTTP {}）：{}",
                    attempt.account.name, status, summary
                ));
            }
            429 => {
                let kind = classify_429(&err_text);
                gw.scheduler
                    .on_failure(&attempt.account, kind, &cfg.scheduler);
                notify_account_failure(&gw, attempt, 429, "rate_limited", &summary);
                errors.push(format!(
                    "账号「{}」被上游限流（HTTP 429）：{}",
                    attempt.account.name, summary
                ));
            }
            400 | 413 | 422 => {
                // A deterministic request-shape error. Do not burn the other
                // accounts of this route, but do continue with the next
                // upstream model instead of failing the whole request.
                skip_routes.insert(attempt.route.id.clone());
                errors.push(format!(
                    "账号「{}」/上游模型「{}」拒绝了请求（HTTP {}）：{}",
                    attempt.account.name, attempt.route.upstream_model, status, summary
                ));
                let latency = started.elapsed().as_millis() as u64;
                gw.record_usage(UsageRecord {
                    ts: now_ts(),
                    request_id: request_id.clone(),
                    account_id: attempt.account.id.clone(),
                    provider_id: attempt.provider.id.clone(),
                    local_key_id: auth.key.as_ref().map(|k| k.id.clone()),
                    client_model: client_model.clone(),
                    upstream_model: attempt.route.upstream_model.clone(),
                    protocol: attempt.upstream_protocol.label().to_string(),
                    status: status.as_u16(),
                    ok: false,
                    stream: stream_requested,
                    cancelled: false,
                    prompt_tokens: 0,
                    cached_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                    cost_micros: 0,
                    latency_ms: latency,
                    kind: "chat".into(),
                });
                gw.scheduler.breaker.record_success();
                continue;
            }
            404 => {
                // The upstream model name or path is wrong for this route; try
                // the next upstream model rather than every account of this one.
                skip_routes.insert(attempt.route.id.clone());
                errors.push(format!(
                    "账号「{}」上游路径或模型不存在（HTTP 404）：{}",
                    attempt.account.name, summary
                ));
            }
            408 => {
                gw.scheduler
                    .on_failure(&attempt.account, FailureKind::Timeout, &cfg.scheduler);
                errors.push(format!(
                    "账号「{}」上游请求超时（HTTP 408）：{}",
                    attempt.account.name, summary
                ));
            }
            500..=599 => {
                gw.scheduler
                    .on_failure(&attempt.account, FailureKind::Server, &cfg.scheduler);
                notify_account_failure(&gw, attempt, status.as_u16(), "server", &summary);
                errors.push(format!(
                    "账号「{}」上游错误（HTTP {}）：{}",
                    attempt.account.name, status, summary
                ));
            }
            _ => {
                errors.push(format!(
                    "账号「{}」返回 HTTP {}：{}",
                    attempt.account.name, status, summary
                ));
            }
        }
    }

    // All attempts failed.
    let latency = started.elapsed().as_millis() as u64;
    let (account_id, provider_id) = last_failure.unwrap_or_default();
    gw.record_usage(UsageRecord {
        ts: now_ts(),
        request_id: request_id.clone(),
        account_id,
        provider_id,
        local_key_id: auth.key.as_ref().map(|k| k.id.clone()),
        client_model: client_model.clone(),
        upstream_model: String::new(),
        protocol: ingress
            .protocol()
            .map(|p| p.label().to_string())
            .unwrap_or_default(),
        status: last_status.as_u16(),
        ok: false,
        stream: stream_requested,
        cancelled: false,
        prompt_tokens: 0,
        cached_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        cost_micros: 0,
        latency_ms: latency,
        kind: "chat".into(),
    });
    let opened = gw
        .scheduler
        .breaker
        .record_all_failed(&cfg.scheduler, now_ts());
    let mut message = format!(
        "所有上游尝试均失败（共 {attempts_made}/{total_attempts} 次）。原因：{}",
        errors.join("；")
    );
    if opened {
        message.push_str(&format!(
            "；连续失败已达阈值，已进入 {} 秒全局熔断。",
            cfg.scheduler.breaker_cooldown_secs
        ));
    }
    let status = if last_status == StatusCode::TOO_MANY_REQUESTS {
        StatusCode::TOO_MANY_REQUESTS
    } else {
        StatusCode::BAD_GATEWAY
    };
    gw.plugins.notify(
        "onError",
        serde_json::json!({
            "event": "onError",
            "model": client_model,
            "status": status.as_u16(),
            "attempts": attempts_made,
            "message": truncate_text(&message, 1200),
        }),
    );
    let mut resp = gw.error_response(
        ingress,
        status,
        "upstream_unavailable",
        &truncate_text(&message, 1200),
    );
    resp.headers_mut().insert(
        "x-yoyo-attempts",
        attempts_made.to_string().parse().unwrap(),
    );
    if status == StatusCode::TOO_MANY_REQUESTS {
        resp.headers_mut()
            .insert("retry-after", "5".parse().unwrap());
    }
    resp
}

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

struct StreamFinalizer {
    db: Arc<Db>,
    scheduler: Arc<Scheduler>,
    account: crate::config::Account,
    route: ModelRoute,
    provider: Provider,
    client_model: String,
    local_key_id: Option<String>,
    request_id: String,
    usage: Arc<Mutex<Usage>>,
    plugins: Arc<PluginHost>,
    account_name: String,
    provider_name: String,
    /// Currency units per 1 USD for this route's pricing currency.
    fx_units_per_usd: f64,
    _slot: Option<AccountSlot>,
    _permit: Option<GatePermit>,
    started: Instant,
    done: bool,
    completed: bool,
    failed: bool,
}

impl StreamFinalizer {
    fn finish(&mut self) {
        self.completed = true;
        self.record();
    }

    fn record(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        let usage = *self.usage.lock().unwrap_or_else(|e| e.into_inner());
        let latency = self.started.elapsed().as_millis() as u64;
        let ok = self.completed && !self.failed;
        let rec = UsageRecord {
            ts: now_ts(),
            request_id: self.request_id.clone(),
            account_id: self.account.id.clone(),
            provider_id: self.provider.id.clone(),
            local_key_id: self.local_key_id.clone(),
            client_model: self.client_model.clone(),
            upstream_model: self.route.upstream_model.clone(),
            protocol: self
                .provider
                .protocol_for(&self.route.upstream_model, self.route.protocol)
                .label()
                .to_string(),
            status: 200,
            ok,
            stream: true,
            cancelled: !self.completed,
            prompt_tokens: usage.input_tokens,
            cached_tokens: usage.cached_tokens,
            completion_tokens: usage.output_tokens,
            total_tokens: usage.total(),
            cost_micros: self.route.cost_micros_with_fx(
                usage.input_tokens,
                usage.cached_tokens,
                usage.output_tokens,
                chrono::Local::now(),
                self.fx_units_per_usd,
            ),
            latency_ms: latency,
            kind: "chat".into(),
        };
        if let Err(err) = self.db.record_usage(&rec) {
            tracing::error!("failed to record stream usage: {err}");
        }
        self.plugins.notify(
            "onRequestCompleted",
            serde_json::json!({
                "event": "onRequestCompleted",
                "request": {
                    "id": self.request_id,
                    "model": self.client_model,
                    "upstreamModel": self.route.upstream_model,
                    "account": self.account_name,
                    "provider": self.provider_name,
                    "protocol": rec.protocol,
                    "status": rec.status,
                    "ok": ok,
                    "stream": true,
                },
                "usage": {
                    "promptTokens": rec.prompt_tokens,
                    "cachedTokens": rec.cached_tokens,
                    "completionTokens": rec.completion_tokens,
                    "totalTokens": rec.total_tokens,
                    "costMicros": rec.cost_micros,
                    "latencyMs": rec.latency_ms,
                }
            }),
        );
        self.scheduler
            .note_account_usage(&self.account.id, rec.total_tokens, 1);
        if ok {
            self.scheduler.on_success(&self.account.id);
            self.scheduler.breaker.record_success();
        } else if self.completed {
            self.scheduler.breaker.record_success();
        }
        // Slots and the global permit are released when this value is dropped.
    }
}

impl Drop for StreamFinalizer {
    fn drop(&mut self) {
        self.record();
    }
}

fn build_sse_body<S>(
    upstream: S,
    mut translator: StreamTranslator,
    mut finalizer: StreamFinalizer,
    usage_cell: Arc<Mutex<Usage>>,
    speed: Arc<SpeedMeter>,
    idle_timeout: Duration,
) -> Body
where
    S: Stream<Item = Result<Bytes, String>> + Send + 'static,
{
    let stream = async_stream::stream! {
        futures_util::pin_mut!(upstream);
        // Streams rarely carry a running token count, so the live meter counts
        // generated characters (about four per token) until the final usage
        // block lands, then reconciles against the real number.
        let mut metered_tokens = 0u64;
        let mut decoder = SseDecoder::default();
        let mut hard_failed = false;
        loop {
            match tokio::time::timeout(idle_timeout, upstream.next()).await {
                Err(_) => {
                    finalizer.failed = true;
                    hard_failed = true;
                    let frame = translator.error_frame("上游流读取超时，连接已中断");
                    yield Ok::<Bytes, std::io::Error>(Bytes::from(frame));
                    break;
                }
                Ok(None) => break,
                Ok(Some(Err(err))) => {
                    finalizer.failed = true;
                    hard_failed = true;
                    let frame = translator.error_frame(&format!(
                        "上游流中断：{}",
                        truncate_text(&err.to_string(), 200)
                    ));
                    yield Ok::<Bytes, std::io::Error>(Bytes::from(frame));
                    break;
                }
                Ok(Some(Ok(chunk))) => {
                    for event in decoder.push(&chunk) {
                        if is_error_event(&event) {
                            finalizer.failed = true;
                            hard_failed = true;
                        }
                        for frame in translator.translate(&event) {
                            if !frame.is_empty() {
                                yield Ok::<Bytes, std::io::Error>(Bytes::from(frame));
                            }
                        }
                        let usage = translator.usage();
                        *usage_cell.lock().unwrap_or_else(|e| e.into_inner()) = usage;
                        let produced = if usage.output_tokens > 0 {
                            usage.output_tokens
                        } else {
                            (translator.produced_chars() / 4) as u64
                        };
                        let delta = produced.saturating_sub(metered_tokens);
                        if delta > 0 {
                            metered_tokens = produced;
                            speed.record(delta);
                        }
                    }
                }
            }
        }
        if let Some(event) = decoder.flush() {
            if is_error_event(&event) {
                finalizer.failed = true;
                hard_failed = true;
            }
            for frame in translator.translate(&event) {
                if !frame.is_empty() {
                    yield Ok::<Bytes, std::io::Error>(Bytes::from(frame));
                }
            }
        }
        if !hard_failed && !finalizer.failed {
            for frame in translator.finish() {
                if !frame.is_empty() {
                    yield Ok::<Bytes, std::io::Error>(Bytes::from(frame));
                }
            }
        }
        let final_usage = translator.usage();
        *usage_cell.lock().unwrap_or_else(|e| e.into_inner()) = final_usage;
        // Charge any tokens the character estimate missed (tool calls, CJK
        // text, or a final usage block that arrived after the last chunk).
        let delta = final_usage
            .output_tokens
            .saturating_sub(metered_tokens);
        if delta > 0 {
            speed.record(delta);
        }
        finalizer.finish();
    };
    Body::from_stream(stream)
}

fn model_allowed(auth: &AuthContext, model: &str) -> bool {
    auth.key
        .as_ref()
        .map(|key| key.allows_model(model))
        .unwrap_or(true)
}

fn notify_account_failure(
    gw: &Gateway,
    attempt: &crate::scheduler::Attempt,
    status: u16,
    kind: &str,
    message: &str,
) {
    gw.plugins.notify(
        "onAccountFailure",
        serde_json::json!({
            "event": "onAccountFailure",
            "account": attempt.account.name,
            "accountId": attempt.account.id,
            "provider": attempt.provider.name,
            "providerId": attempt.provider.id,
            "model": attempt.route.client_model,
            "upstreamModel": attempt.route.upstream_model,
            "status": status,
            "kind": kind,
            "message": message,
        }),
    );
}

fn is_error_event(ev: &protocol::SseEvent) -> bool {
    if ev.event.as_deref() == Some("error") {
        return true;
    }
    serde_json::from_str::<Value>(&ev.data)
        .ok()
        .map(|v| {
            v.get("error").is_some()
                || v.get("type")
                    .and_then(Value::as_str)
                    .map(|t| t == "error" || t == "response.failed")
                    .unwrap_or(false)
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Generic passthrough (embeddings, non-chat models, ...)
// ---------------------------------------------------------------------------

async fn generic_handler(
    State(gw): State<Arc<Gateway>>,
    method: axum::http::Method,
    headers: HeaderMap,
    uri: axum::http::Uri,
    body: Bytes,
) -> Response {
    let path = uri.path();
    if method != axum::http::Method::POST || !path.starts_with("/v1/") || path == "/v1/" {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error":{"message":"未知端点。YoyoRelay 提供 /v1/chat/completions、/v1/messages、/v1/responses、/v1/models。","type":"not_found"}})),
        )
            .into_response();
    }
    let ingress = Ingress::Generic;
    let auth = match gw.authenticate(&headers) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let mut payload: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(err) => {
            return gw.error_response(
                ingress,
                StatusCode::BAD_REQUEST,
                "invalid_json",
                &format!("请求体不是有效 JSON：{err}"),
            )
        }
    };
    let Some(client_model) = protocol::extract_client_model(&payload) else {
        return gw.error_response(
            ingress,
            StatusCode::BAD_REQUEST,
            "missing_model",
            "请求体缺少 model 字段。",
        );
    };
    if let Some(key) = &auth.key {
        if !key.allows_model(&client_model) {
            return gw.error_response(
                ingress,
                StatusCode::FORBIDDEN,
                "model_not_allowed",
                &format!("本地 Key「{}」无权访问模型 {client_model}。", key.name),
            );
        }
    }
    let cfg = gw.config();
    if !gw.scheduler.breaker.allow(&cfg.scheduler, now_ts()) {
        return gw.error_response(
            ingress,
            StatusCode::SERVICE_UNAVAILABLE,
            "relay_breaker_open",
            "全局熔断已开启。",
        );
    }
    let _permit = match gw
        .gate
        .acquire(cfg.server.queue_capacity, cfg.server.queue_timeout_secs)
        .await
    {
        Ok(p) => p,
        Err(_) => {
            return gw.error_response(
                ingress,
                StatusCode::TOO_MANY_REQUESTS,
                "queue_full",
                "本地代理排队已满。",
            )
        }
    };

    let chain = gw
        .scheduler
        .build_chain(&cfg, &client_model, Ingress::Generic, true);
    if chain.attempts.is_empty() {
        return gw.error_response(
            ingress,
            StatusCode::SERVICE_UNAVAILABLE,
            "no_available_route",
            &format!(
                "模型 {client_model} 没有可用的透传路由：{}",
                chain.notes.join("；")
            ),
        );
    }
    let suffix = path.trim_start_matches("/v1/");
    let started = Instant::now();
    let request_id = uuid::Uuid::new_v4().to_string();
    let mut errors = chain.notes.clone();
    let mut attempts = 0usize;
    for attempt in &chain.attempts {
        attempts += 1;
        if attempt.route.mode != crate::config::RouteMode::Passthrough {
            errors.push(format!("路由 {} 不是透传模式", attempt.route.client_model));
            continue;
        }
        let Some(_slot) = gw
            .scheduler
            .try_acquire_slot(&attempt.account.id, attempt.account.max_concurrency)
        else {
            errors.push(format!("账号「{}」并发已满", attempt.account.name));
            continue;
        };
        let key = match gw.master.decrypt(&attempt.account.api_key) {
            Ok(k) => k,
            Err(err) => {
                errors.push(format!("账号 Key 解密失败：{err}"));
                continue;
            }
        };
        payload["model"] = Value::String(attempt.route.upstream_model.clone());
        let url = attempt.provider.join_url(suffix);
        let mut req = gw
            .client_for(&url)
            .post(&url)
            .header(header::CONTENT_TYPE, "application/json")
            .json(&payload);
        req = match apply_auth(req, &attempt.provider.auth, &key) {
            Ok(r) => r,
            Err(err) => {
                errors.push(format!("鉴权头无效：{err}"));
                continue;
            }
        };
        for h in &attempt.provider.extra_headers {
            if !h.name.trim().is_empty() {
                req = req.header(h.name.trim(), h.value.as_str());
            }
        }
        let timeout = attempt
            .provider
            .timeout_secs
            .unwrap_or(cfg.server.upstream_timeout_secs)
            .max(5);
        let slow_secs = if cfg.scheduler.slow_switch {
            cfg.scheduler.slow_first_byte_secs.clamp(3, timeout)
        } else {
            0
        };
        let resp = match tokio::time::timeout(Duration::from_secs(timeout), req.send()).await {
            Err(_) => {
                gw.scheduler
                    .on_failure(&attempt.account, FailureKind::Timeout, &cfg.scheduler);
                errors.push(format!("账号「{}」超时", attempt.account.name));
                continue;
            }
            Ok(Err(err)) => {
                gw.scheduler
                    .on_failure(&attempt.account, FailureKind::Network, &cfg.scheduler);
                errors.push(format!(
                    "账号「{}」{}",
                    attempt.account.name,
                    readable_reqwest_error(&err)
                ));
                continue;
            }
            Ok(Ok(r)) => r,
        };
        let status = resp.status();
        if status.is_success() {
            let content_type = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/json")
                .to_string();
            let first_deadline = if slow_secs > 0 { slow_secs } else { timeout };
            let (first, resp) = match read_first_chunk(resp, first_deadline).await {
                Ok(value) => value,
                Err(FirstChunkError::Slow) => {
                    gw.scheduler.on_failure(
                        &attempt.account,
                        if slow_secs > 0 {
                            FailureKind::Slow
                        } else {
                            FailureKind::Timeout
                        },
                        &cfg.scheduler,
                    );
                    errors.push(format!(
                        "账号「{}」首包超过 {}s，自动切换",
                        attempt.account.name, first_deadline
                    ));
                    continue;
                }
                Err(FirstChunkError::Closed) => {
                    gw.scheduler
                        .on_failure(&attempt.account, FailureKind::Server, &cfg.scheduler);
                    errors.push(format!("账号「{}」返回空响应", attempt.account.name));
                    continue;
                }
                Err(FirstChunkError::Network(err)) => {
                    gw.scheduler
                        .on_failure(&attempt.account, FailureKind::Network, &cfg.scheduler);
                    errors.push(format!(
                        "账号「{}」读取首包失败：{err}",
                        attempt.account.name
                    ));
                    continue;
                }
            };
            let mut bytes = first.to_vec();
            match tokio::time::timeout(Duration::from_secs(timeout), resp.bytes()).await {
                Ok(Ok(rest)) => bytes.extend_from_slice(&rest),
                _ => {
                    gw.scheduler
                        .on_failure(&attempt.account, FailureKind::Network, &cfg.scheduler);
                    errors.push(format!("账号「{}」读取响应失败", attempt.account.name));
                    continue;
                }
            }
            let usage = extract_generic_usage(&bytes);
            let latency = started.elapsed().as_millis() as u64;
            gw.scheduler.on_success(&attempt.account.id);
            gw.scheduler.breaker.record_success();
            gw.record_usage(UsageRecord {
                ts: now_ts(),
                request_id: request_id.clone(),
                account_id: attempt.account.id.clone(),
                provider_id: attempt.provider.id.clone(),
                local_key_id: auth.key.as_ref().map(|k| k.id.clone()),
                client_model: client_model.clone(),
                upstream_model: attempt.route.upstream_model.clone(),
                protocol: "passthrough".into(),
                status: 200,
                ok: true,
                stream: false,
                cancelled: false,
                prompt_tokens: usage.input_tokens,
                cached_tokens: usage.cached_tokens,
                completion_tokens: 0,
                total_tokens: usage.total(),
                cost_micros: attempt.route.cost_micros_with_fx(
                    usage.input_tokens,
                    usage.cached_tokens,
                    0,
                    chrono::Local::now(),
                    cfg.ui
                        .fx_rate(attempt.route.effective_pricing().currency.code()),
                ),
                latency_ms: latency,
                kind: "non_chat".into(),
            });
            let mut response = Response::new(Body::from(bytes));
            *response.status_mut() = status;
            if let Ok(v) = content_type.parse() {
                response.headers_mut().insert(header::CONTENT_TYPE, v);
            }
            response
                .headers_mut()
                .insert("x-yoyo-attempts", attempts.to_string().parse().unwrap());
            return response;
        }
        let err_bytes = resp.bytes().await.unwrap_or_default();
        let text = String::from_utf8_lossy(&err_bytes).to_string();
        match status.as_u16() {
            401 | 403 => {
                gw.scheduler
                    .on_failure(&attempt.account, FailureKind::Auth, &cfg.scheduler);
            }
            429 => {
                gw.scheduler
                    .on_failure(&attempt.account, classify_429(&text), &cfg.scheduler);
            }
            500..=599 => {
                gw.scheduler
                    .on_failure(&attempt.account, FailureKind::Server, &cfg.scheduler);
            }
            _ => {}
        }
        errors.push(format!(
            "账号「{}」HTTP {}：{}",
            attempt.account.name,
            status,
            upstream_error_message(&text)
        ));
    }
    gw.scheduler
        .breaker
        .record_all_failed(&cfg.scheduler, now_ts());
    gw.error_response(
        ingress,
        StatusCode::BAD_GATEWAY,
        "upstream_unavailable",
        &format!("透传请求失败：{}", errors.join("；")),
    )
}

fn extract_generic_usage(bytes: &[u8]) -> Usage {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return Usage::default();
    };
    let mut u = Usage::default();
    if let Some(usage) = value.get("usage").and_then(Value::as_object) {
        u.input_tokens = usage
            .get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        u.output_tokens = usage
            .get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        u.total_tokens = usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
    }
    u
}

#[cfg(test)]
mod speed_tests {
    use super::SpeedMeter;
    use std::time::Duration;

    #[test]
    fn meter_is_idle_without_samples() {
        let meter = SpeedMeter::new();
        assert_eq!(meter.tokens_per_sec(), 0.0);
    }

    #[test]
    fn meter_reports_recent_tokens() {
        let meter = SpeedMeter::new();
        meter.record(60);
        let tps = meter.tokens_per_sec();
        assert!(tps > 0.0, "expected a positive rate, got {tps}");
        // 60 tokens inside a three-second window is at most 60/s.
        assert!(tps <= 60.0 + f64::EPSILON);
    }

    #[test]
    fn completed_span_is_spread_over_time() {
        let meter = SpeedMeter::new();
        meter.record_span(800, Duration::from_secs(4));
        let tps = meter.tokens_per_sec();
        assert!(tps > 0.0, "expected a positive rate, got {tps}");
        // The whole reply must never look faster than 800 tokens over the
        // three-second reporting window.
        assert!(tps <= 800.0 / 3.0 + 1.0, "unexpected burst: {tps}");
    }
}
