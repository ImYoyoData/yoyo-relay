pub mod accounts;
pub mod balances;
pub mod brands;
pub mod calls;
pub mod editors;
pub mod keys;
pub mod kit;
pub mod overview;
pub mod plugins;
pub mod providers;
pub mod routes;
pub mod settings;
pub mod theme;
pub mod usage;
pub mod widgets;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{
    Align, Align2, Color32, CursorIcon, Id, Layout, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use egui_phosphor::regular as icons;
use tokio::sync::mpsc;

use crate::config::{
    now_ts, Account, AppConfig, ClientModelProfile, ConfigStore, ModelRoute, Provider,
};
use crate::runtime::UiCommand;
use crate::snapshot::{NoticeLevel, ProxyStatus, UiSnapshot, UsageDimension};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Taskbar,
    Proxy,
    Scheduler,
    Pricing,
    Security,
    Data,
    About,
}

impl SettingsTab {
    pub fn all() -> [SettingsTab; 8] {
        [
            SettingsTab::General,
            SettingsTab::Taskbar,
            SettingsTab::Proxy,
            SettingsTab::Scheduler,
            SettingsTab::Pricing,
            SettingsTab::Security,
            SettingsTab::Data,
            SettingsTab::About,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            SettingsTab::General => "通用与启动",
            SettingsTab::Taskbar => "任务栏徽标",
            SettingsTab::Proxy => "代理",
            SettingsTab::Scheduler => "调度与熔断",
            SettingsTab::Pricing => "计费价格",
            SettingsTab::Security => "安全",
            SettingsTab::Data => "数据",
            SettingsTab::About => "关于",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            SettingsTab::General => icons::SLIDERS_HORIZONTAL,
            SettingsTab::Taskbar => icons::MONITOR,
            SettingsTab::Proxy => icons::DESKTOP_TOWER,
            SettingsTab::Scheduler => icons::ACTIVITY,
            SettingsTab::Pricing => icons::TAG,
            SettingsTab::Security => icons::SHIELD_CHECK,
            SettingsTab::Data => icons::DATABASE,
            SettingsTab::About => icons::INFO,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Overview,
    Providers,
    Accounts,
    Routes,
    Plugins,
    LocalKeys,
    Usage,
    Calls,
    Balances,
    Settings,
}

impl Page {
    pub fn all() -> [Page; 10] {
        [
            Page::Overview,
            Page::Providers,
            Page::Accounts,
            Page::Routes,
            Page::Plugins,
            Page::LocalKeys,
            Page::Usage,
            Page::Calls,
            Page::Balances,
            Page::Settings,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            Page::Overview => "概览",
            Page::Providers => "供应商",
            Page::Accounts => "账号",
            Page::Routes => "路由",
            Page::Plugins => "插件",
            Page::LocalKeys => "本地 Key",
            Page::Usage => "用量",
            Page::Calls => "调用明细",
            Page::Balances => "余额",
            Page::Settings => "设置",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Page::Overview => icons::HOUSE,
            Page::Providers => icons::HARD_DRIVES,
            Page::Accounts => icons::USERS,
            Page::Routes => icons::GIT_BRANCH,
            Page::Plugins => icons::PUZZLE_PIECE,
            Page::LocalKeys => icons::KEY,
            Page::Usage => icons::CHART_BAR,
            Page::Calls => icons::LIST_BULLETS,
            Page::Balances => icons::WALLET,
            Page::Settings => icons::GEAR_SIX,
        }
    }

    pub fn eyebrow(self) -> &'static str {
        match self {
            Page::Overview => "Workbench · 概览",
            Page::Providers => "Configuration · 供应商",
            Page::Accounts => "Configuration · 账号",
            Page::Routes => "Configuration · 路由",
            Page::Plugins => "Configuration · 插件",
            Page::LocalKeys => "Configuration · 本地 Key",
            Page::Usage => "Observability · 用量",
            Page::Calls => "Observability · 调用明细",
            Page::Balances => "Observability · 余额",
            Page::Settings => "System · 设置",
        }
    }

    pub fn from_key(key: &str) -> Option<Page> {
        match key.to_ascii_lowercase().as_str() {
            "overview" | "home" => Some(Page::Overview),
            "providers" => Some(Page::Providers),
            "accounts" => Some(Page::Accounts),
            "routes" => Some(Page::Routes),
            "plugins" => Some(Page::Plugins),
            "keys" | "local_keys" => Some(Page::LocalKeys),
            "usage" => Some(Page::Usage),
            "calls" | "details" => Some(Page::Calls),
            "balances" => Some(Page::Balances),
            "settings" => Some(Page::Settings),
            _ => None,
        }
    }

    pub fn needs_search(self) -> bool {
        matches!(
            self,
            Page::Providers | Page::Accounts | Page::Routes | Page::Plugins | Page::LocalKeys
        )
    }
}

#[allow(dead_code)]
pub(crate) struct Toast {
    pub id: u64,
    pub level: NoticeLevel,
    pub text: String,
    pub until: Instant,
}

pub(crate) enum Editor {
    Provider(ProviderDraft),
    Account(AccountDraft),
    Route(RouteDraft),
    RouteModel(RouteModelDraft),
    LocalKey(LocalKeyDraft),
    ManualBalance(ManualBalanceDraft),
    BalanceSetup(BalanceSetupDraft),
}

pub(crate) struct ProviderDraft {
    pub is_new: bool,
    pub preset: String,
    pub provider: Provider,
}

pub(crate) struct AccountDraft {
    pub is_new: bool,
    pub account: Account,
    pub api_key_input: String,
    pub quota_enabled: bool,
    pub limit_tokens: u64,
    pub limit_requests: u64,
    pub rolling_hours: u32,
}

pub(crate) struct RouteDraft {
    pub is_new: bool,
    pub route: ModelRoute,
    pub capabilities_text: String,
    pub fallback_text: String,
    pub max_context_enabled: bool,
    pub max_context: u64,
}

pub(crate) struct RouteModelDraft {
    pub is_new: bool,
    pub original_model: String,
    pub profile: ClientModelProfile,
    pub reasoning_text: String,
}

pub(crate) struct LocalKeyDraft {
    pub name: String,
    pub allowed_models_text: String,
    pub max_tokens: u64,
    pub max_requests: u64,
    pub enabled: bool,
}

pub(crate) struct ManualBalanceDraft {
    pub account_id: String,
    pub amount: f64,
}

/// Full balance configuration for one account, editable from the Balances
/// page. It edits the same `Account.balance` as the account editor.
pub(crate) struct BalanceSetupDraft {
    pub account_id: String,
    pub account_name: String,
    pub provider_name: String,
    pub balance: crate::config::BalanceConfig,
    pub percent_used: f64,
}

pub(crate) enum ConfirmAction {
    DeleteProvider(String),
    DeleteAccount(String),
    DeleteRoute(String),
    DeleteRouteModel(String),
    DeleteLocalKey(String),
    ResetLocalKey(String),
    ImportConfig(String),
    ClearCalls,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageMetric {
    Tokens,
    Requests,
    Cost,
}

pub struct App {
    pub(crate) config: AppConfig,
    pub(crate) store: Arc<ConfigStore>,
    pub(crate) cmd_tx: mpsc::UnboundedSender<UiCommand>,
    pub(crate) snap_rx: std::sync::mpsc::Receiver<Arc<UiSnapshot>>,
    pub(crate) snapshot: Arc<UiSnapshot>,
    pub(crate) page: Page,
    pub(crate) settings_tab: SettingsTab,
    pub(crate) logo: egui::TextureHandle,
    pub(crate) startup_minimize_at: Option<Instant>,
    pub(crate) last_title: String,
    pub(crate) autostart_synced: Option<bool>,
    pub(crate) editor: Option<Editor>,
    pub(crate) confirm: Option<ConfirmAction>,
    pub(crate) test_key: Option<String>,
    pub(crate) test_started_at: Option<Instant>,
    pub(crate) toasts: Vec<Toast>,
    pub(crate) toast_seq: u64,
    pub(crate) last_notice_id: u64,
    pub(crate) search: String,
    pub(crate) search_focus: bool,
    pub(crate) usage_days: u32,
    pub(crate) usage_dim: UsageDimension,
    pub(crate) usage_metric: UsageMetric,
    pub(crate) usage_expanded: HashSet<String>,
    pub(crate) usage_pinned: Option<usize>,
    pub(crate) usage_local_key: Option<String>,
    pub(crate) show_onboarding: bool,
    pub(crate) onboarding_step: usize,
    pub(crate) import_path: String,
    pub(crate) last_export_path: Option<String>,
    pub(crate) last_dark: Option<bool>,
    pub(crate) last_applied_fetch: Option<u64>,
    pub(crate) models_requested: HashSet<String>,
    pub(crate) taskbar_badge: Option<crate::taskbar::TaskbarBadge>,
    /// Draft price plan awaiting a name in the "save as plan" modal.
    pub(crate) saving_price_plan: Option<crate::config::PricePlan>,
    pub(crate) price_plan_import_path: String,
    pub(crate) update_modal: bool,
    pub(crate) update_waiting: bool,
    pub(crate) update_prompted: Option<String>,
    pub(crate) plugin_docs_open: bool,
    pub(crate) plugin_ui_id: u64,
    pub(crate) plugin_ui_text: String,
    pub(crate) closing: bool,
    pub(crate) last_zoom: f32,
    pub(crate) signals: Arc<crate::instance::Signals>,
    pub(crate) tray: Option<crate::tray::Tray>,
    pub(crate) tray_ready: bool,
    pub(crate) ctx_attached: bool,
    pub(crate) native_attached: bool,
    pub(crate) native_frame_applied: bool,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        store: Arc<ConfigStore>,
        config: AppConfig,
        cmd_tx: mpsc::UnboundedSender<UiCommand>,
        snap_rx: std::sync::mpsc::Receiver<Arc<UiSnapshot>>,
        initial_page: Page,
        signals: Arc<crate::instance::Signals>,
    ) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let show_onboarding = !config.ui.onboarding_done && config.providers.is_empty();
        let logo_bytes: &[u8] = include_bytes!("../../assets/icon_256.rgba");
        let logo_image = egui::ColorImage::from_rgba_unmultiplied([256, 256], logo_bytes);
        let logo = cc
            .egui_ctx
            .load_texture("yoyo_logo", logo_image, egui::TextureOptions::LINEAR);
        let launch_minimized = std::env::args().any(|a| a == "--minimized");
        let startup_minimize = launch_minimized || config.ui.start_minimized;
        let startup_minimize_at =
            startup_minimize.then(|| Instant::now() + Duration::from_millis(600));
        #[allow(unused_mut)]
        let mut app = Self {
            config,
            store,
            cmd_tx,
            snap_rx,
            snapshot: Arc::new(UiSnapshot::default()),
            page: initial_page,
            settings_tab: SettingsTab::General,
            logo,
            startup_minimize_at,
            last_title: String::new(),
            autostart_synced: None,
            editor: None,
            confirm: None,
            test_key: None,
            test_started_at: None,
            toasts: Vec::new(),
            toast_seq: 1,
            last_notice_id: 0,
            search: String::new(),
            search_focus: false,
            usage_days: 7,
            usage_dim: UsageDimension::Global,
            usage_metric: UsageMetric::Tokens,
            usage_expanded: HashSet::new(),
            usage_pinned: None,
            usage_local_key: None,
            show_onboarding,
            onboarding_step: 0,
            import_path: String::new(),
            last_export_path: None,
            last_dark: None,
            last_applied_fetch: None,
            models_requested: HashSet::new(),
            taskbar_badge: None,
            saving_price_plan: None,
            price_plan_import_path: String::new(),
            update_modal: false,
            update_waiting: false,
            update_prompted: None,
            plugin_docs_open: false,
            plugin_ui_id: 0,
            plugin_ui_text: String::new(),
            closing: false,
            last_zoom: 1.0,
            signals,
            tray: None,
            tray_ready: false,
            ctx_attached: false,
            native_attached: false,
            native_frame_applied: false,
        };
        #[cfg(debug_assertions)]
        if let Ok(tab) = std::env::var("YOYO_RELAY_SETTINGS_TAB") {
            app.settings_tab = match tab.as_str() {
                "general" => SettingsTab::General,
                "taskbar" => SettingsTab::Taskbar,
                "proxy" => SettingsTab::Proxy,
                "scheduler" => SettingsTab::Scheduler,
                "pricing" => SettingsTab::Pricing,
                "security" => SettingsTab::Security,
                "data" => SettingsTab::Data,
                "about" => SettingsTab::About,
                _ => SettingsTab::General,
            };
        }
        #[cfg(debug_assertions)]
        if let Ok(demo) = std::env::var("YOYO_RELAY_DEMO") {
            match demo.as_str() {
                "editor" => app.open_new_provider(),
                "account_editor" => app.open_new_account(),
                "account_edit" => {
                    if let Some(account) = app.config.accounts.first() {
                        let id = account.id.clone();
                        app.open_edit_account(&id);
                    }
                }
                "balance_setup" => {
                    if let Some(account) = app.config.accounts.first() {
                        let id = account.id.clone();
                        app.open_balance_setup(&id);
                    }
                }
                "route_editor" | "route_model_editor" => app.open_new_route_model(),
                "route_model_edit" => {
                    if let Some(model) = app
                        .config
                        .model_profiles
                        .first()
                        .map(|profile| profile.client_model.clone())
                    {
                        app.open_edit_route_model(&model);
                    }
                }
                "route_fetch" => {
                    if let Some(model) = app
                        .config
                        .model_profiles
                        .first()
                        .map(|profile| profile.client_model.clone())
                    {
                        if let Some(route) = app
                            .config
                            .routes
                            .iter()
                            .find(|route| route.client_model == model)
                        {
                            let _ = app.cmd_tx.send(UiCommand::FetchModelInfo {
                                provider_id: route.provider_id.clone(),
                                upstream_model: route.upstream_model.clone(),
                                client_model: model.clone(),
                            });
                        }
                        app.open_edit_route_model(&model);
                    }
                }
                "route_upstream_editor" => {
                    if let Some(route) = app.config.routes.first() {
                        let client_model = route.client_model.clone();
                        app.open_new_route(&client_model);
                    }
                }
                "route_upstream_edit" => {
                    if let Some(id) = app.config.routes.first().map(|route| route.id.clone()) {
                        app.open_edit_route(&id);
                    }
                }
                "key_editor" => app.open_new_local_key(),
                "onboarding" => {
                    app.show_onboarding = true;
                    app.onboarding_step = 0;
                }
                "test" => {
                    if let Some(route) = app.config.routes.first() {
                        let id = route.id.clone();
                        app.test_key = Some(format!("route:{id}"));
                        let _ = app.cmd_tx.send(UiCommand::TestRoute(id));
                    }
                }
                "test_provider" => {
                    if let Some(provider) = app.config.providers.first() {
                        let id = provider.id.clone();
                        app.start_test("provider", &id);
                    }
                }
                "test_account" => {
                    if let Some(account) = app.config.accounts.first() {
                        let id = account.id.clone();
                        app.start_test("account", &id);
                    }
                }
                "confirm" => {
                    if let Some(provider) = app.config.providers.first() {
                        app.confirm = Some(ConfirmAction::DeleteProvider(provider.id.clone()));
                    }
                }
                _ => {}
            }
        }
        app
    }

