use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const CONFIG_SCHEMA_VERSION: u32 = 6;

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
    /// Client-facing model metadata. Routes reference these by client model name.
    pub model_profiles: Vec<ClientModelProfile>,
    /// Reusable pricing tables that routes can apply in one click.
    pub price_plans: Vec<PricePlan>,
    /// JavaScript plugin host settings. Plugin code itself lives on disk.
    pub plugins: PluginSettings,
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
            model_profiles: Vec::new(),
            price_plans: Vec::new(),
            plugins: PluginSettings::default(),
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

    /// All routes for a client model, including disabled ones. Used by the UI
    /// so disabling an upstream never hides it from the route page.
    pub fn all_routes_for(&self, client_model: &str) -> Vec<ModelRoute> {
        let mut routes: Vec<_> = self
            .routes
            .iter()
            .filter(|r| r.client_model == client_model)
            .cloned()
            .collect();
        routes.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });
        routes
    }

    /// All distinct client models known to the configuration, including
    /// disabled routes and profile-only entries.
    pub fn client_models(&self) -> Vec<String> {
        let mut models: Vec<String> = self
            .routes
            .iter()
            .map(|r| r.client_model.clone())
            .chain(self.model_profiles.iter().map(|p| p.client_model.clone()))
            .collect();
        models.sort();
        models.dedup();
        models
    }

    /// Models that can actually receive traffic (at least one enabled route
    /// and, when present, an enabled model profile).
    pub fn servable_models(&self) -> Vec<String> {
        let mut models: Vec<String> = self
            .routes
            .iter()
            .filter(|r| r.enabled && self.model_enabled(&r.client_model))
            .map(|r| r.client_model.clone())
            .collect();
        models.sort();
        models.dedup();
        models
    }

    pub fn model_enabled(&self, client_model: &str) -> bool {
        self.model_profile(client_model)
            .map(|profile| profile.enabled)
            .unwrap_or(true)
    }

    pub fn model_profile(&self, client_model: &str) -> Option<&ClientModelProfile> {
        self.model_profiles
            .iter()
            .find(|p| p.client_model == client_model)
    }

    /// Mapping-level fallback configured on the model profile first, then on
    /// any route (legacy layout).
    pub fn fallback_for(&self, client_model: &str) -> Option<String> {
        self.model_profile(client_model)
            .and_then(|p| p.fallback_client_model.clone())
            .filter(|m| !m.trim().is_empty() && m != client_model)
            .or_else(|| {
                self.routes
                    .iter()
                    .find(|r| r.client_model == client_model)
                    .and_then(|r| r.fallback_client_model.clone())
                    .filter(|m| !m.trim().is_empty() && m != client_model)
            })
    }

    pub fn accounts_of(&self, provider_id: &str) -> Vec<Account> {
        self.accounts
            .iter()
            .filter(|a| a.provider_id == provider_id)
            .cloned()
            .collect()
    }

    #[allow(dead_code)]
    pub fn price_plan(&self, name: &str) -> Option<&PricePlan> {
        self.price_plans.iter().find(|plan| plan.name == name)
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
    /// Retry the same account through a direct (no proxy) client when the
    /// system-proxy connection fails.
    pub direct_fallback: bool,
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
            direct_fallback: true,
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
    /// Detect slow first bytes / first SSE chunks and switch to the next
    /// account (same provider first, then other providers).
    pub slow_switch: bool,
    /// Deadline for the first byte of a response (or first SSE chunk).
    pub slow_first_byte_secs: u64,
    /// Cooldown applied to an account that exceeded the slow deadline.
    pub slow_cooldown_secs: u64,
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
            slow_switch: true,
            slow_first_byte_secs: 20,
            slow_cooldown_secs: 30,
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

/// Live taskbar badge composition and placement (Windows).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct TaskbarBadgeConfig {
    /// Show the proxy status text (running/stopped) as the first fragment.
    pub show_status: bool,
    /// Show the colored status dot.
    pub show_dot: bool,
    pub show_active: bool,
    pub show_tokens: bool,
    pub show_requests: bool,
    pub show_queued: bool,
    pub show_success: bool,
    pub show_cost: bool,
    /// Live aggregate decode throughput (tokens per second).
    pub show_speed: bool,
    /// Draw the capsule on a transparent background instead of a solid card.
    pub transparent_bg: bool,
    /// Override the automatic (theme/backdrop based) badge text colour.
    pub custom_text_color: bool,
    /// `#RRGGBB` text colour used when `custom_text_color` is enabled.
    pub text_color: String,
    /// Overall scale of the badge (text, padding and height).
    pub scale: f32,
    /// Horizontal nudge in physical pixels, relative to the default anchor.
    pub offset_x: i32,
    /// Vertical nudge in physical pixels, relative to the taskbar center.
    pub offset_y: i32,
    /// Distance in physical pixels between the badge and the notification area.
    pub gap: i32,
    /// Whole-badge opacity, 0.25 – 1.0.
    pub opacity: f32,
}

