use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use futures_util::FutureExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::balance;
use crate::config::{now_ts, AppConfig, BalanceMode, ConfigStore};
use crate::db::{self, Db};
use crate::gateway::Gateway;
use crate::probe::{self, TestLevel, TestResult};
use crate::scheduler::{AccountState as SchedState, Scheduler};
use crate::snapshot::*;

// ---------------------------------------------------------------------------
// UI commands
// ---------------------------------------------------------------------------

pub enum UiCommand {
    StartProxy,
    StopProxy,
    RestartProxy,
    SaveConfig(Box<AppConfig>),
    SetUsageView {
        days: u32,
        dimension: UsageDimension,
    },
    TestProvider(String),
    TestAccount(String),
    TestAccountDraft {
        provider_id: String,
        name: String,
        api_key: String,
        model_hint: Option<String>,
    },
    TestRoute(String),
    TestLocalKey(String),
    RefreshBalance(Option<String>),
    CleanupNow,
    Shutdown,
}

pub fn spawn(
    store: Arc<ConfigStore>,
    config: AppConfig,
    warnings: Vec<String>,
) -> Result<(
    mpsc::UnboundedSender<UiCommand>,
    std::sync::mpsc::Receiver<Arc<UiSnapshot>>,
)> {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (snap_tx, snap_rx) = std::sync::mpsc::channel();
    let thread_store = store;
    std::thread::Builder::new()
        .name("yoyo-runtime".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .thread_name("yoyo-worker")
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    eprintln!("无法创建运行时：{err}");
                    return;
                }
            };
            rt.block_on(async move {
                match Runtime::new(thread_store, config, warnings, cmd_rx, snap_tx).await {
                    Ok(runtime) => runtime.run().await,
                    Err(err) => eprintln!("运行时初始化失败：{err:#}"),
                }
            });
        })?;
    Ok((cmd_tx, snap_rx))
}

// ---------------------------------------------------------------------------
// Shared mutable state
// ---------------------------------------------------------------------------

struct Shared {
    proxy: Mutex<ProxyStatus>,
    tests: Mutex<HashMap<String, TestResult>>,
    running_tests: Mutex<HashSet<String>>,
    notices: Mutex<VecDeque<Notice>>,
    next_notice: AtomicU64,
    balance_refreshing: AtomicBool,
    warnings: Mutex<Vec<String>>,
}

impl Shared {
    fn new(warnings: Vec<String>) -> Self {
        Self {
            proxy: Mutex::new(ProxyStatus::Stopped),
            tests: Mutex::new(HashMap::new()),
            running_tests: Mutex::new(HashSet::new()),
            notices: Mutex::new(VecDeque::new()),
            next_notice: AtomicU64::new(1),
            balance_refreshing: AtomicBool::new(false),
            warnings: Mutex::new(warnings),
        }
    }

    fn proxy(&self) -> ProxyStatus {
        self.proxy
            .lock()
            .map(|g| g.clone())
            .unwrap_or(ProxyStatus::Stopped)
    }

    fn set_proxy(&self, status: ProxyStatus) {
        if let Ok(mut guard) = self.proxy.lock() {
            *guard = status;
        }
    }

    fn push_notice(&self, level: NoticeLevel, text: impl Into<String>) {
        let id = self.next_notice.fetch_add(1, Ordering::SeqCst);
        let notice = Notice {
            id,
            level,
            text: text.into(),
            at: now_ts(),
        };
        if let Ok(mut notices) = self.notices.lock() {
            notices.push_back(notice);
            while notices.len() > 30 {
                notices.pop_front();
            }
        }
    }

    fn insert_test(&self, result: TestResult) {
        let level = match result.level {
            TestLevel::Success => NoticeLevel::Success,
            TestLevel::Warning | TestLevel::RateLimited => NoticeLevel::Warning,
            _ => NoticeLevel::Error,
        };
        let text = format!(
            "测试 {}：{} — {}",
            result.title,
            result.level.label(),
            result.message
        );
        self.push_notice(level, text);
        if let Ok(mut tests) = self.tests.lock() {
            if tests.len() > 40 {
                if let Some(oldest) = tests
                    .iter()
                    .min_by_key(|(_, v)| v.at)
                    .map(|(k, _)| k.clone())
                {
                    tests.remove(&oldest);
                }
            }
            tests.insert(result.key.clone(), result);
        }
    }