    pub(crate) fn persist(&mut self) -> bool {
        let mut cfg = self.config.clone();
        if let Err(err) = self.store.save(&mut cfg) {
            self.toast(NoticeLevel::Error, format!("保存失败：{err}"));
            return false;
        }
        self.config = cfg.clone();
        if self
            .cmd_tx
            .send(UiCommand::SaveConfig(Box::new(cfg)))
            .is_err()
        {
            self.toast(NoticeLevel::Error, "运行时已退出，配置未生效。");
            return false;
        }
        true
    }

    pub(crate) fn persist_with_notice(&mut self, message: &str) {
        if self.persist() {
            self.toast(NoticeLevel::Success, message);
        }
    }

    pub(crate) fn toast(&mut self, level: NoticeLevel, text: impl Into<String>) {
        let id = self.toast_seq;
        self.toast_seq += 1;
        self.toasts.push(Toast {
            id,
            level,
            text: text.into(),
            until: Instant::now() + Duration::from_secs(4),
        });
    }

    pub(crate) fn copy(&mut self, ctx: &egui::Context, text: &str, label: &str) {
        ctx.copy_text(text.to_string());
        self.toast(NoticeLevel::Success, format!("{label} 已复制"));
    }

    pub(crate) fn start_test(&mut self, kind: &str, id: &str) {
        // Probe results for both speed kinds are stored under `speed:<id>`,
        // so the modal key must match (it used to wait forever for
        // `speed_provider:<id>` / `speed_account:<id>`).
        let key = if kind.starts_with("speed_") {
            format!("speed:{id}")
        } else {
            format!("{kind}:{id}")
        };
        self.test_key = Some(key);
        self.test_started_at = Some(Instant::now());
        let cmd = match kind {
            "provider" => UiCommand::TestProvider(id.to_string()),
            "account" => UiCommand::TestAccount(id.to_string()),
            "speed_account" => UiCommand::SpeedTestAccount(id.to_string()),
            "speed_provider" => UiCommand::SpeedTestProvider(id.to_string()),
            "route" => UiCommand::TestRoute(id.to_string()),
            "local_key" => UiCommand::TestLocalKey(id.to_string()),
            _ => return,
        };
        if self.cmd_tx.send(cmd).is_err() {
            self.toast(NoticeLevel::Error, "运行时已退出，无法测试。");
        }
    }