impl Default for TaskbarBadgeConfig {
    fn default() -> Self {
        Self {
            show_status: false,
            show_dot: true,
            show_active: true,
            show_tokens: true,
            show_requests: false,
            show_queued: false,
            show_success: true,
            show_cost: false,
            show_speed: true,
            transparent_bg: false,
            custom_text_color: false,
            text_color: "#111827".to_string(),
            scale: 1.0,
            offset_x: 0,
            offset_y: 0,
            gap: 10,
            opacity: 0.95,
        }
    }
}

impl TaskbarBadgeConfig {
    pub fn normalized(&self) -> Self {
        let mut copy = self.clone();
        copy.scale = if copy.scale.is_finite() {
            copy.scale.clamp(0.6, 2.2)
        } else {
            1.0
        };
        copy.opacity = if copy.opacity.is_finite() {
            copy.opacity.clamp(0.25, 1.0)
        } else {
            1.0
        };
        copy.offset_x = copy.offset_x.clamp(-400, 400);
        copy.offset_y = copy.offset_y.clamp(-80, 80);
        copy.gap = copy.gap.clamp(0, 80);
        copy
    }

    pub fn has_content(&self) -> bool {
        self.show_status
            || self.show_active
            || self.show_tokens
            || self.show_requests
            || self.show_queued
            || self.show_success
            || self.show_cost
            || self.show_speed
    }
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
    /// Legacy/global fallback threshold. Per-currency values in
    /// `low_balance_thresholds` take precedence.
    pub low_balance_threshold: f64,
    /// Currency-specific low-balance thresholds, keyed by ISO code.
    pub low_balance_thresholds: BTreeMap<String, f64>,
    /// Currency units per 1 USD, used to estimate manual balances from
    /// USD-priced route costs and to show a CNY-converted total. Missing
    /// entries are treated as 1.0.
    pub fx_rates: BTreeMap<String, f64>,
    /// Fetch live exchange rates instead of relying only on the manual values.
    pub fx_auto: bool,
    /// How often live rates are refreshed, in hours.
    pub fx_refresh_hours: u64,
    /// Unix timestamp of the last successful live-rate refresh.
    pub fx_updated_at: i64,
    /// Human-readable source of the last live-rate refresh.
    pub fx_source: String,
    /// Windows taskbar badge composition and placement.
    pub taskbar: TaskbarBadgeConfig,
    /// Check GitHub for a newer release in the background.
    pub auto_update_check: bool,
    /// Hours between automatic update checks.
    pub update_check_hours: u64,
    /// The version the user asked not to be prompted about again.
    pub update_dismissed_version: String,
    /// Last release metadata seen, kept so the red dot survives a restart
    /// before the next network check.
    pub update_latest_version: String,
    pub update_latest_url: String,
    pub update_latest_notes: String,
    pub update_checked_at: i64,
    pub onboarding_done: bool,
}

impl UiConfig {
    pub fn threshold_for(&self, currency: &str) -> f64 {
        self.low_balance_thresholds
            .get(&normalize_currency(currency))
            .copied()
            .unwrap_or(self.low_balance_threshold)
    }

    /// Units of `currency` per 1 USD.
    pub fn fx_rate(&self, currency: &str) -> f64 {
        let currency = normalize_currency(currency);
        if currency == "USD" {
            return 1.0;
        }
        self.fx_rates
            .get(&currency)
            .copied()
            .filter(|rate| *rate > 0.0)
            .unwrap_or(1.0)
    }