    fn set_running(&self, key: &str, running: bool) {
        if let Ok(mut set) = self.running_tests.lock() {
            if running {
                set.insert(key.to_string());
            } else {
                set.remove(key);
            }
        }
    }

    fn is_running(&self, key: &str) -> bool {
        self.running_tests
            .lock()
            .map(|s| s.contains(key))
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// Proxy supervisor
// ---------------------------------------------------------------------------

struct ProxyHandle {
    cancel: CancellationToken,
    join: tokio::task::JoinHandle<()>,
}

struct UsageParams {
    days: u32,
    dimension: UsageDimension,
    last_built: Option<Instant>,
    view: UsageView,
}

pub struct Runtime {
    store: Arc<ConfigStore>,
    gateway: Arc<Gateway>,
    scheduler: Arc<Scheduler>,
    db: Arc<Db>,
    shared: Arc<Shared>,
    cmd_rx: mpsc::UnboundedReceiver<UiCommand>,
    snapshot_tx: std::sync::mpsc::Sender<Arc<UiSnapshot>>,
    proxy: Option<ProxyHandle>,
    usage: UsageParams,
    last_probe_check: Instant,
    last_balance_refresh: Instant,
    last_cleanup: Instant,
}

impl Runtime {
    async fn new(
        store: Arc<ConfigStore>,
        config: AppConfig,
        warnings: Vec<String>,
        cmd_rx: mpsc::UnboundedReceiver<UiCommand>,
        snapshot_tx: std::sync::mpsc::Sender<Arc<UiSnapshot>>,
    ) -> Result<Self> {
        let db = Arc::new(Db::open(&store.paths.db_file)?);
        let scheduler = Arc::new(Scheduler::new(db.clone()));
        for account in &config.accounts {
            scheduler.runtime_for(&account.id);
        }
        let gateway = Gateway::new(
            Arc::new(config.clone()),
            store.master().clone(),
            db.clone(),
            scheduler.clone(),
        );
        let shared = Arc::new(Shared::new(warnings));
        Ok(Self {
            store,
            gateway,
            scheduler,
            db,
            shared,
            cmd_rx,
            snapshot_tx,
            proxy: None,
            usage: UsageParams {
                days: 7,
                dimension: UsageDimension::Global,
                last_built: None,
                view: UsageView::default(),
            },
            last_probe_check: Instant::now(),
            last_balance_refresh: Instant::now() - Duration::from_secs(3600),
            last_cleanup: Instant::now() - Duration::from_secs(6 * 3600),
        })
    }

    async fn run(mut self) {
        self.start_proxy().await;
        self.emit_snapshot();
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                cmd = self.cmd_rx.recv() => {
                    match cmd {
                        Some(UiCommand::Shutdown) | None => break,
                        Some(cmd) => {
                            let keep_running = self.handle_command(cmd).await;
                            if !keep_running {
                                break;
                            }
                            self.emit_snapshot();
                        }
                    }
                }
                _ = tick.tick() => {
                    self.periodic().await;
                    if !self.emit_snapshot() {
                        break;
                    }
                }
            }
        }
        self.stop_proxy().await;
    }

    fn emit_snapshot(&mut self) -> bool {
        let snapshot = self.build_snapshot();
        self.snapshot_tx.send(snapshot).is_ok()
    }

