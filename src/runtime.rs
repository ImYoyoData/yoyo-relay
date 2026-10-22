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
use crate::db::{self, Db, UsageFilter};
use crate::gateway::Gateway;
use crate::plugins::{PluginHost, PluginUiAnswer};
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
    SetUsageFilter {
        local_key: Option<String>,
    },
    SetCallsPage(usize),
    SetCallsFilter(UsageFilter),
    DeleteUsage(i64),
    ClearUsage,
    TestProvider(String),
    TestAccount(String),
    /// Measure decode throughput for one account.
    SpeedTestAccount(String),
    /// Measure decode throughput for every enabled account of a provider.
    SpeedTestProvider(String),
    TestAccountDraft {
        provider_id: String,
        name: String,
        api_key: String,
        model_hint: Option<String>,
    },
    TestRoute(String),
    TestLocalKey(String),
    FetchModelInfo {
        provider_id: String,
        upstream_model: String,
        client_model: String,
    },
    FetchProviderModels(String),
    RefreshFxRates {
        manual: bool,
    },
    RefreshBalance(Option<String>),
    /// Check GitHub for a new release. `manual` surfaces failures as notices.
    CheckUpdate {
        manual: bool,
    },
    ReloadPlugins,
    SetPluginEnabled {
        id: String,
        enabled: bool,
    },
    LoadPlugin(String),
    UnloadPlugin(String),
    ReloadPlugin(String),
    InvokePlugin {
        id: String,
        method: String,
    },
    AnswerPluginUi {
        id: u64,
        accepted: bool,
        text: String,
    },
    InstallSamplePlugins,
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
    next_fetch: AtomicU64,
    balance_refreshing: AtomicBool,
    fetched_model: Mutex<Option<FetchedModelView>>,
    model_fetch_running: AtomicBool,
    provider_models: Mutex<HashMap<String, ProviderModelsView>>,
    models_fetching: Mutex<HashSet<String>>,
    fx_refreshing: AtomicBool,
    update_checking: AtomicBool,
    update_error: Mutex<Option<String>>,
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
            next_fetch: AtomicU64::new(1),
            balance_refreshing: AtomicBool::new(false),
            fetched_model: Mutex::new(None),
            model_fetch_running: AtomicBool::new(false),
            provider_models: Mutex::new(HashMap::new()),
            models_fetching: Mutex::new(HashSet::new()),
            fx_refreshing: AtomicBool::new(false),
            update_checking: AtomicBool::new(false),
            update_error: Mutex::new(None),
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
    local_key: Option<String>,
    last_built: Option<Instant>,
    view: UsageView,
}