    /// Convert between two currencies using the USD-based rate table.
    pub fn fx_convert(&self, amount: f64, from: &str, to: &str) -> f64 {
        let from = self.fx_rate(from);
        let to = self.fx_rate(to);
        if from <= 0.0 {
            return amount;
        }
        amount * to / from
    }
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
            low_balance_thresholds: BTreeMap::from([
                ("USD".to_string(), 5.0),
                ("CNY".to_string(), 35.0),
            ]),
            fx_rates: BTreeMap::from([("USD".to_string(), 1.0), ("CNY".to_string(), 7.2)]),
            fx_auto: true,
            fx_refresh_hours: 12,
            fx_updated_at: 0,
            fx_source: String::new(),
            taskbar: TaskbarBadgeConfig::default(),
            auto_update_check: true,
            update_check_hours: 12,
            update_dismissed_version: String::new(),
            update_latest_version: String::new(),
            update_latest_url: String::new(),
            update_latest_notes: String::new(),
            update_checked_at: 0,
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
    /// Forward the caller's protocol unchanged (true passthrough). Requests
    /// using a different protocol still go through conversion with the static
    /// protocol as the fallback.
    Ingress,
}

impl ProtocolStrategyMode {
    pub fn label(self) -> &'static str {
        match self {
            ProtocolStrategyMode::Static => "固定协议",
            ProtocolStrategyMode::ModelPrefix => "按模型前缀",
            ProtocolStrategyMode::Explicit => "由路由指定",
            ProtocolStrategyMode::Ingress => "跟随调用方（直通）",
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
            // Without an ingress hint, fall back to the configured protocol.
            ProtocolStrategyMode::Ingress | ProtocolStrategyMode::Static => self.static_protocol,
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

    /// Resolve the upstream protocol, honouring the "follow caller" mode when
    /// the ingress protocol is known. With this mode a client that speaks the
    /// same protocol as the upstream is forwarded byte-for-byte; only other
    /// ingress protocols are converted.
    pub fn protocol_for_ingress(
        &self,
        upstream_model: &str,
        explicit: Option<Protocol>,
        ingress: Option<Protocol>,
    ) -> Protocol {
        if self.protocol_strategy.mode == ProtocolStrategyMode::Ingress {
            if let Some(ingress) = ingress {
                return ingress;
            }
        }
        self.protocol_for(upstream_model, explicit)
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
    /// Optional arithmetic expression over JSON paths, e.g.
    /// `data.balance / 100` or `data.total - data.used`. When empty the value
    /// at `json_path` is used as-is.
    pub formula: String,
    pub scale: f64,
    pub currency: String,
    /// The endpoint returns the used percentage (0–100) instead of money.
    /// It is converted to a balance with `(100 - used) / 100 * quota_total`.
    pub percent_used: bool,
    /// Total subscription/quota amount in `currency`, used by percent mode and
    /// by the percentage quick converter in the balance editor.
    pub quota_total: f64,
}

impl Default for BalanceConfig {
    fn default() -> Self {
        Self {
            mode: BalanceMode::Unavailable,
            manual_amount: 0.0,
            manual_updated_at: 0,
            url: None,
            json_path: "balance".to_string(),
            formula: String::new(),
            scale: 1.0,
            currency: "USD".to_string(),
            percent_used: false,
            quota_total: 0.0,
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

// ---------------------------------------------------------------------------
// Plugins
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct PluginSettings {
    /// Allow `host.http.*` and `fetch`-style network calls from plugins.
    pub allow_network: bool,
    /// Allow `host.process.run`. Disabled by default because it can execute
    /// arbitrary local programs.
    pub allow_process: bool,
    /// Plugin ids the user disabled. Enabled plugins are loaded on startup.
    pub disabled: Vec<String>,
}

impl Default for PluginSettings {
    fn default() -> Self {
        Self {
            allow_network: true,
            allow_process: false,
            disabled: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Pricing
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PriceCurrency {
    #[default]
    #[serde(alias = "USD", alias = "Usd")]
    Usd,
    #[serde(alias = "CNY", alias = "Cny", alias = "RMB")]
    Cny,
}

impl PriceCurrency {
    pub fn code(self) -> &'static str {
        match self {
            PriceCurrency::Usd => "USD",
            PriceCurrency::Cny => "CNY",
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            PriceCurrency::Usd => "$",
            PriceCurrency::Cny => "¥",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PriceCurrency::Usd => "USD 美元",
            PriceCurrency::Cny => "CNY 人民币",
        }
    }
}

/// One per-million-token price triple.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(default)]
pub struct PriceTier {
    pub input_per_mtok: f64,
    /// Price for prompt tokens served from the provider's cache. `0.0` keeps
    /// cached tokens at the regular input price.
    pub cached_input_per_mtok: f64,
    pub output_per_mtok: f64,
}

impl PriceTier {
    pub fn is_zero(&self) -> bool {
        self.input_per_mtok == 0.0
            && self.cached_input_per_mtok == 0.0
            && self.output_per_mtok == 0.0
    }

    /// Cost in millions-of-currency units (i.e. the base unit) for a request.
    pub fn amount(&self, prompt_tokens: u64, cached_tokens: u64, completion_tokens: u64) -> f64 {
        let cached = cached_tokens.min(prompt_tokens);
        let uncached = prompt_tokens.saturating_sub(cached);
        let cached_rate = if self.cached_input_per_mtok > 0.0 {
            self.cached_input_per_mtok
        } else {
            self.input_per_mtok
        };
        (uncached as f64 * self.input_per_mtok
            + cached as f64 * cached_rate
            + completion_tokens as f64 * self.output_per_mtok)
            / 1_000_000.0
    }
}

/// A time-of-day price window. Peak/off-peak tariffs are expressed by giving
/// each window its own price triple; hours outside every window use the base
/// tier.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct PricePeriod {
    pub name: String,
    /// Inclusive local start hour, 0–23.
    pub start_hour: u8,
    /// Exclusive local end hour, 0–24. A smaller end hour wraps past midnight.
    pub end_hour: u8,
    /// Weekdays the window applies to, 0 = Monday … 6 = Sunday. Empty means
    /// every day.
    pub days: Vec<u8>,
    pub tier: PriceTier,
}

impl Default for PricePeriod {
    fn default() -> Self {
        Self {
            name: "高峰".to_string(),
            start_hour: 9,
            end_hour: 18,
            days: Vec::new(),
            tier: PriceTier::default(),
        }
    }
}

impl PricePeriod {
    pub fn normalized(&self) -> Self {
        let mut copy = self.clone();
        copy.name = if copy.name.trim().is_empty() {
            "时段".to_string()
        } else {
            copy.name.trim().to_string()
        };
        copy.start_hour = copy.start_hour.min(23);
        copy.end_hour = copy.end_hour.min(24);
        copy.days.retain(|d| *d < 7);
        copy.days.sort_unstable();
        copy.days.dedup();
        copy.tier.input_per_mtok = sanitize_price(copy.tier.input_per_mtok);
        copy.tier.cached_input_per_mtok = sanitize_price(copy.tier.cached_input_per_mtok);
        copy.tier.output_per_mtok = sanitize_price(copy.tier.output_per_mtok);
        copy
    }

    /// Does this window cover the given local time?
    pub fn matches(&self, at: chrono::DateTime<chrono::Local>) -> bool {
        use chrono::{Datelike, Timelike};
        if !self.days.is_empty()
            && !self
                .days
                .contains(&(at.weekday().num_days_from_monday() as u8))
        {
            return false;
        }
        let hour = at.hour() as u8;
        if self.start_hour == self.end_hour {
            // A zero-length window is treated as a full day.
            return true;
        }
        if self.start_hour < self.end_hour {
            hour >= self.start_hour && hour < self.end_hour
        } else {
            hour >= self.start_hour || hour < self.end_hour
        }
    }

    /// `09:00–18:00` style label, plus weekday hints.
    pub fn schedule_label(&self) -> String {
        let hours = if self.start_hour == self.end_hour {
            "全天".to_string()
        } else {
            format!("{:02}:00–{:02}:00", self.start_hour, self.end_hour)
        };
        if self.days.is_empty() {
            hours
        } else {
            format!("{hours} · {}", weekday_label(&self.days))
        }
    }
}

pub fn weekday_label(days: &[u8]) -> String {
    const NAMES: [&str; 7] = ["一", "二", "三", "四", "五", "六", "日"];
    if days.is_empty() {
        return "每天".to_string();
    }
    if days.len() == 7 {
        return "每天".to_string();
    }
    days.iter()
        .filter(|d| **d < 7)
        .map(|d| format!("周{}", NAMES[*d as usize]))
        .collect::<Vec<_>>()
        .join("、")
}

fn sanitize_price(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1_000_000.0)
    } else {
        0.0
    }
}

/// A reusable price table that can be shared between routes and exported.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct PricePlan {
    pub name: String,
    pub note: String,
    pub currency: PriceCurrency,
    pub base: PriceTier,
    pub periods: Vec<PricePeriod>,
}

impl Default for PricePlan {
    fn default() -> Self {
        Self {
            name: String::new(),
            note: String::new(),
            currency: PriceCurrency::Usd,
            base: PriceTier::default(),
            periods: Vec::new(),
        }
    }
}

impl PricePlan {
    pub fn normalized(&self) -> Self {
        let mut copy = self.clone();
        copy.name = copy.name.trim().to_string();
        copy.note = copy.note.trim().to_string();
        copy.periods = copy.periods.iter().map(PricePeriod::normalized).collect();
        copy
    }
}

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
    /// Legacy USD/Mtok fields. Kept so v3 configurations load unchanged;
    /// `pricing.base` is the source of truth going forward.
    pub input_price_per_mtok: f64,
    pub output_price_per_mtok: f64,
    #[serde(default)]
    pub pricing: RoutePricing,
    pub enabled: bool,
    pub created_at: i64,
}

/// How a route is billed: a currency plus a base tier and optional
/// time-of-day windows (peak / off-peak tariffs).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct RoutePricing {
    pub currency: PriceCurrency,
    pub base: PriceTier,
    pub periods: Vec<PricePeriod>,
}

impl RoutePricing {
    pub fn normalized(&self) -> Self {
        Self {
            currency: self.currency,
            base: PriceTier {
                input_per_mtok: sanitize_price(self.base.input_per_mtok),
                cached_input_per_mtok: sanitize_price(self.base.cached_input_per_mtok),
                output_per_mtok: sanitize_price(self.base.output_per_mtok),
            },
            periods: self.periods.iter().map(PricePeriod::normalized).collect(),
        }
    }