    async fn handle_command(&mut self, cmd: UiCommand) -> bool {
        match cmd {
            UiCommand::StartProxy => {
                let errored = matches!(self.shared.proxy(), ProxyStatus::Error(_));
                if self.proxy.is_some() && !errored {
                    self.shared
                        .push_notice(NoticeLevel::Info, "代理已在运行，无需重复启动。");
                } else {
                    if self.proxy.is_some() {
                        self.stop_proxy().await;
                    }
                    self.start_proxy().await;
                }
            }
            UiCommand::StopProxy => {
                self.stop_proxy().await;
                self.shared.push_notice(NoticeLevel::Info, "代理已停止。");
            }
            UiCommand::RestartProxy => {
                self.stop_proxy().await;
                self.start_proxy().await;
            }
            UiCommand::SaveConfig(cfg) => {
                let old = self.gateway.config();
                let new = Arc::new(*cfg);
                let restart = old.server.bind != new.server.bind
                    || old.server.port != new.server.port
                    || old.server.cors != new.server.cors;
                // Reset state for accounts whose credentials or enabled flag changed.
                for account in &new.accounts {
                    if let Some(previous) = old.account(&account.id) {
                        if previous.api_key != account.api_key
                            || (!previous.enabled && account.enabled)
                        {
                            self.scheduler.reset_account(&account.id);
                        }
                    }
                }
                self.scheduler.remove_missing_accounts(&new);
                self.gateway.update_config(new);
                if restart && self.proxy.is_some() {
                    self.shared
                        .push_notice(NoticeLevel::Info, "监听设置已变更，正在重启代理…");
                    self.stop_proxy().await;
                    self.start_proxy().await;
                }
            }
            UiCommand::SetUsageView { days, dimension } => {
                if self.usage.days != days || self.usage.dimension != dimension {
                    self.usage.days = days;
                    self.usage.dimension = dimension;
                    self.usage.last_built = None;
                }
            }
            UiCommand::TestProvider(id) => self.spawn_test("provider", id),
            UiCommand::TestAccount(id) => self.spawn_test("account", id),
            UiCommand::TestAccountDraft {
                provider_id,
                name,
                api_key,
                model_hint,
            } => self.spawn_account_draft_test(provider_id, name, api_key, model_hint),
            UiCommand::TestRoute(id) => self.spawn_test("route", id),
            UiCommand::TestLocalKey(id) => self.spawn_test("local_key", id),
            UiCommand::RefreshBalance(only) => self.spawn_balance_refresh(only, true),
            UiCommand::CleanupNow => match self.db.cleanup(self.gateway.config().ui.retention_days)
            {
                Ok(n) => self.shared.push_notice(
                    NoticeLevel::Success,
                    format!("清理完成，删除 {n} 条过期用量记录。"),
                ),
                Err(err) => self
                    .shared
                    .push_notice(NoticeLevel::Error, format!("清理失败：{err}")),
            },
            UiCommand::Shutdown => return false,
        }
        true
    }

