use std::collections::{BTreeMap, HashMap, HashSet};

use crate::db::{RecentRequest, SeriesPoint, Totals, UsageDetail, UsageFilter};
use crate::plugins::{PluginInfo, PluginUiRequest};
use crate::probe::TestResult;
use crate::scheduler::{AccountState, BreakerSnapshot, QuotaSnapshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProxyStatus {
    Stopped,
    Starting,
    Running,
    Error(String),
}

impl ProxyStatus {
    pub fn label(&self) -> &'static str {
        match self {
            ProxyStatus::Stopped => "已停止",
            ProxyStatus::Starting => "启动中",
            ProxyStatus::Running => "运行中",
            ProxyStatus::Error(_) => "异常",
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self, ProxyStatus::Running | ProxyStatus::Starting)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum UsageDimension {
    Global,
    Provider,
    Account,
    Model,
}

impl UsageDimension {
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            UsageDimension::Global => "全局",
            UsageDimension::Provider => "供应商",
            UsageDimension::Account => "账号",
            UsageDimension::Model => "客户端模型",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            UsageDimension::Global => "global",
            UsageDimension::Provider => "provider",
            UsageDimension::Account => "account",
            UsageDimension::Model => "model",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Notice {
    pub id: u64,
    pub level: NoticeLevel,
    pub text: String,
    pub at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AlertLevel {
    Info,
    Warning,
    Critical,
}

#[derive(Clone, Debug)]
pub struct Alert {
    pub level: AlertLevel,
    pub text: String,
}

/// One client-facing base URL, local or LAN.
#[derive(Clone, Debug)]
pub struct EndpointView {
    pub label: String,
    pub url: String,
    /// True for addresses other devices can reach.
    pub lan: bool,
}

/// Cached release lookup result shown in the title bar and update modal.
#[derive(Clone, Debug, Default)]
pub struct UpdateView {
    pub current: String,
    pub latest: String,
    pub url: String,
    pub notes: String,
    pub checked_at: i64,
    pub available: bool,
    pub dismissed: bool,
    pub checking: bool,
    pub error: Option<String>,
    /// Self-update download/install progress.
    pub installing: bool,
    pub cancelling: bool,
    pub progress: f32,
    pub install_message: String,
    pub install_error: Option<String>,
    pub restart_pending: bool,
}

#[derive(Clone, Debug, Default)]
pub struct TodayView {
    pub totals: Totals,
    pub series: Vec<SeriesPoint>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CurrencyTotal {
    pub currency: String,
    pub amount: f64,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ProviderHealth {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub account_count: usize,
    pub healthy: usize,
    pub cooling: usize,
    pub exhausted: usize,
    pub disabled: usize,
    pub half_open: usize,
    /// Balances are never summed across currencies.
    pub balances: Vec<CurrencyTotal>,
}

impl ProviderHealth {
    pub fn all_unavailable(&self) -> bool {
        self.account_count > 0 && self.healthy == 0 && self.half_open == 0
    }

    pub fn balance_known(&self) -> bool {
        !self.balances.is_empty()
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct BalanceView {
    pub account_id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub account_name: String,
    pub mode: String,
    pub amount: Option<f64>,
    pub currency: String,
    pub estimated: bool,
    pub updated_at: i64,
    pub note: String,
    pub status_text: String,
    pub quota_preset: String,
    pub quota_note: String,
    pub quota_windows: Vec<QuotaWindowView>,
}

#[derive(Clone, Debug, Default)]
pub struct QuotaWindowView {
    pub label: String,
    pub used: f64,
    pub limit: f64,
    /// `money` or `percent`.
    pub unit: String,
    pub currency: String,
    pub reset_at: i64,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct AccountView {
    pub id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub name: String,
    pub enabled: bool,
    pub priority: i32,
    pub masked_key: String,
    pub state: AccountState,
    pub fail_streak: u32,
    pub inflight: usize,
    pub max_concurrency: usize,
    pub quota: Option<QuotaSnapshot>,
    pub balance: Option<f64>,
    pub balance_currency: String,
    pub balance_estimated: bool,
    pub balance_source: String,
    pub balance_updated_at: i64,
    pub today_tokens: u64,
    pub today_requests: u64,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct LocalKeyView {
    pub id: String,
    pub name: String,
    pub masked: String,
    pub enabled: bool,
    pub allowed_models: Vec<String>,
    pub max_tokens: Option<u64>,
    pub max_requests: Option<u64>,
    pub used_tokens: u64,
    pub used_requests: u64,
    pub created_at: i64,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct RouteRowView {
    pub route_id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub client_model: String,
    pub upstream_model: String,
    pub protocol: String,
    pub mode: String,
    pub priority: i32,
    pub accounts_total: usize,
    pub accounts_available: usize,
    pub warning: Option<String>,
    pub enabled: bool,
    pub fallback: bool,
    /// Price summary such as `¥ 2.00 / 8.00 每 Mtok`; empty when unpriced.
    pub price_label: String,
    /// Number of peak/off-peak windows, 0 when the route uses a flat price.
    pub price_periods: usize,
}

#[derive(Clone, Debug)]
pub struct RouteGroupView {
    pub client_model: String,
    pub rows: Vec<RouteRowView>,
    pub fallback_model: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct UsageGroupView {
    pub key: String,
    pub label: String,
    pub totals: Totals,
}

/// One page of the call-detail table.
#[derive(Clone, Debug, Default)]
pub struct CallsView {
    pub rows: Vec<UsageDetail>,
    pub total: u64,
    pub page: usize,
    pub page_size: usize,
    pub filter: UsageFilter,
}

#[derive(Clone, Debug)]
pub struct UsageView {
    pub days: u32,
    pub dimension: UsageDimension,
    /// Optional local-key filter (`None` means every key plus unauthenticated).
    pub local_key: Option<String>,
    pub totals: Totals,
    pub groups: Vec<UsageGroupView>,
    pub model_pairs: Vec<(String, String, Totals)>,
    pub series: Vec<SeriesPoint>,
    pub range_start: i64,
    pub bucket_secs: i64,
}

impl Default for UsageView {
    fn default() -> Self {
        Self {
            days: 7,
            dimension: UsageDimension::Global,
            local_key: None,
            totals: Totals::default(),
            groups: Vec::new(),
            model_pairs: Vec::new(),
            series: Vec::new(),
            range_start: 0,
            bucket_secs: 86_400,
        }
    }
}

/// Cached `/models` catalogue for one provider, fetched on demand by the route
/// editor so upstream model names can be picked instead of typed.
#[derive(Clone, Debug, Default)]
pub struct ProviderModelsView {
    pub provider_id: String,
    pub provider_name: String,
    pub models: Vec<String>,
    pub ok: bool,
    pub message: String,
    pub at: i64,
}

/// Provider contributed by a running (or discovered) plugin. These are never
/// written to the user's config; the UI shows them with a "临时" badge and the
/// route editor can select them.
#[derive(Clone, Debug, Default)]
pub struct TemporaryProvider {
    pub id: String,
    pub name: String,
    pub description: String,
    pub plugin_id: String,
    pub plugin_name: String,
    pub protocol: String,
    pub models: Vec<String>,
    pub streaming: bool,
    pub running: bool,
}

/// Result of a natural-language price-plan generation. The UI only applies a
/// plan to the route that requested it.
#[derive(Clone, Debug, Default)]
pub struct AiPricingView {
    pub route_id: String,
    /// Monotonic request id; stale background results are ignored.
    pub token: u64,
    pub running: bool,
    pub plan: Option<crate::config::PricePlan>,
    pub error: Option<String>,
}

/// Result of a natural-language provider generation.
#[derive(Clone, Debug, Default)]
pub struct AiProviderView {
    /// Monotonic request id; stale background results are ignored.
    pub token: u64,
    pub running: bool,
    pub provider: Option<crate::config::Provider>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct FetchedModelView {
    pub id: u64,
    pub client_model: String,
    pub provider_name: String,
    pub upstream_model: String,
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

#[derive(Clone)]
#[allow(dead_code)]
pub struct UiSnapshot {
    pub proxy: ProxyStatus,
    pub endpoint: String,
    /// Local plus LAN base URLs. The first entry is always loopback.
    pub endpoints: Vec<EndpointView>,
    /// The listener is bound to all interfaces (0.0.0.0 / ::).
    pub lan_enabled: bool,
    pub uptime_secs: i64,
    pub client_models: Vec<String>,
    pub today: TodayView,
    pub breaker: BreakerSnapshot,
    pub active_requests: usize,
    pub queued_requests: usize,
    /// Live completion throughput over the last few seconds.
    pub tokens_per_sec: f64,
    pub update: UpdateView,
    pub plugins: Vec<PluginInfo>,
    pub plugin_ui: Option<PluginUiRequest>,
    pub plugin_console: Option<crate::plugins::PluginConsoleView>,
    /// GitHub contributors, loaded on demand by the About page.
    pub contributors: Vec<crate::contributors::Contributor>,
    pub contributors_loading: bool,
    pub contributors_error: Option<String>,
    pub plugin_settings: crate::config::PluginSettings,
    pub providers: Vec<ProviderHealth>,
    pub accounts: Vec<AccountView>,
    pub route_groups: Vec<RouteGroupView>,
    pub local_keys: Vec<LocalKeyView>,
    pub balances: Vec<BalanceView>,
    pub usage: UsageView,
    pub calls: CallsView,
    pub recent: Vec<RecentRequest>,
    pub tests: HashMap<String, TestResult>,
    pub running_tests: HashSet<String>,
    pub notices: Vec<Notice>,
    pub alerts: Vec<Alert>,
    pub warnings: Vec<String>,
    pub balance_refreshing: bool,
    pub fetched_model: Option<FetchedModelView>,
    pub model_fetch_running: bool,
    /// Background natural-language pricing generation.
    pub ai_pricing: Option<AiPricingView>,
    /// Background natural-language provider creation.
    pub ai_provider: Option<AiProviderView>,
    /// Latest `/models` catalogue per provider id.
    pub provider_models: HashMap<String, ProviderModelsView>,
    pub models_fetching: HashSet<String>,
    /// Live USD-based rate table plus refresh metadata.
    pub fx_rates: BTreeMap<String, f64>,
    pub fx_updated_at: i64,
    pub fx_source: String,
    pub fx_refreshing: bool,
    pub updated_at: i64,
}

impl Default for UiSnapshot {
    fn default() -> Self {
        Self {
            proxy: ProxyStatus::Stopped,
            endpoint: String::new(),
            endpoints: Vec::new(),
            lan_enabled: false,
            uptime_secs: 0,
            client_models: Vec::new(),
            today: TodayView::default(),
            breaker: BreakerSnapshot {
                open: false,
                open_until: 0,
                consecutive_failures: 0,
                threshold: 20,
            },
            active_requests: 0,
            queued_requests: 0,
            tokens_per_sec: 0.0,
            update: UpdateView::default(),
            plugins: Vec::new(),
            plugin_ui: None,
            plugin_console: None,
            contributors: Vec::new(),
            contributors_loading: false,
            contributors_error: None,
            plugin_settings: crate::config::PluginSettings::default(),
            providers: Vec::new(),
            accounts: Vec::new(),
            route_groups: Vec::new(),
            local_keys: Vec::new(),
            balances: Vec::new(),
            usage: UsageView::default(),
            calls: CallsView::default(),
            recent: Vec::new(),
            tests: HashMap::new(),
            running_tests: HashSet::new(),
            notices: Vec::new(),
            alerts: Vec::new(),
            warnings: Vec::new(),
            balance_refreshing: false,
            fetched_model: None,
            model_fetch_running: false,
            ai_pricing: None,
            ai_provider: None,
            provider_models: HashMap::new(),
            models_fetching: HashSet::new(),
            fx_rates: BTreeMap::new(),
            fx_updated_at: 0,
            fx_source: String::new(),
            fx_refreshing: false,
            updated_at: 0,
        }
    }
}

impl UiSnapshot {
    pub fn account(&self, id: &str) -> Option<&AccountView> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn provider(&self, id: &str) -> Option<&ProviderHealth> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// Providers injected by plugins. The id mirrors
    /// `plugin:<plugin-id>:<provider-index>` from the plugin host.
    pub fn temporary_providers(&self) -> Vec<TemporaryProvider> {
        let mut out = Vec::new();
        for plugin in &self.plugins {
            for (index, spec) in plugin.providers.iter().enumerate() {
                if spec.models.is_empty() {
                    continue;
                }
                out.push(TemporaryProvider {
                    id: format!("plugin:{}:{index}", plugin.id),
                    name: if spec.name.trim().is_empty() {
                        format!("{} · 插件供应商", plugin.name)
                    } else {
                        spec.name.clone()
                    },
                    description: spec.description.clone(),
                    plugin_id: plugin.id.clone(),
                    plugin_name: plugin.name.clone(),
                    protocol: spec.protocol.clone(),
                    models: spec.models.clone(),
                    streaming: spec.streaming,
                    running: plugin.status == crate::plugins::PluginStatus::Running,
                });
            }
        }
        out
    }
}