    pub fn is_zero(&self) -> bool {
        self.base.is_zero() && self.periods.iter().all(|period| period.tier.is_zero())
    }

    /// The tier that applies at `at`, falling back to the base tier.
    pub fn tier_at(&self, at: chrono::DateTime<chrono::Local>) -> PriceTier {
        self.periods
            .iter()
            .find(|period| period.matches(at))
            .map(|period| period.tier)
            .unwrap_or(self.base)
    }

    pub fn amount_at(
        &self,
        prompt_tokens: u64,
        cached_tokens: u64,
        completion_tokens: u64,
        at: chrono::DateTime<chrono::Local>,
    ) -> f64 {
        self.tier_at(at)
            .amount(prompt_tokens, cached_tokens, completion_tokens)
    }
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
            pricing: RoutePricing::default(),
            enabled: true,
            created_at: 0,
        }
    }
}

impl ModelRoute {
    /// Pricing with legacy USD fields folded in. v3 configurations only stored
    /// a flat input/output pair, so an empty `pricing` block inherits them.
    pub fn effective_pricing(&self) -> RoutePricing {
        if !self.pricing.is_zero()
            || (self.input_price_per_mtok == 0.0 && self.output_price_per_mtok == 0.0)
        {
            return self.pricing.clone();
        }
        RoutePricing {
            currency: PriceCurrency::Usd,
            base: PriceTier {
                input_per_mtok: self.input_price_per_mtok,
                cached_input_per_mtok: 0.0,
                output_per_mtok: self.output_price_per_mtok,
            },
            periods: Vec::new(),
        }
    }