    /// Merge imported plans into the local library. A plan with the same name
    /// (case-insensitive) is replaced; everything else is appended.
    pub(crate) fn merge_price_plans(&mut self, plans: Vec<crate::config::PricePlan>) -> usize {
        let mut added = 0;
        for plan in plans {
            let plan = plan.normalized();
            if plan.name.is_empty() {
                continue;
            }
            match self
                .config
                .price_plans
                .iter_mut()
                .find(|existing| existing.name.eq_ignore_ascii_case(&plan.name))
            {
                Some(existing) => *existing = plan,
                None => {
                    self.config.price_plans.push(plan);
                    added += 1;
                }
            }
        }
        added
    }

    pub(crate) fn matches_search(&self, haystack: &str) -> bool {
        if self.search.trim().is_empty() {
            true
        } else {
            haystack
                .to_lowercase()
                .contains(&self.search.trim().to_lowercase())
        }
    }

    pub(crate) fn quit(&mut self, ctx: &egui::Context) {
        self.closing = true;
        let _ = self.cmd_tx.send(UiCommand::Shutdown);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    pub(crate) fn show_window(&mut self, ctx: &egui::Context) {
        self.closing = false;
        self.startup_minimize_at = None;
        self.signals.mark_hidden(false);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    /// Closing with "close to tray" enabled removes the window and its taskbar
    /// entry; the tray menu (or a second launch) brings it back.
    pub(crate) fn hide_to_tray(&mut self, ctx: &egui::Context) {
        self.signals.mark_hidden(true);
        if self.tray.is_some() && cfg!(windows) {
            // Keep the window technically visible (offscreen, toolwindow
            // style) so egui keeps running and the tray tooltip stays live.
            if !self.signals.hide_native() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
    }

    fn poll_snapshot(&mut self, ctx: &egui::Context) {
        let mut latest = None;
        while let Ok(snapshot) = self.snap_rx.try_recv() {
            for notice in &snapshot.notices {
                if notice.id > self.last_notice_id {
                    self.last_notice_id = notice.id;
                    self.toasts.push(Toast {
                        id: self.toast_seq,
                        level: notice.level,
                        text: notice.text.clone(),
                        until: Instant::now() + Duration::from_secs(4),
                    });
                    self.toast_seq += 1;
                }
            }
            latest = Some(snapshot);
        }
        if let Some(snapshot) = latest {
            self.snapshot = snapshot;
            ctx.request_repaint();
        }
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.closing {
            if self.config.ui.close_to_tray {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.hide_to_tray(ctx);
            } else {
                self.closing = true;
                let _ = self.cmd_tx.send(UiCommand::Shutdown);
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if escape {
            if self.editor.is_some() {
                self.editor = None;
            } else if self.confirm.is_some() {
                self.confirm = None;
            } else if self.test_key.is_some() {
                self.test_key = None;
            } else if self.show_onboarding {
                self.show_onboarding = false;
                self.config.ui.onboarding_done = true;
                self.persist();
            }
        }
        let new = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::N));
        if new {
            match self.page {
                Page::Providers => self.open_new_provider(),
                Page::Accounts => self.open_new_account(),
                Page::Routes => self.open_new_route_model(),
                Page::LocalKeys => self.open_new_local_key(),
                _ => {}
            }
        }
        let find = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F));
        if find {
            self.search_focus = true;
        }
    }

    fn sync_autostart(&mut self) {
        let desired = self.config.ui.autostart;
        if self.autostart_synced == Some(desired) {
            return;
        }
        self.autostart_synced = Some(desired);
        if let Err(err) = crate::autostart::set_enabled(desired) {
            self.toast(NoticeLevel::Warning, format!("开机自启设置失败：{err}"));
        }
    }

    /// The runtime owns live-rate refreshes and persists them itself. Adopt its
    /// newer table so an unrelated "save settings" cannot overwrite it with a
    /// stale copy.
    fn sync_plugin_settings(&mut self) {
        if self.snapshot.plugin_settings.disabled != self.config.plugins.disabled
            || self.snapshot.plugin_settings.allow_network != self.config.plugins.allow_network
            || self.snapshot.plugin_settings.allow_process != self.config.plugins.allow_process
        {
            self.config.plugins = self.snapshot.plugin_settings.clone();
        }
    }

    fn sync_live_fx(&mut self) {
        if self.snapshot.fx_updated_at > self.config.ui.fx_updated_at
            && !self.snapshot.fx_rates.is_empty()
        {
            self.config.ui.fx_rates = self.snapshot.fx_rates.clone();
            self.config.ui.fx_updated_at = self.snapshot.fx_updated_at;
            self.config.ui.fx_source = self.snapshot.fx_source.clone();
        }
    }

    fn prompt_update_if_needed(&mut self) {
        let update = self.snapshot.update.clone();
        if update.checking {
            return;
        }
        if self.update_waiting {
            self.update_waiting = false;
            if update.available {
                self.update_modal = true;
            }
            return;
        }
        if update.available && !update.dismissed && !update.latest.is_empty() {
            let latest = update.latest.clone();
            if self.update_prompted.as_deref() != Some(latest.as_str()) {
                self.update_prompted = Some(latest);
                self.update_modal = true;
            }
        }
    }

    fn show_update_modal(&mut self, ctx: &egui::Context) {
        if !self.update_modal {
            return;
        }
        let update = self.snapshot.update.clone();
        if update.latest.is_empty() {
            self.update_modal = false;
            return;
        }
        let mut close = false;
        let mut dismiss = false;
        let mut update_now = false;
        let closed = kit::modal_shell(
            ctx,
            Id::new("yoyo_update"),
            &format!("发现新版本 v{}", update.latest),
            &format!("当前版本 v{} · 更新后需要重新启动程序", update.current),
            620.0,
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    kit::badge(ui, &format!("v{}", update.current), theme::MUTED);
                    ui.label(
                        RichText::new(icons::ARROW_RIGHT)
                            .size(12.0)
                            .color(theme::muted(ui)),
                    );
                    kit::badge(ui, &format!("v{}", update.latest), theme::GREEN);
                    if update.checked_at > 0 {
                        ui.label(
                            RichText::new(format!(
                                "检查于 {}",
                                widgets::time_ago(update.checked_at, crate::config::now_ts())
                            ))
                            .size(10.0)
                            .color(theme::muted(ui)),
                        );
                    }
                });
                ui.add_space(8.0);
                egui::Frame::new()
                    .fill(theme::subtle_fill(ui))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("update_notes")
                            .max_height(260.0)
                            .show(ui, |ui| {
                                if update.notes.trim().is_empty() {
                                    ui.label(
                                        RichText::new("该版本没有提供发布说明。")
                                            .size(11.0)
                                            .color(theme::muted(ui)),
                                    );
                                } else {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(update.notes.trim()).size(11.0),
                                        )
                                        .wrap(),
                                    );
                                }
                            });
                    });
                ui.add_space(8.0);
                kit::info_row(
                    ui,
                    icons::INFO,
                    "「立即更新」会在浏览器中打开 Release 页面，由你确认下载与安装。",
                );
            },
            |ui| {
                if kit::primary_button(ui, icons::DOWNLOAD_SIMPLE, "立即更新").clicked() {
                    update_now = true;
                }
                if kit::secondary_button(ui, icons::BELL_SLASH, "不再提示").clicked() {
                    dismiss = true;
                }
                if kit::secondary_button(ui, "", "稍后").clicked() {
                    close = true;
                }
            },
        );
        if update_now {
            open_url(&update.url);
            self.update_modal = false;
            self.toast(NoticeLevel::Info, "已在浏览器中打开 Release 页面");
            return;
        }
        if dismiss {
            self.config.ui.update_dismissed_version = update.latest.clone();
            self.update_modal = false;
            self.persist_with_notice("已不再提示该版本的更新");
            return;
        }
        if closed || close {
            self.update_modal = false;
        }
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let title = if self.config.ui.show_taskbar_stats {
            let (_, label) = self.proxy_status();
            format!(
                "YoyoRelay · {} 活跃 · {} tokens · {}",
                self.snapshot.active_requests,
                widgets::format_count(self.snapshot.today.totals.total_tokens),
                label
            )
        } else {
            "YoyoRelay".to_string()
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }

    fn update_tray_stats(&mut self) {
        let (summary, detail) = if self.config.ui.show_taskbar_stats {
            let (_, label) = self.proxy_status();
            let totals = &self.snapshot.today.totals;
            let rate = if totals.requests > 0 {
                (totals.ok as f64 * 100.0 / totals.requests as f64).round() as u64
            } else {
                100
            };
            (
                format!(
                    "{} · {} 活跃 · {} tokens",
                    label,
                    self.snapshot.active_requests,
                    widgets::format_count(totals.total_tokens)
                ),
                format!(
                    "队列 {} · 今日 {} 请求 · 成功率 {}% · {}",
                    self.snapshot.queued_requests, totals.requests, rate, self.snapshot.endpoint
                ),
            )
        } else {
            (
                "本地 LLM 代理".to_string(),
                "打开主窗口查看请求与用量".to_string(),
            )
        };
        if let Some(tray) = self.tray.as_mut() {
            tray.update_stats(&summary, &detail);
        }
    }

    fn taskbar_stats_widget(&mut self, ctx: &egui::Context) {
        #[cfg(windows)]
        {
            if !self.config.ui.show_taskbar_stats {
                self.taskbar_badge = None;
                return;
            }
            let badge_cfg = self.config.ui.taskbar.normalized();
            let (status_color, status_label) = self.proxy_status();
            let running = self.snapshot.proxy.is_running();
            let totals = &self.snapshot.today.totals;
            let rate = if totals.requests > 0 {
                (totals.ok as f64 * 100.0 / totals.requests as f64).round() as u64
            } else {
                100
            };
            let mut parts: Vec<String> = Vec::new();
            if badge_cfg.show_status {
                parts.push(if running {
                    format!("代理{status_label}")
                } else {
                    "代理未运行".to_string()
                });
            }
            if badge_cfg.show_active {
                parts.push(format!("{} 活跃", self.snapshot.active_requests));
            }
            if badge_cfg.show_tokens {
                parts.push(format!(
                    "{} tok",
                    widgets::format_count(totals.total_tokens)
                ));
            }
            if badge_cfg.show_requests {
                parts.push(format!("{} 请求", widgets::format_count(totals.requests)));
            }
            if badge_cfg.show_queued {
                parts.push(format!("队列 {}", self.snapshot.queued_requests));
            }
            if badge_cfg.show_success {
                parts.push(format!("{rate}%"));
            }
            if badge_cfg.show_cost {
                parts.push(widgets::format_cost(totals.cost_micros));
            }
            if badge_cfg.show_speed {
                parts.push(format!(
                    "{} tok/s",
                    widgets::format_count(self.snapshot.tokens_per_sec.round() as u64)
                ));
            }
            let text = if !badge_cfg.has_content() || parts.is_empty() {
                "YoyoRelay".to_string()
            } else {
                parts.join(" · ")
            };
            let dark = ctx
                .system_theme()
                .map(|theme| theme == egui::Theme::Dark)
                .unwrap_or(false);
            if self.taskbar_badge.is_none() {
                self.taskbar_badge = crate::taskbar::TaskbarBadge::new();
            }
            if let Some(badge) = self.taskbar_badge.as_mut() {
                let dpi = ctx
                    .input(|i| i.viewport().native_pixels_per_point)
                    .unwrap_or(1.0);
                badge.update(
                    &text,
                    (status_color.r(), status_color.g(), status_color.b()),
                    dark,
                    dpi,
                    &crate::taskbar::BadgeStyle {
                        scale: badge_cfg.scale,
                        offset_x: badge_cfg.offset_x,
                        offset_y: badge_cfg.offset_y,
                        gap: badge_cfg.gap,
                        opacity: badge_cfg.opacity,
                        show_dot: badge_cfg.show_dot,
                        transparent_bg: badge_cfg.transparent_bg,
                        text_color: if badge_cfg.custom_text_color {
                            crate::ui::settings::parse_hex_color(&badge_cfg.text_color)
                        } else {
                            None
                        },
                    },
                );
            }
        }
        #[cfg(not(windows))]
        {
            let _ = ctx;
        }
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        let dark = theme::is_dark(ctx, self.config.ui.theme);
        if self.last_dark != Some(dark) {
            theme::apply(ctx, dark);
            self.last_dark = Some(dark);
        }
    }

    fn screen_width(&self, ctx: &egui::Context) -> f32 {
        ctx.content_rect().width()
    }

    fn sidebar_width(&self, ctx: &egui::Context) -> f32 {
        if self.screen_width(ctx) < 960.0 {
            64.0
        } else {
            198.0
        }
    }

    // -- custom title bar ---------------------------------------------------

    fn title_bar(&mut self, ctx: &egui::Context) {
        let t = theme::tokens_ctx(ctx);
        let sidebar_w = self.sidebar_width(ctx);
        let height = 60.0;
        egui::TopBottomPanel::top("title_bar")
            .exact_height(height)
            .frame(
                egui::Frame::new()
                    .fill(t.sidebar)
                    .inner_margin(egui::Margin::ZERO),
            )
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                ui.painter().rect_filled(
                    Rect::from_min_max(Pos2::new(rect.left() + sidebar_w, rect.top()), rect.max),
                    egui::CornerRadius::ZERO,
                    t.bg,
                );
                // Brand zone. Painted directly so the mark and the two text
                // lines are always optically centered in the title bar,
                // independent of layout cross-axis quirks.
                let brand_rect = Rect::from_min_size(rect.min, Vec2::new(sidebar_w, height));
                // A tiny optical lift compensates for the cap-height of the
                // name so the lockup reads as centered, not top-aligned.
                let center_y = brand_rect.center().y + 2.0;
                let logo_size = 32.0;
                let logo_rect = Rect::from_center_size(
                    Pos2::new(brand_rect.left() + 15.0 + logo_size / 2.0, center_y),
                    Vec2::splat(logo_size),
                );
                ui.painter().image(
                    self.logo.id(),
                    logo_rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                if sidebar_w > 100.0 {
                    let text_x = logo_rect.right() + 9.0;
                    ui.painter().text(
                        Pos2::new(text_x, center_y - 8.5),
                        Align2::LEFT_CENTER,
                        "YoyoRelay",
                        egui::FontId::proportional(14.5),
                        t.text,
                    );
                    ui.painter().text(
                        Pos2::new(text_x, center_y + 9.0),
                        Align2::LEFT_CENTER,
                        "本地多上游代理",
                        egui::FontId::proportional(9.5),
                        t.muted,
                    );
                }
                // Content zone
                let content_rect =
                    Rect::from_min_max(Pos2::new(rect.left() + sidebar_w, rect.top()), rect.max);
                let content_inner = content_rect.shrink2(Vec2::new(16.0, 0.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(content_inner), |ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(content_inner.width(), height),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.label(RichText::new("工作台").size(11.0).color(t.muted));
                            ui.label(RichText::new("/").size(10.5).color(t.muted));
                            ui.label(
                                RichText::new(self.page.label())
                                    .size(11.0)
                                    .strong()
                                    .color(t.text),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let close_hint = if self.config.ui.close_to_tray {
                                    if cfg!(windows) {
                                        "隐藏到系统托盘"
                                    } else {
                                        "最小化到任务栏"
                                    }
                                } else {
                                    "退出 YoyoRelay"
                                };
                                if kit::window_button(ui, icons::X, close_hint, true).clicked() {
                                    if self.config.ui.close_to_tray {
                                        self.hide_to_tray(ctx);
                                    } else {
                                        self.quit(ctx);
                                    }
                                }
                                let maximized =
                                    ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                                if kit::window_button(
                                    ui,
                                    if maximized {
                                        icons::COPY
                                    } else {
                                        icons::SQUARE
                                    },
                                    if maximized { "还原" } else { "最大化" },
                                    false,
                                )
                                .clicked()
                                {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(
                                        !maximized,
                                    ));
                                }
                                if kit::window_button(ui, icons::MINUS, "最小化", false).clicked()
                                {
                                    self.signals.mark_hidden(true);
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                                }
                                ui.add_space(4.0);
                                let (color, label) = self.proxy_status();
                                kit::status_pill(ui, label, color);
                                let update = self.snapshot.update.clone();
                                let update_icon = if update.checking {
                                    icons::SPINNER
                                } else {
                                    icons::ARROWS_CLOCKWISE
                                };
                                let response = kit::icon_button(
                                    ui,
                                    update_icon,
                                    if update.available {
                                        "发现新版本，点击查看"
                                    } else {
                                        "检查更新"
                                    },
                                );
                                if update.available && !update.dismissed {
                                    let dot = response.rect.right_top() + Vec2::new(-2.0, 2.0);
                                    ui.painter().circle_filled(dot, 3.5, theme::RED);
                                }
                                if response.clicked() {
                                    let _ =
                                        self.cmd_tx.send(UiCommand::CheckUpdate { manual: true });
                                    if update.available {
                                        self.update_modal = true;
                                    } else {
                                        self.update_waiting = true;
                                        self.toast(NoticeLevel::Info, "正在检查更新…");
                                    }
                                }
                                if self.page.needs_search() {
                                    let search_width = if content_rect.width() > 760.0 {
                                        180.0
                                    } else {
                                        120.0
                                    };
                                    egui::Frame::new()
                                        .fill(t.card)
                                        .stroke(Stroke::new(1.0_f32, t.card_border))
                                        .corner_radius(8)
                                        .inner_margin(egui::Margin::symmetric(8, 3))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new(icons::MAGNIFYING_GLASS)
                                                        .size(12.0)
                                                        .color(t.muted),
                                                );
                                                let response = ui.add(
                                                    egui::TextEdit::singleline(&mut self.search)
                                                        .frame(false)
                                                        .hint_text("搜索")
                                                        .desired_width(search_width),
                                                );
                                                if self.search_focus {
                                                    response.request_focus();
                                                    self.search_focus = false;
                                                }
                                            });
                                        });
                                }
                                if content_rect.width() > 900.0 {
                                    ui.label(
                                        RichText::new(
                                            chrono::Local::now().format("%Y.%m.%d").to_string(),
                                        )
                                        .size(10.0)
                                        .color(t.muted),
                                    );
                                }
                            });
                        },
                    );
                });

                // A single hairline keeps the header crisp without boxing it in.
                ui.painter().line_segment(
                    [
                        Pos2::new(rect.left(), rect.bottom() - 0.5),
                        Pos2::new(rect.right(), rect.bottom() - 0.5),
                    ],
                    Stroke::new(1.0_f32, t.card_border),
                );

                // Drag region excludes the right-side controls.
                let drag_left = rect.left() + sidebar_w;
                let drag_right = (rect.right() - 420.0).max(drag_left + 40.0);
                let drag_rect = Rect::from_min_max(
                    Pos2::new(drag_left, rect.top()),
                    Pos2::new(drag_right, rect.bottom()),
                );
                let response = ui.interact(
                    drag_rect,
                    Id::new("title_bar_drag"),
                    Sense::click_and_drag(),
                );
                if response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if response.double_clicked() {
                    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
            });
    }

    fn resize_handles(&self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
            return;
        }
        let screen = ctx.content_rect();
        let thickness = 5.0;
        let arrows: [(&str, Rect, CursorIcon, egui::ResizeDirection); 8] = [
            (
                "n",
                Rect::from_min_max(
                    Pos2::new(screen.left() + 8.0, screen.top()),
                    Pos2::new(screen.right() - 8.0, screen.top() + thickness),
                ),
                CursorIcon::ResizeNorth,
                egui::ResizeDirection::North,
            ),
            (
                "s",
                Rect::from_min_max(
                    Pos2::new(screen.left() + 8.0, screen.bottom() - thickness),
                    Pos2::new(screen.right() - 8.0, screen.bottom()),
                ),
                CursorIcon::ResizeSouth,
                egui::ResizeDirection::South,
            ),
            (
                "w",
                Rect::from_min_max(
                    Pos2::new(screen.left(), screen.top() + 8.0),
                    Pos2::new(screen.left() + thickness, screen.bottom() - 8.0),
                ),
                CursorIcon::ResizeWest,
                egui::ResizeDirection::West,
            ),
            (
                "e",
                Rect::from_min_max(
                    Pos2::new(screen.right() - thickness, screen.top() + 8.0),
                    Pos2::new(screen.right(), screen.bottom() - 8.0),
                ),
                CursorIcon::ResizeEast,
                egui::ResizeDirection::East,
            ),
            (
                "nw",
                Rect::from_min_max(
                    screen.min,
                    Pos2::new(screen.left() + 10.0, screen.top() + 10.0),
                ),
                CursorIcon::ResizeNorthWest,
                egui::ResizeDirection::NorthWest,
            ),
            (
                "ne",
                Rect::from_min_max(
                    Pos2::new(screen.right() - 10.0, screen.top()),
                    Pos2::new(screen.right(), screen.top() + 10.0),
                ),
                CursorIcon::ResizeNorthEast,
                egui::ResizeDirection::NorthEast,
            ),
            (
                "sw",
                Rect::from_min_max(
                    Pos2::new(screen.left(), screen.bottom() - 10.0),
                    Pos2::new(screen.left() + 10.0, screen.bottom()),
                ),
                CursorIcon::ResizeSouthWest,
                egui::ResizeDirection::SouthWest,
            ),
            (
                "se",
                Rect::from_min_max(
                    Pos2::new(screen.right() - 10.0, screen.bottom() - 10.0),
                    screen.max,
                ),
                CursorIcon::ResizeSouthEast,
                egui::ResizeDirection::SouthEast,
            ),
        ];
        egui::Area::new(Id::new("resize_handles"))
            .fixed_pos(Pos2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for (name, rect, cursor, direction) in arrows {
                    let response = ui.interact(rect, Id::new(("resize", name)), Sense::drag());
                    if response.hovered() {
                        ctx.set_cursor_icon(cursor);
                    }
                    if response.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
                    }
                }
            });
    }

    // -- sidebar ------------------------------------------------------------

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let t = theme::tokens_ctx(ui.ctx());
        let compact = self.sidebar_width(ui.ctx()) < 100.0;
        ui.add_space(8.0);
        if compact {
            for page in Page::all() {
                if kit::nav_rail_item(ui, page.icon(), page.label(), self.page == page).clicked() {
                    self.page = page;
                    self.search.clear();
                }
                ui.add_space(2.0);
            }
        } else {
            kit::nav_label(ui, "工作台");
            if kit::nav_item(ui, icons::HOUSE, "概览", self.page == Page::Overview, None).clicked()
            {
                self.page = Page::Overview;
                self.search.clear();
            }
            kit::nav_label(ui, "配置");
            if kit::nav_item(
                ui,
                icons::HARD_DRIVES,
                "供应商",
                self.page == Page::Providers,
                Some(self.config.providers.len().to_string()),
            )
            .clicked()
            {
                self.page = Page::Providers;
                self.search.clear();
            }
            if kit::nav_item(
                ui,
                icons::USERS,
                "账号",
                self.page == Page::Accounts,
                Some(self.config.accounts.len().to_string()),
            )
            .clicked()
            {
                self.page = Page::Accounts;
                self.search.clear();
            }
            if kit::nav_item(
                ui,
                icons::GIT_BRANCH,
                "路由",
                self.page == Page::Routes,
                Some(self.config.routes.len().to_string()),
            )
            .clicked()
            {
                self.page = Page::Routes;
                self.search.clear();
            }
            if kit::nav_item(
                ui,
                icons::PUZZLE_PIECE,
                "插件",
                self.page == Page::Plugins,
                Some(self.snapshot.plugins.len().to_string()),
            )
            .clicked()
            {
                self.page = Page::Plugins;
                self.search.clear();
            }
            if kit::nav_item(
                ui,
                icons::KEY,
                "本地 Key",
                self.page == Page::LocalKeys,
                Some(self.config.local_keys.len().to_string()),
            )
            .clicked()
            {
                self.page = Page::LocalKeys;
                self.search.clear();
            }
            kit::nav_label(ui, "观测");
            if kit::nav_item(ui, icons::CHART_BAR, "用量", self.page == Page::Usage, None).clicked()
            {
                self.page = Page::Usage;
                self.search.clear();
            }
            if kit::nav_item(
                ui,
                icons::LIST_BULLETS,
                "调用明细",
                self.page == Page::Calls,
                Some(self.snapshot.calls.total.to_string()),
            )
            .clicked()
            {
                self.page = Page::Calls;
                self.search.clear();
            }
            if kit::nav_item(ui, icons::WALLET, "余额", self.page == Page::Balances, None).clicked()
            {
                self.page = Page::Balances;
                self.search.clear();
            }
        }

        ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
            ui.add_space(8.0);
            if compact {
                if kit::nav_rail_item(ui, icons::GEAR_SIX, "设置", self.page == Page::Settings)
                    .clicked()
                {
                    self.page = Page::Settings;
                    self.search.clear();
                }
                ui.add_space(4.0);
                let (color, label) = self.proxy_status();
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
                ui.painter().rect_filled(rect, 8.0, t.hover);
                ui.painter()
                    .circle_filled(Pos2::new(rect.center().x, rect.center().y), 4.0, color);
                if response.on_hover_text(format!("代理{label}")).clicked() {
                    let running = self.snapshot.proxy.is_running();
                    let _ = self.cmd_tx.send(if running {
                        UiCommand::StopProxy
                    } else {
                        UiCommand::StartProxy
                    });
                }
            } else {
                if kit::nav_item(
                    ui,
                    icons::GEAR_SIX,
                    "设置",
                    self.page == Page::Settings,
                    None,
                )
                .clicked()
                {
                    self.page = Page::Settings;
                    self.search.clear();
                }
                ui.add_space(6.0);
                let (color, label) = self.proxy_status();
                egui::Frame::new()
                    .fill(t.card)
                    .stroke(Stroke::new(1.0_f32, t.card_border))
                    .corner_radius(12)
                    .inner_margin(egui::Margin::symmetric(11, 9))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{} 活跃", self.snapshot.active_requests))
                                    .size(9.5)
                                    .color(t.muted),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "{} tokens",
                                        widgets::format_count(
                                            self.snapshot.today.totals.total_tokens
                                        )
                                    ))
                                    .size(9.5)
                                    .color(t.muted),
                                );
                            });
                        });
                        ui.add_space(3.0);
                        let endpoint = self
                            .snapshot
                            .endpoint
                            .trim_start_matches("http://")
                            .trim_start_matches("https://")
                            .to_string();
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(if endpoint.is_empty() {
                                    "—".to_string()
                                } else {
                                    endpoint.clone()
                                })
                                .monospace()
                                .size(9.0)
                                .color(t.muted),
                            );
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(icons::COPY).size(10.5).color(t.muted),
                                    )
                                    .frame(false)
                                    .min_size(Vec2::splat(16.0)),
                                )
                                .on_hover_text("复制端点")
                                .clicked()
                            {
                                ui.ctx().copy_text(self.snapshot.endpoint.clone());
                                self.toast(NoticeLevel::Success, "端点已复制");
                            }
                        });
                        let lan_endpoints: Vec<_> = self
                            .snapshot
                            .endpoints
                            .iter()
                            .filter(|endpoint| endpoint.lan)
                            .take(2)
                            .cloned()
                            .collect();
                        for view in &lan_endpoints {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(&view.url)
                                        .monospace()
                                        .size(8.5)
                                        .color(t.muted),
                                );
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(icons::COPY).size(9.5).color(t.muted),
                                        )
                                        .frame(false)
                                        .min_size(Vec2::splat(14.0)),
                                    )
                                    .on_hover_text("复制局域网端点")
                                    .clicked()
                                {
                                    ui.ctx().copy_text(view.url.clone());
                                    self.toast(NoticeLevel::Success, "局域网端点已复制");
                                }
                            });
                        }
                        ui.add_space(3.0);
                        ui.horizontal(|ui| {
                            kit::status_pill(ui, &format!("代理{label}"), color);
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let running = self.snapshot.proxy.is_running();
                                if kit::icon_button(
                                    ui,
                                    if running { icons::STOP } else { icons::PLAY },
                                    if running {
                                        "停止代理"
                                    } else {
                                        "启动代理"
                                    },
                                )
                                .clicked()
                                {
                                    let _ = self.cmd_tx.send(if running {
                                        UiCommand::StopProxy
                                    } else {
                                        UiCommand::StartProxy
                                    });
                                }
                            });
                        });
                    });
            }
            ui.separator();
            ui.add_space(2.0);
        });
    }

    fn proxy_status(&self) -> (Color32, &'static str) {
        match &self.snapshot.proxy {
            ProxyStatus::Running => (theme::GREEN, "运行中"),
            ProxyStatus::Starting => (theme::ORANGE, "启动中"),
            ProxyStatus::Stopped => (theme::MUTED, "已停止"),
            ProxyStatus::Error(_) => (theme::RED, "异常"),
        }
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        let t = theme::tokens_ctx(ctx);
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(28.0)
            .frame(
                egui::Frame::new()
                    .fill(t.sidebar)
                    .inner_margin(egui::Margin::symmetric(16, 4))
                    .stroke(Stroke::new(1.0_f32, t.card_border)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let color = if self.snapshot.breaker.open {
                        t.red
                    } else {
                        t.green
                    };
                    ui.label(
                        RichText::new(if self.snapshot.breaker.open {
                            icons::SHIELD_WARNING
                        } else {
                            icons::SHIELD_CHECK
                        })
                        .size(12.0)
                        .color(color),
                    );
                    let breaker = if self.snapshot.breaker.open {
                        format!(
                            "熔断中 · 剩余 {}s",
                            (self.snapshot.breaker.open_until - now_ts()).max(0)
                        )
                    } else {
                        format!(
                            "熔断保护 · 连续失败 {}/{}",
                            self.snapshot.breaker.consecutive_failures,
                            self.snapshot.breaker.threshold
                        )
                    };
                    ui.label(RichText::new(breaker).size(9.5).color(t.muted));
                    ui.label(RichText::new("·").size(9.5).color(t.muted));
                    ui.label(
                        RichText::new(format!(
                            "{} 活跃 / {} 排队",
                            self.snapshot.active_requests, self.snapshot.queued_requests
                        ))
                        .size(9.5)
                        .color(t.muted),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "更新 {}",
                                widgets::format_time(self.snapshot.updated_at)
                            ))
                            .size(9.0)
                            .color(t.muted),
                        );
                    });
                });
            });
    }

    fn central(&mut self, ctx: &egui::Context) {
        let t = theme::tokens_ctx(ctx);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(t.bg)
                    .inner_margin(egui::Margin::symmetric(22, 16)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("central_content")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let max_w = 1260.0_f32;
                        let avail = ui.available_width();
                        if avail > max_w {
                            ui.add_space((avail - max_w) / 2.0);
                        }
                        let width = avail.min(max_w);
                        ui.allocate_ui_with_layout(
                            Vec2::new(width, 0.0),
                            Layout::top_down(Align::Min),
                            |ui| {
                                ui.set_width(width);
                                match self.page {
                                    Page::Overview => self.page_overview(ui),
                                    Page::Providers => self.page_providers(ui),
                                    Page::Accounts => self.page_accounts(ui),
                                    Page::Routes => self.page_routes(ui),
                                    Page::Plugins => self.page_plugins(ui),
                                    Page::LocalKeys => self.page_local_keys(ui),
                                    Page::Usage => self.page_usage(ui),
                                    Page::Calls => self.page_calls(ui),
                                    Page::Balances => self.page_balances(ui),
                                    Page::Settings => self.page_settings(ui),
                                }
                                ui.add_space(24.0);
                            },
                        );
                    });
            });
    }

    fn show_toasts(&mut self, ctx: &egui::Context) {
        self.toasts.retain(|t| t.until > Instant::now());
        if self.toasts.is_empty() {
            return;
        }
        let t = theme::tokens_ctx(ctx);
        egui::Area::new(Id::new("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, [-18.0, -42.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for toast in self.toasts.iter().rev().take(3) {
                    let color = match toast.level {
                        NoticeLevel::Success => t.green,
                        NoticeLevel::Warning => t.amber,
                        NoticeLevel::Error => t.red,
                        NoticeLevel::Info => t.blue,
                    };
                    egui::Frame::new()
                        .fill(t.card)
                        .stroke(Stroke::new(1.0_f32, t.card_border))
                        .corner_radius(10)
                        .shadow(t.card_shadow)
                        .inner_margin(egui::Margin::symmetric(12, 9))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(6.0), Sense::hover());
                                ui.painter().circle_filled(rect.center(), 3.0, color);
                                ui.add_space(4.0);
                                ui.label(RichText::new(&toast.text).size(11.5).color(t.text));
                            });
                        });
                    ui.add_space(6.0);
                }
            });
        ctx.request_repaint_after(Duration::from_millis(300));
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let _ = &frame;
        if !self.ctx_attached {
            self.signals.attach(ctx);
            self.signals.attach_quit(self.cmd_tx.clone());
            self.ctx_attached = true;
        }
        if !self.native_attached {
            #[cfg(windows)]
            {
                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                if let Ok(handle) = frame.window_handle() {
                    if let RawWindowHandle::Win32(win) = handle.as_raw() {
                        self.signals.attach_window(win.hwnd.get());
                        if !self.native_frame_applied {
                            crate::windowfx::apply_native_frame(win.hwnd.get() as isize);
                            self.native_frame_applied = true;
                        }
                    }
                }
            }
            self.native_attached = true;
        }
        if !self.tray_ready {
            // Deferred until the first frame: macOS needs the event loop to be
            // running before the status item is created.
            self.tray = crate::tray::install(self.signals.clone());
            self.tray_ready = true;
        }
        if self.signals.take_quit() {
            self.quit(ctx);
        } else if self.signals.take_show() {
            self.show_window(ctx);
        }
        if !ctx.input(|i| i.viewport().minimized.unwrap_or(false)) {
            self.signals.mark_hidden(false);
        }
        self.poll_snapshot(ctx);
        self.prompt_update_if_needed();
        self.sync_plugin_settings();
        self.sync_live_fx();
        self.apply_theme(ctx);
        self.sync_autostart();
        self.update_window_title(ctx);
        self.update_tray_stats();
        if let Some(at) = self.startup_minimize_at {
            let now = Instant::now();
            if now >= at {
                self.startup_minimize_at = None;
                self.hide_to_tray(ctx);
            } else {
                ctx.request_repaint_after(at - now);
            }
        }
        {
            let native = ctx
                .input(|i| i.viewport().native_pixels_per_point)
                .unwrap_or_else(|| ctx.pixels_per_point());
            let desired = if self.config.ui.ui_scale > 0.05 {
                self.config.ui.ui_scale.clamp(0.7, 1.5)
            } else if native > 1.45 {
                (1.4 / native).clamp(0.7, 1.0)
            } else {
                1.0
            };
            if (desired - self.last_zoom).abs() > 0.01 {
                self.last_zoom = desired;
                ctx.set_zoom_factor(desired);
                ctx.request_repaint();
            }
        }
        self.taskbar_stats_widget(ctx);
        self.handle_close(ctx);
        self.handle_shortcuts(ctx);

        let t = theme::tokens_ctx(ctx);
        self.resize_handles(ctx);
        self.title_bar(ctx);

        let sidebar_w = self.sidebar_width(ctx);
        egui::SidePanel::left("sidebar")
            .exact_width(sidebar_w)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(t.sidebar)
                    .inner_margin(egui::Margin::symmetric(10, 4))
                    .stroke(Stroke::new(1.0_f32, t.card_border)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("sidebar_scroll")
                    .show(ui, |ui| self.sidebar(ui));
            });

        self.status_bar(ctx);
        self.central(ctx);

        self.show_editor(ctx);
        self.show_price_plan_modal(ctx);
        self.show_update_modal(ctx);
        self.show_plugin_ui(ctx);
        self.show_plugin_docs(ctx);
        self.show_confirm(ctx);
        self.show_test_modal(ctx);
        self.show_onboarding(ctx);
        self.show_toasts(ctx);

        ctx.request_repaint_after(Duration::from_secs(1));
        if !self.snapshot.running_tests.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(180));
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if !self.closing {
            let _ = self.cmd_tx.send(UiCommand::Shutdown);
        }
    }
}

/// Open a URL in the platform's default browser.
pub(crate) fn open_url(url: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}
