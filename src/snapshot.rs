use std::collections::{HashMap, HashSet};

use crate::db::{RecentRequest, SeriesPoint, Totals};
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

#[derive(Clone, Debug)]
pub struct UsageView {
    pub days: u32,
    pub dimension: UsageDimension,
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
            totals: Totals::default(),
            groups: Vec::new(),
            model_pairs: Vec::new(),
            series: Vec::new(),
            range_start: 0,
            bucket_secs: 86_400,
        }
    }
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct UiSnapshot {
    pub proxy: ProxyStatus,
    pub endpoint: String,
    pub uptime_secs: i64,
    pub client_models: Vec<String>,
    pub today: TodayView,
    pub breaker: BreakerSnapshot,
    pub active_requests: usize,
    pub queued_requests: usize,
    pub providers: Vec<ProviderHealth>,
    pub accounts: Vec<AccountView>,
    pub route_groups: Vec<RouteGroupView>,
    pub local_keys: Vec<LocalKeyView>,
    pub balances: Vec<BalanceView>,
    pub usage: UsageView,
    pub recent: Vec<RecentRequest>,
    pub tests: HashMap<String, TestResult>,
    pub running_tests: HashSet<String>,
    pub notices: Vec<Notice>,
    pub alerts: Vec<Alert>,
    pub warnings: Vec<String>,
    pub balance_refreshing: bool,
    pub updated_at: i64,
}

impl Default for UiSnapshot {
    fn default() -> Self {
        Self {
            proxy: ProxyStatus::Stopped,
            endpoint: String::new(),
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
            providers: Vec::new(),
            accounts: Vec::new(),
            route_groups: Vec::new(),
            local_keys: Vec::new(),
            balances: Vec::new(),
            usage: UsageView::default(),
            recent: Vec::new(),
            tests: HashMap::new(),
            running_tests: HashSet::new(),
            notices: Vec::new(),
            alerts: Vec::new(),
            warnings: Vec::new(),
            balance_refreshing: false,
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
}