    /// Cost in micro-USD, using the base tier only. Period-aware billing goes
    /// through [`Self::cost_micros_at`].
    #[allow(dead_code)]
    pub fn cost_micros(&self, prompt_tokens: u64, completion_tokens: u64) -> u64 {
        self.cost_micros_at(prompt_tokens, 0, completion_tokens, chrono::Local::now())
    }

    /// Cost in micro-USD for a request that happened at `at`.
    ///
    /// Prices may be quoted in CNY; `currency_per_usd` is the route currency's
    /// units per 1 USD, so every stored cost stays USD-denominated and totals
    /// remain summable across providers.
    #[allow(dead_code)]
    pub fn cost_micros_at(
        &self,
        prompt_tokens: u64,
        cached_tokens: u64,
        completion_tokens: u64,
        at: chrono::DateTime<chrono::Local>,
    ) -> u64 {
        let pricing = self.effective_pricing();
        let amount = pricing.amount_at(prompt_tokens, cached_tokens, completion_tokens, at);
        let usd = match pricing.currency {
            PriceCurrency::Usd => amount,
            PriceCurrency::Cny => amount / 7.2,
        };
        (usd * 1_000_000.0).round().max(0.0) as u64
    }

    /// Same as [`Self::cost_micros_at`] but with an explicit FX rate supplied
    /// by the live rate table.
    pub fn cost_micros_with_fx(
        &self,
        prompt_tokens: u64,
        cached_tokens: u64,
        completion_tokens: u64,
        at: chrono::DateTime<chrono::Local>,
        units_per_usd: f64,
    ) -> u64 {
        let pricing = self.effective_pricing();
        let amount = pricing.amount_at(prompt_tokens, cached_tokens, completion_tokens, at);
        let rate = if units_per_usd.is_finite() && units_per_usd > 0.0 {
            units_per_usd
        } else {
            1.0
        };
        ((amount / rate).max(0.0) * 1_000_000.0).round() as u64
    }
}