pub struct Runtime {
    store: Arc<ConfigStore>,
    gateway: Arc<Gateway>,
    scheduler: Arc<Scheduler>,
    db: Arc<Db>,
    shared: Arc<Shared>,
    plugins: Arc<PluginHost>,
    cmd_rx: mpsc::UnboundedReceiver<UiCommand>,
    snapshot_tx: std::sync::mpsc::Sender<Arc<UiSnapshot>>,
    proxy: Option<ProxyHandle>,
    usage: UsageParams,
    calls_page: usize,
    calls_filter: UsageFilter,
    last_probe_check: Instant,
    last_balance_refresh: Instant,
    last_fx_refresh: Instant,
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
        let plugins = PluginHost::new(&store.paths.config_dir, tokio::runtime::Handle::current());
        plugins.apply_settings(config.plugins.clone());
        let gateway = Gateway::new(
            Arc::new(config.clone()),
            store.master().clone(),
            db.clone(),
            scheduler.clone(),
            plugins.clone(),
        );
        plugins.set_runtime_info(gateway.api_base(), config.servable_models());
        let shared = Arc::new(Shared::new(warnings));
        Ok(Self {
            store,
            gateway,
            scheduler,
            db,
            shared,
            plugins,
            cmd_rx,
            snapshot_tx,
            proxy: None,
            usage: UsageParams {
                days: 7,
                dimension: UsageDimension::Global,
                local_key: None,
                last_built: None,
                view: UsageView::default(),
            },
            calls_page: 0,
            calls_filter: UsageFilter::default(),
            last_probe_check: Instant::now(),
            last_balance_refresh: Instant::now() - Duration::from_secs(3600),
            // Refresh rates on the first periodic tick so a fresh install gets
            // a realistic CNY total without waiting for the interval.
            last_fx_refresh: Instant::now() - Duration::from_secs(24 * 3600),
            last_cleanup: Instant::now() - Duration::from_secs(6 * 3600),
        })
    }

    async fn run(mut self) {
        self.start_proxy().await;
        self.plugins.load_all_enabled().await;
        self.emit_snapshot();
        // Emit twice as often while requests are in flight so the live
        // tokens-per-second badge stays responsive; fall back to a slower
        // cadence when idle to keep the aggregation queries cheap.
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut idle_ticks: u8 = 0;
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
                    let busy = self.gateway.queue_depth().0 > 0
                        || self.gateway.speed.tokens_per_sec() > 0.0;
                    idle_ticks = if busy { 0 } else { idle_ticks.saturating_add(1) };
                    if !busy && idle_ticks % 2 == 0 {
                        // Skip the expensive snapshot on alternate idle ticks.
                        continue;
                    }
                    self.periodic().await;
                    if !self.emit_snapshot() {
                        break;
                    }
                }
            }
        }
        self.plugins.unload_all().await;
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
                let mut new = *cfg;
                // Never let a stale UI copy roll back a live-rate refresh that
                // landed after the last snapshot was emitted.
                if new.ui.fx_updated_at < old.ui.fx_updated_at {
                    new.ui.fx_rates = old.ui.fx_rates.clone();
                    new.ui.fx_updated_at = old.ui.fx_updated_at;
                    new.ui.fx_source = old.ui.fx_source.clone();
                }
                if new.ui.update_checked_at < old.ui.update_checked_at {
                    new.ui.update_latest_version = old.ui.update_latest_version.clone();
                    new.ui.update_latest_url = old.ui.update_latest_url.clone();
                    new.ui.update_latest_notes = old.ui.update_latest_notes.clone();
                    new.ui.update_checked_at = old.ui.update_checked_at;
                }
                let new = Arc::new(new);
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
                let permissions_changed = old.plugins.allow_network != new.plugins.allow_network
                    || old.plugins.allow_process != new.plugins.allow_process;
                self.gateway.update_config(new.clone());
                self.plugins.apply_settings(new.plugins.clone());
                let host = self.plugins.clone();
                tokio::spawn(async move {
                    if permissions_changed {
                        host.reload_all().await;
                    }
                    host.sync_enabled().await;
                });
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
            UiCommand::SetUsageFilter { local_key } => {
                if self.usage.local_key != local_key {
                    self.usage.local_key = local_key;
                    self.usage.last_built = None;
                }
            }
            UiCommand::SetCallsPage(page) => {
                self.calls_page = page;
            }
            UiCommand::SetCallsFilter(filter) => {
                if self.calls_filter != filter {
                    self.calls_filter = filter;
                    self.calls_page = 0;
                }
            }
            UiCommand::DeleteUsage(id) => match self.db.delete_usage(id) {
                Ok(true) => self
                    .shared
                    .push_notice(NoticeLevel::Success, "调用记录已删除。"),
                Ok(false) => self
                    .shared
                    .push_notice(NoticeLevel::Info, "记录不存在或已被清理。"),
                Err(err) => self
                    .shared
                    .push_notice(NoticeLevel::Error, format!("删除记录失败：{err}")),
            },
            UiCommand::ClearUsage => match self.db.clear_usage() {
                Ok(n) => self
                    .shared
                    .push_notice(NoticeLevel::Success, format!("已清空 {n} 条调用明细。")),
                Err(err) => self
                    .shared
                    .push_notice(NoticeLevel::Error, format!("清空失败：{err}")),
            },
            UiCommand::TestProvider(id) => self.spawn_test("provider", id),
            UiCommand::TestAccount(id) => self.spawn_test("account", id),
            UiCommand::SpeedTestAccount(id) => self.spawn_speed_test("account", id),
            UiCommand::SpeedTestProvider(id) => self.spawn_speed_test("provider", id),
            UiCommand::TestAccountDraft {
                provider_id,
                name,
                api_key,
                model_hint,
            } => self.spawn_account_draft_test(provider_id, name, api_key, model_hint),
            UiCommand::TestRoute(id) => self.spawn_test("route", id),
            UiCommand::TestLocalKey(id) => self.spawn_test("local_key", id),
            UiCommand::FetchModelInfo {
                provider_id,
                upstream_model,
                client_model,
            } => self.spawn_model_fetch(provider_id, upstream_model, client_model),
            UiCommand::FetchProviderModels(provider_id) => self.spawn_model_list_fetch(provider_id),
            UiCommand::RefreshFxRates { manual } => self.spawn_fx_refresh(manual),
            UiCommand::RefreshBalance(only) => self.spawn_balance_refresh(only, true),
            UiCommand::CheckUpdate { manual } => self.spawn_update_check(manual),
            UiCommand::ReloadPlugins => {
                self.plugins.scan();
                let host = self.plugins.clone();
                tokio::spawn(async move {
                    host.sync_enabled().await;
                });
                self.shared
                    .push_notice(NoticeLevel::Info, "已重新扫描插件目录。");
            }
            UiCommand::SetPluginEnabled { id, enabled } => {
                let mut cfg = (*self.gateway.config()).clone();
                cfg.plugins.disabled.retain(|plugin| plugin != &id);
                if !enabled {
                    cfg.plugins.disabled.push(id.clone());
                }
                match self.store.save(&mut cfg) {
                    Ok(()) => {
                        self.gateway.update_config(Arc::new(cfg.clone()));
                        self.plugins.apply_settings(cfg.plugins.clone());
                        if enabled {
                            if let Err(err) = self.plugins.load(&id, &cfg.plugins).await {
                                self.shared.push_notice(
                                    NoticeLevel::Error,
                                    format!("启用插件失败：{err}"),
                                );
                            } else {
                                self.plugins
                                    .notify("onEnable", serde_json::json!({"event": "onEnable"}));
                            }
                        } else {
                            self.plugins
                                .notify("onDisable", serde_json::json!({"event": "onDisable"}));
                            self.plugins.unload(&id).await;
                        }
                    }
                    Err(err) => self
                        .shared
                        .push_notice(NoticeLevel::Error, format!("保存插件设置失败：{err}")),
                }
            }
            UiCommand::LoadPlugin(id) => {
                let settings = self.gateway.config().plugins.clone();
                if let Err(err) = self.plugins.load(&id, &settings).await {
                    self.shared
                        .push_notice(NoticeLevel::Error, format!("加载插件失败：{err}"));
                }
            }
            UiCommand::UnloadPlugin(id) => {
                self.plugins.unload(&id).await;
            }
            UiCommand::ReloadPlugin(id) => {
                let settings = self.gateway.config().plugins.clone();
                match self.plugins.reload(&id).await {
                    Ok(()) => self
                        .shared
                        .push_notice(NoticeLevel::Success, "插件已重新加载。"),
                    Err(err) => self
                        .shared
                        .push_notice(NoticeLevel::Error, format!("重载失败：{err}")),
                }
                let _ = settings;
            }
            UiCommand::InvokePlugin { id, method } => {
                match self
                    .plugins
                    .invoke(&id, &method, serde_json::json!({"event": method}))
                    .await
                {
                    Ok(value) => {
                        let text = value
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "undefined".to_string());
                        self.plugins
                            .log(&id, "info", format!("{method} 返回：{text}"));
                        self.shared.push_notice(
                            NoticeLevel::Success,
                            format!("插件 {id} · {method}：{text}"),
                        );
                    }
                    Err(err) => {
                        self.plugins.log(&id, "error", &err);
                        self.shared
                            .push_notice(NoticeLevel::Error, format!("插件调用失败：{err}"));
                    }
                }
            }
            UiCommand::AnswerPluginUi { id, accepted, text } => {
                self.plugins
                    .answer_ui(id, PluginUiAnswer { accepted, text });
            }
            UiCommand::InstallSamplePlugins => match self.plugins.install_samples() {
                Ok(dirs) => {
                    self.plugins.scan();
                    self.shared.push_notice(
                        NoticeLevel::Success,
                        format!("已写入示例插件：{}", dirs.join("、")),
                    );
                }
                Err(err) => self
                    .shared
                    .push_notice(NoticeLevel::Error, format!("安装示例失败：{err}")),
            },
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

        let update_interval = Duration::from_secs(cfg.ui.update_check_hours.max(1) * 3600);
        if cfg.ui.auto_update_check
            && (now - cfg.ui.update_checked_at) >= update_interval.as_secs() as i64
            && !self.shared.update_checking.load(Ordering::SeqCst)
        {
            self.spawn_update_check(false);
        }

        let fx_interval = Duration::from_secs(cfg.ui.fx_refresh_hours.max(1) * 3600);
        if cfg.ui.fx_auto
            && self.last_fx_refresh.elapsed() >= fx_interval
            && !self.shared.fx_refreshing.load(Ordering::SeqCst)
        {
            self.last_fx_refresh = Instant::now();
            self.spawn_fx_refresh(false);
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
                        tokens_per_sec: None,
                        first_token_ms: None,
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

    fn spawn_speed_test(&self, kind: &str, id: String) {
        let key = format!("speed:{id}");
        if self.shared.is_running(&key) {
            return;
        }
        self.shared.set_running(&key, true);
        let gateway = self.gateway.clone();
        let shared = self.shared.clone();
        let kind = kind.to_string();
        tokio::spawn(async move {
            let result = match kind.as_str() {
                "provider" => probe::test_speed_provider(&gateway, &id).await,
                _ => probe::test_speed_account(&gateway, &id, None).await,
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

    fn spawn_model_fetch(&self, provider_id: String, upstream_model: String, client_model: String) {
        if self.shared.model_fetch_running.swap(true, Ordering::SeqCst) {
            self.shared
                .push_notice(NoticeLevel::Info, "模型信息获取已在进行中。");
            return;
        }
        let gateway = self.gateway.clone();
        let shared = self.shared.clone();
        let provider_name = self
            .gateway
            .config()
            .provider(&provider_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        tokio::spawn(async move {
            let fetched = probe::fetch_model_info(&gateway, &provider_id, &upstream_model).await;
            let level = if fetched.ok {
                NoticeLevel::Success
            } else {
                NoticeLevel::Warning
            };
            shared.push_notice(level, format!("{}：{}", upstream_model, fetched.message));
            let view = FetchedModelView {
                id: shared.next_fetch.fetch_add(1, Ordering::SeqCst),
                client_model,
                provider_name,
                upstream_model,
                ok: fetched.ok,
                message: fetched.message,
                context_window: fetched.context_window,
                max_output_tokens: fetched.max_output_tokens,
                input_modalities: fetched.input_modalities,
                output_modalities: fetched.output_modalities,
                reasoning_levels: fetched.reasoning_levels,
                supports_tools: fetched.supports_tools,
                supports_streaming: fetched.supports_streaming,
                models_seen: fetched.models_seen,
            };
            if let Ok(mut slot) = shared.fetched_model.lock() {
                *slot = Some(view);
            }
            shared.model_fetch_running.store(false, Ordering::SeqCst);
        });
    }

    fn spawn_model_list_fetch(&self, provider_id: String) {
        {
            let mut fetching = self
                .shared
                .models_fetching
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if !fetching.insert(provider_id.clone()) {
                return;
            }
        }
        let gateway = self.gateway.clone();
        let shared = self.shared.clone();
        let provider_name = self
            .gateway
            .config()
            .provider(&provider_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        tokio::spawn(async move {
            let fetched = probe::fetch_model_list(&gateway, &provider_id).await;
            let level = if fetched.ok {
                NoticeLevel::Info
            } else {
                NoticeLevel::Warning
            };
            shared.push_notice(level, format!("{}：{}", provider_name, fetched.message));
            if let Ok(mut map) = shared.provider_models.lock() {
                map.insert(
                    provider_id.clone(),
                    ProviderModelsView {
                        provider_id: provider_id.clone(),
                        provider_name,
                        models: fetched.models,
                        ok: fetched.ok,
                        message: fetched.message,
                        at: now_ts(),
                    },
                );
            }
            if let Ok(mut fetching) = shared.models_fetching.lock() {
                fetching.remove(&provider_id);
            }
        });
    }

    fn spawn_update_check(&self, manual: bool) {
        if self.shared.update_checking.swap(true, Ordering::SeqCst) {
            if manual {
                self.shared
                    .push_notice(NoticeLevel::Info, "更新检查已在进行中。");
            }
            return;
        }
        let gateway = self.gateway.clone();
        let shared = self.shared.clone();
        let store = self.store.clone();
        tokio::spawn(async move {
            let client = gateway.client_for("https://api.github.com").clone();
            let result = crate::update::fetch_latest_release(&client).await;
            match result {
                Ok(release) => {
                    let current = env!("CARGO_PKG_VERSION");
                    let newer = crate::update::is_newer(&release.version, current);
                    let mut cfg = (*gateway.config()).clone();
                    cfg.ui.update_latest_version = release.version.clone();
                    cfg.ui.update_latest_url = release.url.clone();
                    cfg.ui.update_latest_notes = release.notes.clone();
                    cfg.ui.update_checked_at = now_ts();
                    if let Err(err) = store.save(&mut cfg) {
                        shared.push_notice(NoticeLevel::Error, format!("更新信息保存失败：{err}"));
                    } else {
                        gateway.update_config(Arc::new(cfg));
                    }
                    if let Ok(mut slot) = shared.update_error.lock() {
                        *slot = None;
                    }
                    if manual {
                        if newer {
                            shared.push_notice(
                                NoticeLevel::Success,
                                format!(
                                    "发现新版本 v{}（当前 v{current}），可在标题栏查看。",
                                    release.version
                                ),
                            );
                        } else {
                            shared.push_notice(
                                NoticeLevel::Info,
                                format!("当前已是最新版本 v{current}。"),
                            );
                        }
                    }
                }
                Err(err) => {
                    if let Ok(mut slot) = shared.update_error.lock() {
                        *slot = Some(format!("{err:#}"));
                    }
                    if manual {
                        shared.push_notice(NoticeLevel::Warning, format!("检查更新失败：{err:#}"));
                    }
                }
            }
            shared.update_checking.store(false, Ordering::SeqCst);
        });
    }

    fn spawn_fx_refresh(&self, manual: bool) {
        if self.shared.fx_refreshing.swap(true, Ordering::SeqCst) {
            if manual {
                self.shared
                    .push_notice(NoticeLevel::Info, "汇率刷新已在进行中。");
            }
            return;
        }
        let gateway = self.gateway.clone();
        let shared = self.shared.clone();
        let store = self.store.clone();
        tokio::spawn(async move {
            let client = gateway
                .client_for("https://api.frankfurter.app/latest?from=USD")
                .clone();
            let result = crate::fx::fetch_rates(&client).await;
            match result {
                Ok(rates) => {
                    let mut cfg = (*gateway.config()).clone();
                    cfg.ui.fx_rates = rates.rates.clone();
                    cfg.ui.fx_updated_at = now_ts();
                    cfg.ui.fx_source = rates.source.clone();
                    match store.save(&mut cfg) {
                        Ok(()) => {
                            gateway.update_config(Arc::new(cfg));
                            if manual {
                                shared.push_notice(
                                    NoticeLevel::Success,
                                    format!(
                                        "汇率已更新（{}）：1 USD = {:.4} CNY。",
                                        rates.source,
                                        rates.rates.get("CNY").copied().unwrap_or(0.0)
                                    ),
                                );
                            }
                        }
                        Err(err) => shared.push_notice(
                            NoticeLevel::Error,
                            format!("汇率已获取但保存失败：{err}"),
                        ),
                    }
                }
                Err(err) => {
                    if manual {
                        shared.push_notice(NoticeLevel::Warning, format!("汇率获取失败：{err:#}"));
                    } else {
                        tracing::warn!("自动汇率刷新失败：{err:#}");
                    }
                }
            }
            shared.fx_refreshing.store(false, Ordering::SeqCst);
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
        const CALLS_PAGE_SIZE: usize = 20;
        let calls_total = self.db.usage_count(&self.calls_filter).unwrap_or(0);
        let max_page = ((calls_total as usize).saturating_sub(1)) / CALLS_PAGE_SIZE;
        if self.calls_page > max_page {
            self.calls_page = max_page;
        }
        let calls_rows = self
            .db
            .usage_page(
                &self.calls_filter,
                self.calls_page * CALLS_PAGE_SIZE,
                CALLS_PAGE_SIZE,
            )
            .unwrap_or_default();
        let calls = CallsView {
            rows: calls_rows,
            total: calls_total,
            page: self.calls_page,
            page_size: CALLS_PAGE_SIZE,
            filter: self.calls_filter.clone(),
        };
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

        for (level, text) in self.plugins.drain_notices() {
            self.shared.push_notice(level, text);
        }
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
            tokens_per_sec: self.gateway.speed.tokens_per_sec(),
            providers,
            accounts,
            route_groups,
            local_keys,
            balances,
            usage,
            calls,
            recent,
            tests,
            running_tests,
            notices,
            alerts,
            warnings,
            balance_refreshing: self.shared.balance_refreshing.load(Ordering::SeqCst),
            fetched_model: self
                .shared
                .fetched_model
                .lock()
                .map(|slot| slot.clone())
                .unwrap_or(None),
            model_fetch_running: self.shared.model_fetch_running.load(Ordering::SeqCst),
            provider_models: self
                .shared
                .provider_models
                .lock()
                .map(|map| map.clone())
                .unwrap_or_default(),
            models_fetching: self
                .shared
                .models_fetching
                .lock()
                .map(|set| set.clone())
                .unwrap_or_default(),
            plugins: self.plugins.views(),
            plugin_ui: self.plugins.ui_request(),
            plugin_settings: cfg.plugins.clone(),
            update: UpdateView {
                current: env!("CARGO_PKG_VERSION").to_string(),
                latest: cfg.ui.update_latest_version.clone(),
                url: cfg.ui.update_latest_url.clone(),
                notes: cfg.ui.update_latest_notes.clone(),
                checked_at: cfg.ui.update_checked_at,
                available: !cfg.ui.update_latest_version.is_empty()
                    && crate::update::is_newer(
                        &cfg.ui.update_latest_version,
                        env!("CARGO_PKG_VERSION"),
                    ),
                dismissed: crate::update::normalize_version(&cfg.ui.update_dismissed_version)
                    == crate::update::normalize_version(&cfg.ui.update_latest_version),
                checking: self.shared.update_checking.load(Ordering::SeqCst),
                error: self
                    .shared
                    .update_error
                    .lock()
                    .map(|slot| slot.clone())
                    .unwrap_or(None),
            },
            fx_rates: cfg.ui.fx_rates.clone(),
            fx_updated_at: cfg.ui.fx_updated_at,
            fx_source: cfg.ui.fx_source.clone(),
            fx_refreshing: self.shared.fx_refreshing.load(Ordering::SeqCst),
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
            let routes = cfg.all_routes_for(&model);
            let enabled_count = routes.iter().filter(|route| route.enabled).count();
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
                if route.enabled {
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
                }
                let pricing = route.effective_pricing();
                let price_label = if pricing.is_zero() {
                    String::new()
                } else {
                    format!(
                        "{} {} / {} 每 Mtok",
                        pricing.currency.symbol(),
                        format_price_value(pricing.base.input_per_mtok),
                        format_price_value(pricing.base.output_per_mtok)
                    )
                };
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
                    price_label,
                    price_periods: pricing.periods.len(),
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
            } else if enabled_count == 0 {
                notes.push("该路由模型的所有上游都已停用，不会参与调度。".into());
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
        let local_key = self.usage.local_key.clone();
        let since = db::range_start(days);
        let totals = self
            .db
            .totals_since_key(since, local_key.as_deref())
            .unwrap_or_default();
        let group_key = if dimension == UsageDimension::Global {
            "model"
        } else {
            dimension.key()
        };
        let raw_groups = self
            .db
            .group_since_key(group_key, since, local_key.as_deref())
            .unwrap_or_default();
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
            .model_upstream_since_key(since, local_key.as_deref())
            .unwrap_or_default()
            .into_iter()
            .map(|r| (r.client_model, r.upstream_model, r.totals))
            .collect();
        let bucket = if days <= 1 { 3600 } else { 86_400 };
        let series = self
            .db
            .series_since_key(since, bucket, local_key.as_deref())
            .unwrap_or_default();
        self.usage.last_built = Some(Instant::now());
        self.usage.view = UsageView {
            days,
            dimension,
            local_key,
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

/// Compact price rendering for route rows.
fn format_price_value(value: f64) -> String {
    if value == 0.0 {
        "0".to_string()
    } else if value >= 1.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.4}")
    }
}

fn quota_exceeded(q: &crate::scheduler::QuotaSnapshot) -> bool {
    q.limit_tokens.map(|l| q.used_tokens >= l).unwrap_or(false)
        || q.limit_requests
            .map(|l| q.used_requests >= l)
            .unwrap_or(false)
}