    async fn start_proxy(&mut self) {
        if self.proxy.is_some() {
            return;
        }
        self.shared.set_proxy(ProxyStatus::Starting);
        let cfg = self.gateway.config();
        let addr = format!("{}:{}", cfg.server.bind, cfg.server.port);
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(err) => {
                let message = format!("监听 {addr} 失败：{err}");
                self.shared.set_proxy(ProxyStatus::Error(message.clone()));
                self.shared.push_notice(NoticeLevel::Error, message);
                return;
            }
        };
        let router = self.gateway.router();
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let status_cancel = cancel.clone();
        let shared = self.shared.clone();
        let join = tokio::spawn(async move {
            let server_fut = async move {
                axum::serve(listener, router)
                    .with_graceful_shutdown(async move {
                        task_cancel.cancelled().await;
                    })
                    .await
            };
            let result = AssertUnwindSafe(server_fut).catch_unwind().await;
            if status_cancel.is_cancelled() {
                shared.set_proxy(ProxyStatus::Stopped);
                return;
            }
            let message = match result {
                Ok(Ok(())) => "代理服务器意外退出".to_string(),
                Ok(Err(err)) => format!("代理服务器错误：{err}"),
                Err(_) => "代理服务器发生 panic，UI 仍在运行，可点击重启".to_string(),
            };
            shared.set_proxy(ProxyStatus::Error(message.clone()));
            shared.push_notice(NoticeLevel::Error, message);
        });
        self.proxy = Some(ProxyHandle { cancel, join });
        self.shared.set_proxy(ProxyStatus::Running);
        self.shared.push_notice(
            NoticeLevel::Success,
            format!("代理已启动：{}", self.gateway.endpoint()),
        );
    }

    async fn stop_proxy(&mut self) {
        if let Some(handle) = self.proxy.take() {
            handle.cancel.cancel();
            let mut join = handle.join;
            if tokio::time::timeout(Duration::from_secs(3), &mut join)
                .await
                .is_err()
            {
                join.abort();
            }
        }
        self.shared.set_proxy(ProxyStatus::Stopped);
    }

    async fn periodic(&mut self) {
        let cfg = self.gateway.config();
        let now = now_ts();

        if self.last_probe_check.elapsed() >= Duration::from_secs(5) {
            self.last_probe_check = Instant::now();
            let due = self.scheduler.due_probes(&cfg, now);
            for account in due {
                let Some(provider) = cfg.provider(&account.provider_id).cloned() else {
                    self.scheduler
                        .probe_finished(&account, false, &cfg.scheduler, now_ts());
                    continue;
                };
                let model_hint = cfg
                    .routes
                    .iter()
                    .find(|r| r.provider_id == provider.id)
                    .map(|r| r.upstream_model.clone())
                    .or_else(|| provider.test_model.clone());
                let gw = self.gateway.clone();
                let scheduler = self.scheduler.clone();
                let shared = self.shared.clone();
                let scheduler_cfg = cfg.scheduler.clone();
                tokio::spawn(async move {
                    let result = tokio::time::timeout(
                        Duration::from_secs(45),
                        probe::half_open_probe(&gw, &account, &provider, model_hint),
                    )
                    .await
                    .unwrap_or(false);
                    scheduler.probe_finished(&account, result, &scheduler_cfg, now_ts());
                    if result {
                        shared.push_notice(
                            NoticeLevel::Success,
                            format!("账号「{}」半开探测成功，已恢复。", account.name),
                        );
                    }
                });
            }
        }

        let balance_interval = Duration::from_secs(cfg.ui.balance_refresh_mins.max(1) * 60);
        if self.last_balance_refresh.elapsed() >= balance_interval
            && !self.shared.balance_refreshing.load(Ordering::SeqCst)
        {
            self.last_balance_refresh = Instant::now();
            self.spawn_balance_refresh(None, false);
        }

        if self.last_cleanup.elapsed() >= Duration::from_secs(6 * 3600) {
            self.last_cleanup = Instant::now();
            match self.db.cleanup(cfg.ui.retention_days) {
                Ok(n) if n > 0 => self.shared.push_notice(
                    NoticeLevel::Info,
                    format!("已按保留策略清理 {n} 条过期记录。"),
                ),
                Ok(_) => {}
                Err(err) => self
                    .shared
                    .push_notice(NoticeLevel::Warning, format!("定期清理失败：{err}")),
            }
        }
    }

    fn spawn_test(&self, kind: &str, id: String) {
        let key = format!("{kind}:{id}");
        if self.shared.is_running(&key) {
            return;
        }
        self.shared.set_running(&key, true);
        let gateway = self.gateway.clone();
        let shared = self.shared.clone();
        let kind = kind.to_string();
        tokio::spawn(async move {
            let result: TestResult = match kind.as_str() {
                "provider" => probe::test_provider(&gateway, &id).await,
                "account" => probe::test_account(&gateway, &id).await,
                "route" => probe::test_route(&gateway, &id).await,
                "local_key" => probe::test_local_key(&gateway, &id).await,
                _ => {
                    let mut r = probe::TestResult {
                        key: key.clone(),
                        kind: kind.clone(),
                        title: "未知测试".into(),
                        level: TestLevel::Error,
                        status: None,
                        latency_ms: 0,
                        message: "未知测试类型".into(),
                        detail: String::new(),
                        models: Vec::new(),
                        tokens: None,
                        at: now_ts(),
                    };
                    r.level = TestLevel::Error;
                    r
                }
            };
            shared.set_running(&key, false);
            shared.insert_test(result);
        });
    }

    fn spawn_account_draft_test(
        &self,
        provider_id: String,
        name: String,
        api_key: String,
        model_hint: Option<String>,
    ) {
        let key = "draft:account".to_string();
        if self.shared.is_running(&key) {
            return;
        }
        self.shared.set_running(&key, true);
        let gateway = self.gateway.clone();
        let shared = self.shared.clone();
        tokio::spawn(async move {
            let result =
                probe::test_account_draft(&gateway, &provider_id, &name, &api_key, model_hint)
                    .await;
            shared.set_running(&key, false);
            shared.insert_test(result);
        });
    }

    fn spawn_balance_refresh(&self, only: Option<String>, manual: bool) {
        if self.shared.balance_refreshing.swap(true, Ordering::SeqCst) {
            if manual {
                self.shared
                    .push_notice(NoticeLevel::Info, "余额刷新已在进行中。");
            }
            return;
        }
        let gateway = self.gateway.clone();
        let shared = self.shared.clone();
        tokio::spawn(async move {
            let results = balance::refresh_balances(&gateway, only.as_deref()).await;
            let errors: Vec<String> = results
                .iter()
                .filter_map(|(id, r)| r.as_ref().err().map(|e| format!("{id}: {e}")))
                .collect();
            if !errors.is_empty() {
                shared.push_notice(
                    NoticeLevel::Warning,
                    format!("余额刷新失败：{}", errors.join("；")),
                );
            } else if manual {
                shared.push_notice(
                    NoticeLevel::Success,
                    format!("余额刷新完成（{} 个账号）。", results.len()),
                );
            }
            shared.balance_refreshing.store(false, Ordering::SeqCst);
        });
    }

    // -- snapshot building --------------------------------------------------

    fn build_snapshot(&mut self) -> Arc<UiSnapshot> {
        let cfg = self.gateway.config();
        let now = now_ts();
        let today_start = db::today_start();
        let today_totals = self.db.totals_since(today_start).unwrap_or_default();
        let today_series = self.db.series_since(today_start, 3600).unwrap_or_default();
        let breaker = self.scheduler.breaker.snapshot(&cfg.scheduler, now);
        let states = self.scheduler.snapshot_states(&cfg, now);
        let balance_rows = balance::load_balance_rows(&self.gateway);

        let mut accounts = Vec::new();
        let mut provider_available: HashMap<String, (usize, usize)> = HashMap::new();
        for account in &cfg.accounts {
            let state = states
                .get(&account.id)
                .cloned()
                .unwrap_or(SchedState::Healthy);
            let inflight = self.scheduler.inflight_of(&account.id);
            let quota = self.scheduler.quota_snapshot(account, &cfg.scheduler, now);
            let (balance, estimated, source, updated, _note) =
                self.compute_balance(account, &cfg, &balance_rows);
            let (today_tokens, today_requests) = self
                .db
                .sum_account_since(&account.id, today_start)
                .unwrap_or((0, 0));
            if state == SchedState::Healthy
                && (account.max_concurrency == 0 || inflight < account.max_concurrency)
                && quota.as_ref().map(|q| !quota_exceeded(q)).unwrap_or(true)
            {
                provider_available
                    .entry(account.provider_id.clone())
                    .or_default()
                    .1 += 1;
            }
            provider_available
                .entry(account.provider_id.clone())
                .or_default()
                .0 += 1;
            accounts.push(AccountView {
                id: account.id.clone(),
                provider_id: account.provider_id.clone(),
                provider_name: cfg
                    .provider(&account.provider_id)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "未知供应商".into()),
                name: account.name.clone(),
                enabled: account.enabled,
                priority: account.priority,
                masked_key: self.store.masked_secret(&account.api_key),
                state,
                fail_streak: self.scheduler.runtime_for(&account.id).fail_streak(),
                inflight,
                max_concurrency: account.max_concurrency,
                quota,
                balance,
                balance_currency: if account.balance.currency.is_empty() {
                    "USD".into()
                } else {
                    account.balance.currency.clone()
                },
                balance_estimated: estimated,
                balance_source: source,
                balance_updated_at: updated,
                today_tokens,
                today_requests,
            });
        }

        let mut providers = Vec::new();
        let mut balances = Vec::new();
        for provider in &cfg.providers {
            let mut healthy = 0;
            let mut cooling = 0;
            let mut exhausted = 0;
            let mut disabled = 0;
            let mut half_open = 0;
            let mut total = 0;
            let mut provider_balances: BTreeMap<String, f64> = BTreeMap::new();
            for account in cfg.accounts_of(&provider.id) {
                total += 1;
                let view = accounts.iter().find(|a| a.id == account.id);
                match view.map(|v| &v.state) {
                    Some(SchedState::Healthy) => healthy += 1,
                    Some(SchedState::Cooling { .. }) => cooling += 1,
                    Some(SchedState::HalfOpen) => half_open += 1,
                    Some(SchedState::Exhausted { .. }) => exhausted += 1,
                    Some(SchedState::Disabled) | None => disabled += 1,
                }
                if let Some(view) = view {
                    if let Some(amount) = view.balance {
                        *provider_balances
                            .entry(crate::config::normalize_currency(&view.balance_currency))
                            .or_insert(0.0) += amount;
                    }
                    balances.push(BalanceView {
                        account_id: account.id.clone(),
                        provider_id: provider.id.clone(),
                        provider_name: provider.name.clone(),
                        account_name: account.name.clone(),
                        mode: account.balance.mode.label().to_string(),
                        amount: view.balance,
                        currency: view.balance_currency.clone(),
                        estimated: view.balance_estimated,
                        updated_at: view.balance_updated_at,
                        note: view.balance_source.clone(),
                        status_text: view.balance_source.clone(),
                    });
                }
            }
            providers.push(ProviderHealth {
                id: provider.id.clone(),
                name: provider.name.clone(),
                kind: provider.kind.label().to_string(),
                base_url: provider.base_url.clone(),
                account_count: total,
                healthy,
                cooling,
                exhausted,
                disabled,
                half_open,
                balances: provider_balances
                    .into_iter()
                    .map(|(currency, amount)| CurrencyTotal { currency, amount })
                    .collect(),
            });
        }

        let route_groups = self.build_route_groups(&cfg, &provider_available);

        let mut local_keys = Vec::new();
        for key in &cfg.local_keys {
            let (used_tokens, used_requests) =
                self.db.sum_local_key_since(&key.id, 0).unwrap_or((0, 0));
            local_keys.push(LocalKeyView {
                id: key.id.clone(),
                name: key.name.clone(),
                masked: self.store.masked_secret(&key.key),
                enabled: key.enabled,
                allowed_models: key.allowed_models.clone(),
                max_tokens: key.max_tokens,
                max_requests: key.max_requests,
                used_tokens,
                used_requests,
                created_at: key.created_at,
            });
        }

        let usage = self.build_usage(&cfg);
        let recent = self.db.recent_requests(40).unwrap_or_default();

        let notices = self
            .shared
            .notices
            .lock()
            .map(|n| n.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let tests = self
            .shared
            .tests
            .lock()
            .map(|t| t.clone())
            .unwrap_or_default();
        let running_tests = self
            .shared
            .running_tests
            .lock()
            .map(|t| t.clone())
            .unwrap_or_default();
        let mut warnings = self
            .shared
            .warnings
            .lock()
            .map(|w| w.clone())
            .unwrap_or_default();
        warnings.extend(crate::config::store::validate(&cfg));

        let mut alerts = Vec::new();
        let proxy_status = self.shared.proxy();
        if let ProxyStatus::Error(err) = &proxy_status {
            alerts.push(Alert {
                level: AlertLevel::Critical,
                text: format!("代理异常：{err}"),
            });
        }
        if breaker.open {
            alerts.push(Alert {
                level: AlertLevel::Critical,
                text: format!(
                    "全局熔断中，剩余 {} 秒。",
                    (breaker.open_until - now).max(0)
                ),
            });
        }
        if cfg.providers.is_empty() {
            alerts.push(Alert {
                level: AlertLevel::Info,
                text: "尚未配置供应商，请从「供应商」页开始。".into(),
            });
        } else if cfg.routes.is_empty() {
            alerts.push(Alert {
                level: AlertLevel::Warning,
                text: "尚未配置模型路由，客户端无法调用任何模型。".into(),
            });
        }
        for p in &providers {
            if p.all_unavailable() {
                alerts.push(Alert {
                    level: AlertLevel::Warning,
                    text: format!("供应商「{}」当前没有可用账号。", p.name),
                });
            }
        }
        for b in &balances {
            if let Some(amount) = b.amount {
                if amount < cfg.ui.threshold_for(&b.currency) && amount >= 0.0 {
                    alerts.push(Alert {
                        level: AlertLevel::Warning,
                        text: format!(
                            "账号「{}」余额偏低：{:.2} {}。",
                            b.account_name, amount, b.currency
                        ),
                    });
                }
            }
        }
        for warning in &warnings {
            alerts.push(Alert {
                level: AlertLevel::Warning,
                text: warning.clone(),
            });
        }
        alerts.sort_by(|a, b| b.level.cmp(&a.level));
        alerts.dedup_by(|a, b| a.text == b.text);

        let (active_requests, queued_requests) = self.gateway.queue_depth();
        let snapshot = UiSnapshot {
            proxy: proxy_status,
            endpoint: self.gateway.api_base(),
            uptime_secs: now - self.gateway.started_at,
            client_models: cfg.servable_models(),
            today: TodayView {
                totals: today_totals,
                series: today_series,
            },
            breaker,
            active_requests,
            queued_requests,
            providers,
            accounts,
            route_groups,
            local_keys,
            balances,
            usage,
            recent,
            tests,
            running_tests,
            notices,
            alerts,
            warnings,
            balance_refreshing: self.shared.balance_refreshing.load(Ordering::SeqCst),
            updated_at: now,
        };
        Arc::new(snapshot)
    }

    fn compute_balance(
        &self,
        account: &crate::config::Account,
        cfg: &AppConfig,
        rows: &HashMap<String, crate::db::BalanceRow>,
    ) -> (Option<f64>, bool, String, i64, String) {
        match account.balance.mode {
            BalanceMode::Auto => {
                if let Some(row) = rows.get(&account.id) {
                    (
                        Some(row.amount),
                        false,
                        "自动查询".to_string(),
                        row.updated_at,
                        row.note.clone(),
                    )
                } else if account
                    .balance
                    .url
                    .as_deref()
                    .map(|u| !u.trim().is_empty())
                    .unwrap_or(false)
                {
                    (
                        None,
                        false,
                        "自动查询（未刷新）".to_string(),
                        0,
                        String::new(),
                    )
                } else {
                    (
                        None,
                        false,
                        "自动查询（未配置地址）".to_string(),
                        0,
                        String::new(),
                    )
                }
            }
            BalanceMode::Manual => {
                let spent_usd = self
                    .db
                    .cost_account_since(&account.id, account.balance.manual_updated_at)
                    .unwrap_or(0) as f64
                    / 1_000_000.0;
                let currency = crate::config::normalize_currency(&account.balance.currency);
                // Route prices are USD per Mtok; convert before subtracting
                // from a balance that may be in CNY or another currency.
                let spent = spent_usd * cfg.ui.fx_rate(&currency);
                let amount = (account.balance.manual_amount - spent).max(0.0);
                (
                    Some(amount),
                    spent > 0.0,
                    "手动（按用量估算）".to_string(),
                    account.balance.manual_updated_at,
                    String::new(),
                )
            }
            BalanceMode::Unavailable => (None, false, "不可用".to_string(), 0, String::new()),
        }
    }

    fn build_route_groups(
        &self,
        cfg: &AppConfig,
        provider_available: &HashMap<String, (usize, usize)>,
    ) -> Vec<RouteGroupView> {
        let mut groups = Vec::new();
        for model in cfg.client_models() {
            let routes = cfg.routes_for(&model);
            let mut rows = Vec::new();
            let mut previous_protocol: Option<(String, crate::config::RouteMode)> = None;
            for route in &routes {
                let Some(provider) = cfg.provider(&route.provider_id) else {
                    continue;
                };
                let protocol = provider.protocol_for(&route.upstream_model, route.protocol);
                let (total, available) = provider_available
                    .get(&provider.id)
                    .copied()
                    .unwrap_or((0, 0));
                let mut warning = None;
                if total == 0 {
                    warning = Some("该供应商没有账号".to_string());
                } else if available == 0 {
                    warning = Some("当前没有可用账号".to_string());
                }
                if let Some((prev_protocol, prev_mode)) = &previous_protocol {
                    if prev_protocol != protocol.label() {
                        if route.mode == crate::config::RouteMode::Passthrough
                            || *prev_mode == crate::config::RouteMode::Passthrough
                        {
                            warning = Some(format!(
                                "协议从 {} 切换为 {}，透传模式下无法自动转换",
                                prev_protocol,
                                protocol.label()
                            ));
                        } else {
                            warning = Some(format!(
                                "故障切换到此条需要协议转换（{} → {}）",
                                prev_protocol,
                                protocol.label()
                            ));
                        }
                    }
                }
                previous_protocol = Some((protocol.label().to_string(), route.mode));
                rows.push(RouteRowView {
                    route_id: route.id.clone(),
                    provider_id: provider.id.clone(),
                    provider_name: provider.name.clone(),
                    client_model: route.client_model.clone(),
                    upstream_model: route.upstream_model.clone(),
                    protocol: protocol.label().to_string(),
                    mode: route.mode.label().to_string(),
                    priority: route.priority,
                    accounts_total: total,
                    accounts_available: available,
                    warning,
                    enabled: route.enabled,
                    fallback: false,
                });
            }
            let mut notes = Vec::new();
            let chain =
                self.scheduler
                    .build_chain(cfg, &model, crate::protocol::Ingress::Chat, false);
            notes.extend(chain.notes);
            let fallback_model = cfg.fallback_for(&model);
            if let Some(fallback) = &fallback_model {
                notes.push(format!("映射级兜底：{model} → {fallback}"));
            }
            if routes.is_empty() {
                notes.push("没有启用的路由".into());
            }
            groups.push(RouteGroupView {
                client_model: model,
                rows,
                fallback_model,
                notes,
            });
        }
        groups
    }

    fn build_usage(&mut self, cfg: &AppConfig) -> UsageView {
        let refresh = self
            .usage
            .last_built
            .map(|t| t.elapsed() >= Duration::from_secs(4))
            .unwrap_or(true);
        if !refresh {
            return self.usage.view.clone();
        }
        let days = self.usage.days;
        let dimension = self.usage.dimension;
        let since = db::range_start(days);
        let totals = self.db.totals_since(since).unwrap_or_default();
        let group_key = if dimension == UsageDimension::Global {
            "model"
        } else {
            dimension.key()
        };
        let raw_groups = self.db.group_since(group_key, since).unwrap_or_default();
        let groups = raw_groups
            .into_iter()
            .map(|row| {
                let label = match dimension {
                    UsageDimension::Global | UsageDimension::Model => {
                        if row.key.is_empty() {
                            "未知模型".into()
                        } else {
                            row.key.clone()
                        }
                    }
                    UsageDimension::Provider => cfg
                        .provider(&row.key)
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "已删除供应商".into()),
                    UsageDimension::Account => cfg
                        .account(&row.key)
                        .map(|a| {
                            let provider = cfg
                                .provider(&a.provider_id)
                                .map(|p| p.name.as_str())
                                .unwrap_or("?");
                            format!("{provider} / {}", a.name)
                        })
                        .unwrap_or_else(|| "已删除账号".into()),
                };
                UsageGroupView {
                    key: row.key,
                    label,
                    totals: row.totals,
                }
            })
            .collect();
        let model_pairs = self
            .db
            .model_upstream_since(since)
            .unwrap_or_default()
            .into_iter()
            .map(|r| (r.client_model, r.upstream_model, r.totals))
            .collect();
        let bucket = if days <= 1 { 3600 } else { 86_400 };
        let series = self.db.series_since(since, bucket).unwrap_or_default();
        self.usage.last_built = Some(Instant::now());
        self.usage.view = UsageView {
            days,
            dimension,
            totals,
            groups,
            model_pairs,
            series,
            range_start: since,
            bucket_secs: bucket,
        };
        self.usage.view.clone()
    }
}

fn quota_exceeded(q: &crate::scheduler::QuotaSnapshot) -> bool {
    q.limit_tokens.map(|l| q.used_tokens >= l).unwrap_or(false)
        || q.limit_requests
            .map(|l| q.used_requests >= l)
            .unwrap_or(false)
}
