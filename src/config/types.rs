use serde::{Deserialize, Serialize};

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Top-level configuration
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct AppConfig {
    pub schema_version: u32,
    pub server: ServerConfig,
    pub scheduler: SchedulerConfig,
    pub ui: UiConfig,
    pub providers: Vec<Provider>,
    pub accounts: Vec<Account>,
    pub routes: Vec<ModelRoute>,
    pub local_keys: Vec<LocalKey>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            server: ServerConfig::default(),
            scheduler: SchedulerConfig::default(),
            ui: UiConfig::default(),
            providers: Vec::new(),
            accounts: Vec::new(),
            routes: Vec::new(),
            local_keys: Vec::new(),
        }
    }
}

impl AppConfig {
    pub fn provider(&self, id: &str) -> Option<&Provider> {
        self.providers.iter().find(|p| p.id == id)
    }

    pub fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn route(&self, id: &str) -> Option<&ModelRoute> {
        self.routes.iter().find(|r| r.id == id)
    }

    pub fn local_key(&self, id: &str) -> Option<&LocalKey> {
        self.local_keys.iter().find(|k| k.id == id)
    }

    /// Enabled routes for a client model, ordered by priority (high first).
    pub fn routes_for(&self, client_model: &str) -> Vec<ModelRoute> {
        let mut routes: Vec<_> = self
            .routes
            .iter()
            .filter(|r| r.enabled && r.client_model == client_model)
            .cloned()
            .collect();
        routes.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });
        routes
    }

    /// All distinct client models exposed by enabled routes.
    pub fn client_models(&self) -> Vec<String> {
        let mut models: Vec<String> = self
            .routes
            .iter()
            .filter(|r| r.enabled)
            .map(|r| r.client_model.clone())
            .collect();
        models.sort();
        models.dedup();
        models
    }

    pub fn accounts_of(&self, provider_id: &str) -> Vec<Account> {
        self.accounts
            .iter()
            .filter(|a| a.provider_id == provider_id)
            .cloned()
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Server / scheduler / UI settings
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    pub auth_enabled: bool,
    pub max_global_concurrency: usize,
    pub queue_capacity: usize,
    pub queue_timeout_secs: u64,
    pub upstream_timeout_secs: u64,
    pub stream_idle_timeout_secs: u64,
    pub cors: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".to_string(),
            port: 7789,
            auth_enabled: true,
            max_global_concurrency: 50,
            queue_capacity: 100,
            queue_timeout_secs: 30,
            upstream_timeout_secs: 120,
            stream_idle_timeout_secs: 120,
            cors: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SchedulerConfig {
    /// Number of switch attempts after the first one (4 total by default).
    pub max_retries: u32,
    /// Extra attempts granted to the third (mapping) fallback layer.
    pub fallback_retries: u32,
    pub cooldown_secs: u64,
    pub concurrency_cooldown_secs: u64,
    pub auth_cooldown_secs: u64,
    pub max_cooldown_secs: u64,
    pub breaker_threshold: u32,
    pub breaker_cooldown_secs: u64,
    pub half_open_probe_secs: u64,
    pub quota_cache_secs: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            fallback_retries: 2,
            cooldown_secs: 60,
            concurrency_cooldown_secs: 15,
            auth_cooldown_secs: 300,
            max_cooldown_secs: 1800,
            breaker_threshold: 20,
            breaker_cooldown_secs: 30,
            half_open_probe_secs: 10,
            quota_cache_secs: 10,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    Light,
    Dark,
    System,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct UiConfig {
    pub theme: ThemeMode,
    /// Launch YoyoRelay automatically when the user signs in.
    pub autostart: bool,
    /// Start minimized instead of showing the window.
    pub start_minimized: bool,
    /// Hide the window instead of quitting when the close button is pressed.
    pub close_to_tray: bool,
    /// Show live request/token information in the window title (taskbar tooltip).
    pub show_taskbar_stats: bool,
    pub notifications: bool,
    pub retention_days: u32,
    pub debug_mode: bool,
    pub sidebar_width: f32,
    pub ui_scale: f32,
    pub balance_refresh_mins: u64,
    pub low_balance_threshold: f64,
    pub onboarding_done: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: ThemeMode::Light,
            autostart: false,
            start_minimized: false,
            close_to_tray: false,
            show_taskbar_stats: true,
            notifications: true,
            retention_days: 90,
            debug_mode: false,
            sidebar_width: 190.0,
            ui_scale: 0.0,
            balance_refresh_mins: 10,
            low_balance_threshold: 5.0,
            onboarding_done: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Protocol vocabulary
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    #[default]
    #[serde(rename = "openai_chat", alias = "openai-chat")]
    OpenAiChat,
    #[serde(rename = "anthropic_messages", alias = "anthropic-messages")]
    AnthropicMessages,
    #[serde(rename = "openai_responses", alias = "openai-responses")]
    OpenAiResponses,
}

impl Protocol {
    pub fn label(self) -> &'static str {
        match self {
            Protocol::OpenAiChat => "openai-chat",
            Protocol::AnthropicMessages => "anthropic-messages",
            Protocol::OpenAiResponses => "openai-responses",
        }
    }

    #[allow(dead_code)]
    pub fn ingress_path(self) -> &'static str {
        match self {
            Protocol::OpenAiChat => "/v1/chat/completions",
            Protocol::AnthropicMessages => "/v1/messages",
            Protocol::OpenAiResponses => "/v1/responses",
        }
    }

    #[allow(dead_code)]
    pub fn supports_tools(self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[default]
    Official,
    Aggregator,
    Custom,
}

impl ProviderKind {
    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Official => "官方直连",
            ProviderKind::Aggregator => "聚合平台",
            ProviderKind::Custom => "自定义",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    #[default]
    Bearer,
    XApiKey,
    Custom,
    None,
}

impl AuthMode {
    pub fn label(self) -> &'static str {
        match self {
            AuthMode::Bearer => "Authorization: Bearer",
            AuthMode::XApiKey => "x-api-key",
            AuthMode::Custom => "自定义 Header",
            AuthMode::None => "无需鉴权",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct AuthConfig {
    pub mode: AuthMode,
    pub header_name: String,
    pub scheme: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            mode: AuthMode::Bearer,
            header_name: String::new(),
            scheme: String::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolStrategyMode {
    #[default]
    Static,
    ModelPrefix,
    Explicit,
}

impl ProtocolStrategyMode {
    pub fn label(self) -> &'static str {
        match self {
            ProtocolStrategyMode::Static => "固定协议",
            ProtocolStrategyMode::ModelPrefix => "按模型前缀",
            ProtocolStrategyMode::Explicit => "由路由指定",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct PrefixRule {
    pub prefix: String,
    pub protocol: Protocol,
}

impl Default for PrefixRule {
    fn default() -> Self {
        Self {
            prefix: String::new(),
            protocol: Protocol::OpenAiChat,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ProtocolStrategy {
    pub mode: ProtocolStrategyMode,
    pub prefix_rules: Vec<PrefixRule>,
}

impl Default for ProtocolStrategy {
    fn default() -> Self {
        Self {
            mode: ProtocolStrategyMode::Static,
            prefix_rules: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct HeaderPair {
    pub name: String,
    pub value: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub auth: AuthConfig,
    pub static_protocol: Protocol,
    pub protocol_strategy: ProtocolStrategy,
    pub extra_headers: Vec<HeaderPair>,
    pub timeout_secs: Option<u64>,
    pub test_model: Option<String>,
    pub preset: Option<String>,
    pub created_at: i64,
}

impl Default for Provider {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: ProviderKind::Official,
            base_url: String::new(),
            auth: AuthConfig::default(),
            static_protocol: Protocol::OpenAiChat,
            protocol_strategy: ProtocolStrategy::default(),
            extra_headers: Vec::new(),
            timeout_secs: None,
            test_model: None,
            preset: None,
            created_at: 0,
        }
    }
}

impl Provider {
    /// Resolve the upstream protocol for a given upstream model.
    pub fn protocol_for(&self, upstream_model: &str, explicit: Option<Protocol>) -> Protocol {
        match self.protocol_strategy.mode {
            ProtocolStrategyMode::Static => self.static_protocol,
            ProtocolStrategyMode::Explicit => explicit.unwrap_or(self.static_protocol),
            ProtocolStrategyMode::ModelPrefix => {
                let mut best: Option<(&str, Protocol)> = None;
                for rule in &self.protocol_strategy.prefix_rules {
                    if !rule.prefix.is_empty() && upstream_model.starts_with(&rule.prefix) {
                        if best
                            .map(|(p, _)| rule.prefix.len() > p.len())
                            .unwrap_or(true)
                        {
                            best = Some((&rule.prefix, rule.protocol));
                        }
                    }
                }
                best.map(|(_, p)| p)
                    .or(explicit)
                    .unwrap_or(self.static_protocol)
            }
        }
    }

    pub fn join_url(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            base.to_string()
        } else {
            format!("{base}/{path}")
        }
    }
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum QuotaWindow {
    #[default]
    Daily,
    Weekly,
    Monthly,
    Rolling,
}

impl QuotaWindow {
    pub fn label(self) -> &'static str {
        match self {
            QuotaWindow::Daily => "每日",
            QuotaWindow::Weekly => "每周",
            QuotaWindow::Monthly => "每月",
            QuotaWindow::Rolling => "滚动窗口",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct QuotaConfig {
    pub window: QuotaWindow,
    pub rolling_hours: u32,
    pub limit_tokens: Option<u64>,
    pub limit_requests: Option<u64>,
}

impl Default for QuotaConfig {
    fn default() -> Self {
        Self {
            window: QuotaWindow::Rolling,
            rolling_hours: 5,
            limit_tokens: None,
            limit_requests: None,
        }
    }
}

impl QuotaConfig {
    pub fn window_start(&self, now: chrono::DateTime<chrono::Local>) -> i64 {
        use chrono::{Datelike, Duration, TimeZone};
        match self.window {
            QuotaWindow::Daily => now
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .and_then(|d| chrono::Local.from_local_datetime(&d).single())
                .map(|d| d.timestamp())
                .unwrap_or_else(|| now.timestamp() - 86_400),
            QuotaWindow::Weekly => {
                let days = now.weekday().num_days_from_monday() as i64;
                (now - Duration::days(days))
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .and_then(|d| chrono::Local.from_local_datetime(&d).single())
                    .map(|d| d.timestamp())
                    .unwrap_or_else(|| now.timestamp() - 7 * 86_400)
            }
            QuotaWindow::Monthly => {
                let first = now.date_naive().with_day(1).unwrap_or(now.date_naive());
                first
                    .and_hms_opt(0, 0, 0)
                    .and_then(|d| chrono::Local.from_local_datetime(&d).single())
                    .map(|d| d.timestamp())
                    .unwrap_or_else(|| now.timestamp() - 30 * 86_400)
            }
            QuotaWindow::Rolling => now.timestamp() - (self.rolling_hours.max(1) as i64) * 3600,
        }
    }

    pub fn reset_at(&self, now: chrono::DateTime<chrono::Local>) -> i64 {
        use chrono::{Datelike, Duration, TimeZone};
        match self.window {
            QuotaWindow::Daily => {
                let next = now.date_naive() + Duration::days(1);
                next.and_hms_opt(0, 0, 0)
                    .and_then(|d| chrono::Local.from_local_datetime(&d).single())
                    .map(|d| d.timestamp())
                    .unwrap_or_else(|| now.timestamp() + 86_400)
            }
            QuotaWindow::Weekly => {
                let days_to_monday = 7 - now.weekday().num_days_from_monday() as i64;
                let next = now.date_naive() + Duration::days(days_to_monday);
                next.and_hms_opt(0, 0, 0)
                    .and_then(|d| chrono::Local.from_local_datetime(&d).single())
                    .map(|d| d.timestamp())
                    .unwrap_or_else(|| now.timestamp() + 7 * 86_400)
            }
            QuotaWindow::Monthly => {
                let (y, m) = if now.month() == 12 {
                    (now.year() + 1, 1)
                } else {
                    (now.year(), now.month() + 1)
                };
                chrono::NaiveDate::from_ymd_opt(y, m, 1)
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .and_then(|d| chrono::Local.from_local_datetime(&d).single())
                    .map(|d| d.timestamp())
                    .unwrap_or_else(|| now.timestamp() + 30 * 86_400)
            }
            QuotaWindow::Rolling => now.timestamp() + (self.rolling_hours.max(1) as i64) * 3600,
        }
    }

    #[allow(dead_code)]
    pub fn has_limit(&self) -> bool {
        self.limit_tokens.is_some() || self.limit_requests.is_some()
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BalanceMode {
    #[default]
    Auto,
    Manual,
    Unavailable,
}

impl BalanceMode {
    pub fn label(self) -> &'static str {
        match self {
            BalanceMode::Auto => "自动查询",
            BalanceMode::Manual => "手动录入",
            BalanceMode::Unavailable => "不可用",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct BalanceConfig {
    pub mode: BalanceMode,
    pub manual_amount: f64,
    pub manual_updated_at: i64,
    pub url: Option<String>,
    pub json_path: String,
    pub scale: f64,
    pub currency: String,
}

impl Default for BalanceConfig {
    fn default() -> Self {
        Self {
            mode: BalanceMode::Unavailable,
            manual_amount: 0.0,
            manual_updated_at: 0,
            url: None,
            json_path: "balance".to_string(),
            scale: 1.0,
            currency: "USD".to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Account {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    /// AES-256-GCM envelope. Never serialized in plaintext.
    pub api_key: String,
    pub priority: i32,
    pub enabled: bool,
    pub max_concurrency: usize,
    pub quota: Option<QuotaConfig>,
    pub cooldown_override_secs: Option<u64>,
    pub balance: BalanceConfig,
    pub created_at: i64,
}

impl Default for Account {
    fn default() -> Self {
        Self {
            id: String::new(),
            provider_id: String::new(),
            name: String::new(),
            api_key: String::new(),
            priority: 0,
            enabled: true,
            max_concurrency: 4,
            quota: None,
            cooldown_override_secs: None,
            balance: BalanceConfig::default(),
            created_at: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RouteMode {
    /// Convert between client and upstream protocols when they differ.
    #[default]
    Adapt,
    /// Only use this route when the protocols match, forward bytes untouched.
    Passthrough,
}

impl RouteMode {
    pub fn label(self) -> &'static str {
        match self {
            RouteMode::Adapt => "自动适配",
            RouteMode::Passthrough => "原样透传",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ModelRoute {
    pub id: String,
    pub client_model: String,
    pub provider_id: String,
    pub upstream_model: String,
    /// Optional explicit protocol for providers using the "explicit" strategy.
    pub protocol: Option<Protocol>,
    pub mode: RouteMode,
    pub priority: i32,
    pub capabilities: Vec<String>,
    pub max_context_tokens: Option<u64>,
    /// Third-layer fallback: another client-model mapping chain.
    pub fallback_client_model: Option<String>,
    pub input_price_per_mtok: f64,
    pub output_price_per_mtok: f64,
    pub enabled: bool,
    pub created_at: i64,
}

impl Default for ModelRoute {
    fn default() -> Self {
        Self {
            id: String::new(),
            client_model: String::new(),
            provider_id: String::new(),
            upstream_model: String::new(),
            protocol: None,
            mode: RouteMode::Adapt,
            priority: 0,
            capabilities: vec!["chat".to_string()],
            max_context_tokens: None,
            fallback_client_model: None,
            input_price_per_mtok: 0.0,
            output_price_per_mtok: 0.0,
            enabled: true,
            created_at: 0,
        }
    }
}

impl ModelRoute {
    pub fn cost_micros(&self, prompt_tokens: u64, completion_tokens: u64) -> u64 {
        let input = (prompt_tokens as f64) * self.input_price_per_mtok / 1_000_000.0;
        let output = (completion_tokens as f64) * self.output_price_per_mtok / 1_000_000.0;
        ((input + output) * 1_000_000.0).round() as u64
    }
}

// ---------------------------------------------------------------------------
// Local keys
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct LocalKey {
    pub id: String,
    pub name: String,
    /// AES-256-GCM envelope of the `yr-...` plaintext.
    pub key: String,
    pub enabled: bool,
    pub allowed_models: Vec<String>,
    pub max_tokens: Option<u64>,
    pub max_requests: Option<u64>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

impl Default for LocalKey {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            key: String::new(),
            enabled: true,
            allowed_models: Vec::new(),
            max_tokens: None,
            max_requests: None,
            created_at: 0,
            last_used_at: None,
        }
    }
}

impl LocalKey {
    pub fn allows_model(&self, model: &str) -> bool {
        self.allowed_models.is_empty() || self.allowed_models.iter().any(|m| m == model || m == "*")
    }
}

// ---------------------------------------------------------------------------
// Small shared helpers
// ---------------------------------------------------------------------------

pub fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn mask_secret(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= 8 {
        return "••••".to_string();
    }
    let head: String = chars.iter().take(4).collect();
    let tail: String = chars.iter().skip(chars.len() - 4).collect();
    format!("{head}…{tail}")
}

#[allow(dead_code)]
pub fn fmt_ts(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "-".to_string())
}

pub fn fmt_duration_secs(secs: i64) -> String {
    if secs <= 0 {
        return "0s".to_string();
    }
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h{m:02}m")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}