// ---------------------------------------------------------------------------
// Client-facing model metadata
// ---------------------------------------------------------------------------

/// Metadata advertised by `GET /v1/models` for one client model.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ClientModelProfile {
    pub client_model: String,
    pub display_name: String,
    pub description: String,
    pub owned_by: String,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    /// text / image / audio / file
    pub input_modalities: Vec<String>,
    pub output_modalities: Vec<String>,
    /// Free-form effort names such as off, low, medium, high.
    pub reasoning_levels: Vec<String>,
    pub supports_tools: bool,
    pub supports_streaming: bool,
    pub fallback_client_model: Option<String>,
    pub enabled: bool,
    pub created_at: i64,
}

impl Default for ClientModelProfile {
    fn default() -> Self {
        Self {
            client_model: String::new(),
            display_name: String::new(),
            description: String::new(),
            owned_by: "yoyo-relay".to_string(),
            context_window: None,
            max_output_tokens: None,
            input_modalities: vec!["text".to_string()],
            output_modalities: vec!["text".to_string()],
            reasoning_levels: Vec::new(),
            supports_tools: true,
            supports_streaming: true,
            fallback_client_model: None,
            enabled: true,
            created_at: 0,
        }
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

pub fn normalize_currency(currency: &str) -> String {
    let value = currency.trim().to_ascii_uppercase();
    match value.as_str() {
        "" => "USD".to_string(),
        "RMB" => "CNY".to_string(),
        _ => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32, weekday_offset: i64) -> chrono::DateTime<chrono::Local> {
        // 2026-10-19 is a Monday.
        let base = chrono::Local
            .with_ymd_and_hms(2026, 10, 19, hour, 30, 0)
            .single()
            .unwrap();
        base + chrono::Duration::days(weekday_offset)
    }

    fn tier(input: f64, cached: f64, output: f64) -> PriceTier {
        PriceTier {
            input_per_mtok: input,
            cached_input_per_mtok: cached,
            output_per_mtok: output,
        }
    }

    #[test]
    fn price_tier_bills_cached_tokens_at_cached_rate() {
        let price = tier(3.0, 0.3, 15.0);
        // 1M prompt tokens of which 400k hit the cache, plus 100k output.
        let amount = price.amount(1_000_000, 400_000, 100_000);
        let expected = 0.6 * 3.0 + 0.4 * 0.3 + 0.1 * 15.0;
        assert!((amount - expected).abs() < 1e-9, "{amount} != {expected}");
    }

    #[test]
    fn cached_rate_of_zero_falls_back_to_input_rate() {
        let price = tier(2.0, 0.0, 4.0);
        assert!((price.amount(1_000_000, 500_000, 0) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn peak_period_overrides_base_price() {
        let pricing = RoutePricing {
            currency: PriceCurrency::Cny,
            base: tier(1.0, 0.0, 2.0),
            periods: vec![PricePeriod {
                name: "峰时".into(),
                start_hour: 9,
                end_hour: 18,
                days: Vec::new(),
                tier: tier(4.0, 0.0, 8.0),
            }],
        };
        let peak = pricing.amount_at(1_000_000, 0, 0, at(10, 0));
        let off_peak = pricing.amount_at(1_000_000, 0, 0, at(20, 0));
        assert!((peak - 4.0).abs() < 1e-9);
        assert!((off_peak - 1.0).abs() < 1e-9);
    }

    #[test]
    fn weekday_filter_and_midnight_wrap() {
        let period = PricePeriod {
            name: "夜间".into(),
            start_hour: 22,
            end_hour: 6,
            days: vec![0, 1], // Monday, Tuesday
            tier: tier(1.0, 0.0, 1.0),
        };
        assert!(period.matches(at(23, 0)), "Monday 23:00 should match");
        assert!(period.matches(at(3, 0)), "Monday 03:00 should match");
        assert!(!period.matches(at(12, 0)), "Monday noon should not match");
        assert!(
            !period.matches(at(23, 2)),
            "Wednesday 23:00 is outside the weekday list"
        );
    }

    #[test]
    fn legacy_usd_fields_are_folded_into_pricing() {
        let route = ModelRoute {
            input_price_per_mtok: 2.5,
            output_price_per_mtok: 10.0,
            ..ModelRoute::default()
        };
        let effective = route.effective_pricing();
        assert_eq!(effective.currency, PriceCurrency::Usd);
        assert!((effective.base.input_per_mtok - 2.5).abs() < 1e-9);
        assert!((effective.base.output_per_mtok - 10.0).abs() < 1e-9);
        // 1M input + 1M output at those rates = $12.50 = 12_500_000 micro-USD.
        assert_eq!(route.cost_micros(1_000_000, 1_000_000), 12_500_000);
    }

    #[test]
    fn explicit_pricing_wins_over_legacy_fields() {
        let route = ModelRoute {
            input_price_per_mtok: 99.0,
            output_price_per_mtok: 99.0,
            pricing: RoutePricing {
                currency: PriceCurrency::Usd,
                base: tier(1.0, 0.0, 1.0),
                periods: Vec::new(),
            },
            ..ModelRoute::default()
        };
        assert!((route.effective_pricing().base.input_per_mtok - 1.0).abs() < 1e-9);
    }

    #[test]
    fn cny_pricing_is_converted_to_usd_micros() {
        let route = ModelRoute {
            pricing: RoutePricing {
                currency: PriceCurrency::Cny,
                base: tier(7.2, 0.0, 7.2),
                periods: Vec::new(),
            },
            ..ModelRoute::default()
        };
        // 7.2 CNY per Mtok is 1 USD per Mtok with the live-rate fallback.
        let micros = route.cost_micros_with_fx(1_000_000, 0, 1_000_000, at(12, 0), 7.2);
        assert_eq!(micros, 2_000_000);
    }

    #[test]
    fn period_normalization_clamps_and_drops_empty_tiers() {
        let period = PricePeriod {
            name: "  ".into(),
            start_hour: 30,
            end_hour: 40,
            days: vec![9, 1, 1, 2],
            tier: PriceTier::default(),
        };
        let normalized = period.normalized();
        assert_eq!(normalized.name, "时段");
        assert_eq!(normalized.start_hour, 23);
        assert_eq!(normalized.end_hour, 24);
        assert_eq!(normalized.days, vec![1, 2]);
    }

    #[test]
    fn badge_config_reports_content_with_speed_only() {
        let config = TaskbarBadgeConfig {
            show_status: false,
            show_active: false,
            show_tokens: false,
            show_requests: false,
            show_queued: false,
            show_success: false,
            show_cost: false,
            show_speed: true,
            ..TaskbarBadgeConfig::default()
        };
        assert!(config.has_content());
        assert!(!config.transparent_bg);
    }
}
