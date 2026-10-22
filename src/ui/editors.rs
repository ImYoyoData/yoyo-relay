use egui::{ComboBox, Id, RichText, TextEdit};
use egui_phosphor::regular as icons;

use crate::config::{
    new_id, now_ts, Account, AuthMode, BalanceConfig, BalanceMode, ModelRoute, PrefixRule,
    Protocol, ProtocolStrategyMode, ProviderKind, QuotaConfig, QuotaWindow, RouteMode,
};
use crate::snapshot::NoticeLevel;

use super::{theme, widgets};
use super::{
    AccountDraft, App, ConfirmAction, Editor, LocalKeyDraft, ManualBalanceDraft, Page,
    ProviderDraft, RouteDraft,
};

impl App {
    // -- openers ------------------------------------------------------------

    pub(crate) fn open_new_provider(&mut self) {
        let provider = self.store.preset_provider("openai");
        self.editor = Some(Editor::Provider(ProviderDraft {
            is_new: true,
            preset: "openai".into(),
            provider,
        }));
    }

    pub(crate) fn open_edit_provider(&mut self, id: &str) {
        if let Some(provider) = self.config.provider(id).cloned() {
            let preset = provider.preset.clone().unwrap_or_else(|| "custom".into());
            self.editor = Some(Editor::Provider(ProviderDraft {
                is_new: false,
                preset,
                provider,
            }));
        }
    }

    pub(crate) fn open_new_account(&mut self) {
        let Some(provider) = self.config.providers.first().cloned() else {
            self.toast(NoticeLevel::Warning, "请先添加供应商。");
            self.page = Page::Providers;
            return;
        };
        self.open_new_account_for(&provider.id);
    }

    pub(crate) fn open_new_account_for(&mut self, provider_id: &str) {
        let Some(provider) = self.config.provider(provider_id).cloned() else {
            self.toast(NoticeLevel::Warning, "供应商不存在。");
            return;
        };
        let account = Account {
            id: new_id(),
            provider_id: provider.id,
            name: "新账号".into(),
            enabled: true,
            max_concurrency: 4,
            created_at: now_ts(),
            balance: BalanceConfig {
                mode: BalanceMode::Unavailable,
                ..BalanceConfig::default()
            },
            ..Account::default()
        };
        self.editor = Some(Editor::Account(AccountDraft {
            is_new: true,
            account,
            api_key_input: String::new(),
            quota_enabled: false,
            limit_tokens: 0,
            limit_requests: 0,
            rolling_hours: 5,
        }));
    }

    pub(crate) fn open_edit_account(&mut self, id: &str) {
        if let Some(account) = self.config.account(id).cloned() {
            let quota = account.quota.clone();
            self.editor = Some(Editor::Account(AccountDraft {
                is_new: false,
                api_key_input: String::new(),
                quota_enabled: quota.is_some(),
                limit_tokens: quota.as_ref().and_then(|q| q.limit_tokens).unwrap_or(0),
                limit_requests: quota.as_ref().and_then(|q| q.limit_requests).unwrap_or(0),
                rolling_hours: quota.as_ref().map(|q| q.rolling_hours).unwrap_or(5),
                account,
            }));
        }
    }

    pub(crate) fn open_new_route(&mut self) {
        let Some(provider) = self.config.providers.first().cloned() else {
            self.toast(NoticeLevel::Warning, "请先添加供应商。");
            self.page = Page::Providers;
            return;
        };
        let client_model = self
            .config
            .client_models()
            .into_iter()
            .next()
            .unwrap_or_default();
        let route = ModelRoute {
            id: new_id(),
            client_model,
            provider_id: provider.id,
            upstream_model: provider.test_model.clone().unwrap_or_default(),
            priority: 10,
            created_at: now_ts(),
            ..ModelRoute::default()
        };
        self.editor = Some(Editor::Route(RouteDraft {
            is_new: true,
            capabilities_text: route.capabilities.join(","),
            fallback_text: String::new(),
            max_context_enabled: route.max_context_tokens.is_some(),
            max_context: route.max_context_tokens.unwrap_or(0),
            route,
        }));
    }

    pub(crate) fn open_edit_route(&mut self, id: &str) {
        if let Some(route) = self.config.route(id).cloned() {
            let capabilities_text = route.capabilities.join(",");
            let fallback_text = route.fallback_client_model.clone().unwrap_or_default();
            let max_context_enabled = route.max_context_tokens.is_some();
            let max_context = route.max_context_tokens.unwrap_or(0);
            self.editor = Some(Editor::Route(RouteDraft {
                is_new: false,
                route,
                capabilities_text,
                fallback_text,
                max_context_enabled,
                max_context,
            }));
        }
    }

    pub(crate) fn open_new_local_key(&mut self) {
        self.editor = Some(Editor::LocalKey(LocalKeyDraft {
            name: "新 Key".into(),
            allowed_models_text: String::new(),
            max_tokens: 0,
            max_requests: 0,
            enabled: true,
        }));
    }

    pub(crate) fn open_manual_balance(&mut self, account_id: &str) {
        let account = self.config.account(account_id);
        let amount = account
            .map(|a| a.balance.manual_amount)
            .filter(|v| *v > 0.0)
            .or_else(|| self.snapshot.account(account_id).and_then(|a| a.balance))
            .unwrap_or(0.0);
        self.editor = Some(Editor::ManualBalance(ManualBalanceDraft {
            account_id: account_id.to_string(),
            amount,
        }));
    }

    // -- save actions -------------------------------------------------------

    fn save_provider(&mut self, draft: &ProviderDraft, test: bool) -> bool {
        let mut provider = draft.provider.clone();
        provider.name = provider.name.trim().to_string();
        provider.base_url = provider.base_url.trim().to_string();
        if provider.name.is_empty() {
            self.toast(NoticeLevel::Error, "供应商名称不能为空。");
            return false;
        }
        if !(provider.base_url.starts_with("http://") || provider.base_url.starts_with("https://"))
        {
            self.toast(
                NoticeLevel::Error,
                "Base URL 必须以 http:// 或 https:// 开头。",
            );
            return false;
        }
        let backup = self.config.clone();
        if draft.is_new {
            provider.id = new_id();
            provider.created_at = now_ts();
            self.config.providers.push(provider.clone());
        } else if let Some(existing) = self
            .config
            .providers
            .iter_mut()
            .find(|p| p.id == provider.id)
        {
            *existing = provider.clone();
        } else {
            self.config.providers.push(provider.clone());
        }
        if !self.persist() {
            self.config = backup;
            return false;
        }
        self.toast(NoticeLevel::Success, "供应商已保存");
        if test {
            self.start_test("provider", &provider.id);
        }
        true
    }

    fn save_account(&mut self, draft: &AccountDraft, test: bool) -> bool {
        let mut account = draft.account.clone();
        account.name = account.name.trim().to_string();
        if account.name.is_empty() {
            self.toast(NoticeLevel::Error, "账号名称不能为空。");
            return false;
        }
        if !self
            .config
            .providers
            .iter()
            .any(|p| p.id == account.provider_id)
        {
            self.toast(NoticeLevel::Error, "请选择所属供应商。");
            return false;
        }
        account.quota = if draft.quota_enabled {
            Some(QuotaConfig {
                window: account
                    .quota
                    .as_ref()
                    .map(|q| q.window)
                    .unwrap_or(QuotaWindow::Rolling),
                rolling_hours: draft.rolling_hours.max(1),
                limit_tokens: (draft.limit_tokens > 0).then_some(draft.limit_tokens),
                limit_requests: (draft.limit_requests > 0).then_some(draft.limit_requests),
            })
        } else {
            None
        };
        if !draft.api_key_input.trim().is_empty() {
            account.api_key = draft.api_key_input.trim().to_string();
        }
        account.name = account.name.trim().to_string();
        let backup = self.config.clone();
        if draft.is_new {
            if account.id.is_empty() {
                account.id = new_id();
            }
            if account.created_at == 0 {
                account.created_at = now_ts();
            }
            self.config.accounts.push(account.clone());
        } else if let Some(existing) = self.config.accounts.iter_mut().find(|a| a.id == account.id)
        {
            *existing = account.clone();
        } else {
            self.config.accounts.push(account.clone());
        }
        if !self.persist() {
            self.config = backup;
            return false;
        }
        self.toast(NoticeLevel::Success, "账号已保存");
        if test {
            self.start_test("account", &account.id);
        }
        true
    }

    fn save_route(&mut self, draft: &RouteDraft, test: bool) -> bool {
        let mut route = draft.route.clone();
        route.client_model = route.client_model.trim().to_string();
        route.upstream_model = route.upstream_model.trim().to_string();
        if route.client_model.is_empty() || route.upstream_model.is_empty() {
            self.toast(NoticeLevel::Error, "客户端模型和上游模型都不能为空。");
            return false;
        }
        if !self
            .config
            .providers
            .iter()
            .any(|p| p.id == route.provider_id)
        {
            self.toast(NoticeLevel::Error, "请选择供应商。");
            return false;
        }
        route.capabilities = draft
            .capabilities_text
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        route.fallback_client_model = {
            let value = draft.fallback_text.trim();
            if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            }
        };
        route.max_context_tokens = if draft.max_context_enabled {
            Some(draft.max_context)
        } else {
            None
        };
        let backup = self.config.clone();
        if draft.is_new {
            if route.id.is_empty() {
                route.id = new_id();
            }
            if route.created_at == 0 {
                route.created_at = now_ts();
            }
            self.config.routes.push(route.clone());
        } else if let Some(existing) = self.config.routes.iter_mut().find(|r| r.id == route.id) {
            *existing = route.clone();
        } else {
            self.config.routes.push(route.clone());
        }
        if !self.persist() {
            self.config = backup;
            return false;
        }
        self.toast(NoticeLevel::Success, "路由已保存");
        if test {
            self.start_test("route", &route.id);
        }
        true
    }

    fn create_local_key(&mut self, draft: &LocalKeyDraft) -> bool {
        let name = draft.name.trim();
        if name.is_empty() {
            self.toast(NoticeLevel::Error, "Key 名称不能为空。");
            return false;
        }
        let mut key = match self.store.new_local_key(name) {
            Ok(k) => k,
            Err(err) => {
                self.toast(NoticeLevel::Error, format!("生成 Key 失败：{err}"));
                return false;
            }
        };
        key.allowed_models = draft
            .allowed_models_text
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        key.max_tokens = (draft.max_tokens > 0).then_some(draft.max_tokens);
        key.max_requests = (draft.max_requests > 0).then_some(draft.max_requests);
        key.enabled = draft.enabled;
        let backup = self.config.clone();
        self.config.local_keys.push(key);
        if !self.persist() {
            self.config = backup;
            return false;
        }
        self.toast(NoticeLevel::Success, "本地 Key 已创建，可在列表中复制");
        true
    }

    fn save_manual_balance(&mut self, draft: &ManualBalanceDraft) -> bool {
        let backup = self.config.clone();
        let Some(account) = self
            .config
            .accounts
            .iter_mut()
            .find(|a| a.id == draft.account_id)
        else {
            self.toast(NoticeLevel::Error, "账号不存在。");
            return false;
        };
        account.balance.mode = BalanceMode::Manual;
        account.balance.manual_amount = draft.amount.max(0.0);
        account.balance.manual_updated_at = now_ts();
        if !self.persist() {
            self.config = backup;
            return false;
        }
        self.toast(NoticeLevel::Success, "手动余额已更新");
        true
    }

    // -- editor window ------------------------------------------------------

    pub(crate) fn show_editor(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        #[derive(Clone, Copy)]
        enum EditorKind {
            Provider,
            Account,
            Route,
            Key,
            Balance,
        }
        let (kind, title, subtitle) = match &editor {
            Editor::Provider(d) => (
                EditorKind::Provider,
                if d.is_new {
                    "新增供应商"
                } else {
                    "编辑供应商"
                },
                "配置上游端点与协议解析，保存后可直接测试连接。",
            ),
            Editor::Account(d) => (
                EditorKind::Account,
                if d.is_new {
                    "新增账号"
                } else {
                    "编辑账号"
                },
                "API Key 加密保存；建议先用当前 Key 预检一次。",
            ),
            Editor::Route(d) => (
                EditorKind::Route,
                if d.is_new {
                    "新增路由"
                } else {
                    "编辑路由"
                },
                "定义客户端模型到上游模型的映射与降级优先级。",
            ),
            Editor::LocalKey(_) => (
                EditorKind::Key,
                "新增本地 Key",
                "本地 Key 用于客户端鉴权，可限制模型与额度。",
            ),
            Editor::ManualBalance(_) => (
                EditorKind::Balance,
                "录入手动余额",
                "从当前时间起按请求花费估算递减。",
            ),
        };
        let mut action = None;
        let closed = super::kit::modal_shell(
            ctx,
            Id::new("yoyo_editor"),
            title,
            subtitle,
            760.0,
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("editor_form")
                    .auto_shrink([false, false])
                    .show(ui, |ui| match &mut editor {
                        Editor::Provider(draft) => provider_form(self, ui, draft),
                        Editor::Account(draft) => account_form(self, ui, draft),
                        Editor::Route(draft) => route_form(self, ui, draft),
                        Editor::LocalKey(draft) => local_key_form(ui, draft),
                        Editor::ManualBalance(draft) => manual_balance_form(self, ui, draft),
                    });
            },
            |ui| {
                match kind {
                    EditorKind::Provider | EditorKind::Account | EditorKind::Route => {
                        if super::kit::primary_button(ui, icons::FLASK, "保存并测试").clicked()
                        {
                            action = Some(EditorAction::SaveTest);
                        }
                        if super::kit::secondary_button(ui, "", "仅保存").clicked() {
                            action = Some(EditorAction::Save);
                        }
                    }
                    EditorKind::Key => {
                        if super::kit::primary_button(ui, icons::CHECK, "创建").clicked() {
                            action = Some(EditorAction::Save);
                        }
                    }
                    EditorKind::Balance => {
                        if super::kit::primary_button(ui, icons::CHECK, "保存").clicked() {
                            action = Some(EditorAction::Save);
                        }
                    }
                }
                if super::kit::secondary_button(ui, "", "取消").clicked() {
                    action = Some(EditorAction::Cancel);
                }
            },
        );
        if closed && action.is_none() {
            action = Some(EditorAction::Cancel);
        }
        match action {
            None => self.editor = Some(editor),
            Some(EditorAction::Cancel) => self.editor = None,
            Some(kind) => {
                let test = kind == EditorAction::SaveTest;
                let saved = match &editor {
                    Editor::Provider(draft) => self.save_provider(draft, test),
                    Editor::Account(draft) => self.save_account(draft, test),
                    Editor::Route(draft) => self.save_route(draft, test),
                    Editor::LocalKey(draft) => self.create_local_key(draft),
                    Editor::ManualBalance(draft) => self.save_manual_balance(draft),
                };
                if !saved {
                    self.editor = Some(editor);
                }
            }
        }
    }

    // -- confirm window -----------------------------------------------------

    // -- confirm window -----------------------------------------------------

    pub(crate) fn show_confirm(&mut self, ctx: &egui::Context) {
        let Some(action) = self.confirm.take() else {
            return;
        };
        let (title, description, danger, confirm_label) = self.confirm_text(&action);
        let mut confirmed = false;
        let mut cancelled = false;
        let closed = super::kit::modal_shell(
            ctx,
            Id::new("yoyo_confirm"),
            title,
            "此操作会立即生效",
            520.0,
            |ui| {
                egui::Frame::new()
                    .fill(if danger {
                        super::theme::tokens(ui).red_soft
                    } else {
                        super::theme::tokens(ui).hover
                    })
                    .corner_radius(10)
                    .inner_margin(egui::Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(if danger { icons::WARNING } else { icons::INFO })
                                    .size(16.0)
                                    .color(if danger {
                                        super::theme::tokens(ui).red
                                    } else {
                                        super::theme::tokens(ui).primary
                                    }),
                            );
                            ui.add(egui::Label::new(RichText::new(&description).size(12.0)).wrap());
                        });
                    });
            },
            |ui| {
                let clicked = if danger {
                    super::kit::danger_button(ui, icons::TRASH_SIMPLE, confirm_label).clicked()
                } else {
                    super::kit::primary_button(ui, icons::CHECK, confirm_label).clicked()
                };
                if super::kit::secondary_button(ui, "", "取消").clicked() {
                    cancelled = true;
                }
                if clicked {
                    confirmed = true;
                }
            },
        );
        if (closed || cancelled) && !confirmed {
            return;
        }
        if confirmed {
            self.execute_confirm(action, ctx);
        } else {
            self.confirm = Some(action);
        }
    }

    fn confirm_text(&self, action: &ConfirmAction) -> (&'static str, String, bool, &'static str) {
        match action {
            ConfirmAction::DeleteProvider(id) => {
                let name = self
                    .config
                    .provider(id)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "该供应商".into());
                let accounts = self
                    .config
                    .accounts
                    .iter()
                    .filter(|a| a.provider_id == *id)
                    .count();
                let routes = self
                    .config
                    .routes
                    .iter()
                    .filter(|r| r.provider_id == *id)
                    .count();
                (
                    "删除供应商",
                    format!(
                        "将删除「{name}」，并级联删除 {accounts} 个账号、{routes} 条路由。此操作不可撤销。"
                    ),
                    true,
                    "确认删除",
                )
            }
            ConfirmAction::DeleteAccount(id) => {
                let name = self
                    .config
                    .account(id)
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| "该账号".into());
                (
                    "删除账号",
                    format!("将删除账号「{name}」。历史用量记录会保留。"),
                    true,
                    "确认删除",
                )
            }
            ConfirmAction::DeleteRoute(id) => {
                let route = self.config.route(id);
                let desc = route
                    .map(|r| format!("将删除路由 {} → {}。", r.client_model, r.upstream_model))
                    .unwrap_or_else(|| "将删除该路由。".into());
                ("删除路由", desc, true, "确认删除")
            }
            ConfirmAction::DeleteLocalKey(id) => {
                let name = self
                    .config
                    .local_key(id)
                    .map(|k| k.name.clone())
                    .unwrap_or_else(|| "该 Key".into());
                (
                    "删除本地 Key",
                    format!("将删除「{name}」。使用该 Key 的客户端会立即失去访问权限。"),
                    true,
                    "确认删除",
                )
            }
            ConfirmAction::ResetLocalKey(id) => {
                let name = self
                    .config
                    .local_key(id)
                    .map(|k| k.name.clone())
                    .unwrap_or_else(|| "该 Key".into());
                (
                    "重置本地 Key",
                    format!("将为「{name}」生成新的随机 Key，旧 Key 立即失效。模型白名单与额度限制保留。"),
                    true,
                    "确认重置",
                )
            }
            ConfirmAction::ImportConfig(path) => (
                "导入配置",
                format!(
                    "将从 {path} 导入配置并替换当前全部供应商、账号、路由与本地 Key。导入文件不包含密钥，所有 API Key 需要重新录入。"
                ),
                true,
                "确认导入",
            ),
            ConfirmAction::Quit => (
                "退出 YoyoRelay",
                "代理将停止，正在进行的请求会被中断。确定退出吗？".into(),
                true,
                "退出",
            ),
        }
    }

    fn execute_confirm(&mut self, action: ConfirmAction, ctx: &egui::Context) {
        match action {
            ConfirmAction::DeleteProvider(id) => {
                let backup = self.config.clone();
                self.config.providers.retain(|p| p.id != id);
                self.config.accounts.retain(|a| a.provider_id != id);
                self.config.routes.retain(|r| r.provider_id != id);
                if self.persist() {
                    self.toast(NoticeLevel::Success, "供应商及其级联数据已删除");
                } else {
                    self.config = backup;
                }
            }
            ConfirmAction::DeleteAccount(id) => {
                let backup = self.config.clone();
                self.config.accounts.retain(|a| a.id != id);
                if self.persist() {
                    self.toast(NoticeLevel::Success, "账号已删除");
                } else {
                    self.config = backup;
                }
            }
            ConfirmAction::DeleteRoute(id) => {
                let backup = self.config.clone();
                self.config.routes.retain(|r| r.id != id);
                if self.persist() {
                    self.toast(NoticeLevel::Success, "路由已删除");
                } else {
                    self.config = backup;
                }
            }
            ConfirmAction::DeleteLocalKey(id) => {
                let backup = self.config.clone();
                self.config.local_keys.retain(|k| k.id != id);
                if self.persist() {
                    self.toast(NoticeLevel::Success, "本地 Key 已删除");
                } else {
                    self.config = backup;
                }
            }
            ConfirmAction::ResetLocalKey(id) => {
                let name = self
                    .config
                    .local_key(&id)
                    .map(|k| k.name.clone())
                    .unwrap_or_else(|| "Key".into());
                let backup = self.config.clone();
                match self.store.new_local_key(&name) {
                    Ok(fresh) => {
                        if let Some(existing) =
                            self.config.local_keys.iter_mut().find(|k| k.id == id)
                        {
                            existing.key = fresh.key;
                        }
                        if self.persist() {
                            self.toast(NoticeLevel::Success, "本地 Key 已重置，请重新复制");
                        } else {
                            self.config = backup;
                        }
                    }
                    Err(err) => self.toast(NoticeLevel::Error, format!("生成 Key 失败：{err}")),
                }
            }
            ConfirmAction::ImportConfig(path) => {
                match self
                    .store
                    .import_without_secrets(std::path::Path::new(&path))
                {
                    Ok(imported) => {
                        self.config = imported;
                        if self.persist() {
                            self.toast(
                                NoticeLevel::Warning,
                                "配置已导入。请到「账号」页重新录入 API Key，并到「本地 Key」页重置本地凭据。",
                            );
                        }
                    }
                    Err(err) => self.toast(NoticeLevel::Error, format!("导入失败：{err}")),
                }
            }
            ConfirmAction::Quit => {
                self.quit(ctx);
            }
        }
    }

    // -- test modal ---------------------------------------------------------

    pub(crate) fn show_test_modal(&mut self, ctx: &egui::Context) {
        let Some(key) = self.test_key.clone() else {
            return;
        };
        let running = self.snapshot.running_tests.contains(&key);
        let result = self.snapshot.tests.get(&key).cloned();
        let mut close = false;
        let title = if let Some(result) = &result {
            format!("连接测试 · {}", result.title)
        } else {
            "连接测试".to_string()
        };
        let closed = super::kit::modal_shell(
            ctx,
            Id::new("yoyo_test"),
            &title,
            "最小请求探测，不写入正式用量",
            720.0,
            |ui| {
                if running {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("正在发起最小请求，测试结束后会自动显示结果…");
                    });
                } else if let Some(result) = &result {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&result.title).size(15.0).strong());
                        super::kit::badge(
                            ui,
                            result.level.label(),
                            widgets::level_color(result.level),
                        );
                        if let Some(status) = result.status {
                            ui.label(
                                RichText::new(format!("HTTP {status}"))
                                    .monospace()
                                    .size(10.5)
                                    .color(theme::muted(ui)),
                            );
                        }
                        ui.label(
                            RichText::new(format!("{} ms", result.latency_ms))
                                .monospace()
                                .size(10.5)
                                .color(theme::muted(ui)),
                        );
                    });
                    ui.add_space(6.0);
                    ui.label(RichText::new(&result.message).size(12.5));
                    ui.add_space(8.0);
                    egui::Frame::new()
                        .fill(theme::subtle_fill(ui))
                        .corner_radius(10)
                        .inner_margin(egui::Margin::symmetric(12, 10))
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&result.detail).monospace().size(11.0),
                                )
                                .wrap(),
                            );
                        });
                    if !result.models.is_empty() {
                        ui.add_space(8.0);
                        egui::CollapsingHeader::new(
                            RichText::new(format!("返回的模型（{}）", result.models.len()))
                                .size(11.5)
                                .strong(),
                        )
                        .id_salt("test_models")
                        .show(ui, |ui| {
                            for model in result.models.iter().take(100) {
                                ui.label(RichText::new(model).monospace().size(11.0));
                            }
                        });
                    }
                    ui.add_space(8.0);
                    super::kit::info_row(ui, icons::INFO, "测试请求不计入正式用量与额度统计。");
                } else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("等待测试结果…");
                    });
                }
            },
            |ui| {
                if super::kit::secondary_button(ui, icons::X, "关闭").clicked() {
                    close = true;
                }
            },
        );
        if closed || close {
            self.test_key = None;
        }
    }

    // -- onboarding ---------------------------------------------------------
    // -- onboarding ---------------------------------------------------------

    pub(crate) fn show_onboarding(&mut self, ctx: &egui::Context) {
        if !self.show_onboarding {
            return;
        }
        let mut finish = false;
        let mut skip = false;
        let closed = super::kit::modal_shell(
            ctx,
            Id::new("yoyo_onboarding"),
            "快速开始",
            "三步完成本地代理配置",
            560.0,
            |ui| {
                ui.label(
                    RichText::new(format!("第 {}/3 步", self.onboarding_step + 1))
                        .size(10.5)
                        .color(theme::muted(ui)),
                );
                ui.add_space(4.0);
                match self.onboarding_step {
                    0 => {
                        ui.label(RichText::new("添加一个供应商").size(17.0).strong());
                        ui.label(
                            RichText::new(
                                "支持官方直连、聚合平台或任意 OpenAI / Anthropic 兼容端点。",
                            )
                            .size(11.5)
                            .color(theme::muted(ui)),
                        );
                        ui.add_space(10.0);
                        ui.horizontal_wrapped(|ui| {
                            for (key, label) in [
                                ("openai", "OpenAI"),
                                ("anthropic", "Anthropic"),
                                ("command_code", "Command Code"),
                                ("opencode_zen", "OpenCode Zen"),
                            ] {
                                if super::kit::secondary_button(ui, "", label).clicked() {
                                    let id_placeholder = new_id();
                                    let mut provider = self.store.preset_provider(key);
                                    provider.id = id_placeholder;
                                    provider.created_at = now_ts();
                                    self.editor = Some(Editor::Provider(ProviderDraft {
                                        is_new: true,
                                        preset: key.to_string(),
                                        provider,
                                    }));
                                    self.show_onboarding = false;
                                }
                            }
                        });
                        ui.add_space(10.0);
                        if !self.config.providers.is_empty()
                            && super::kit::primary_button(ui, icons::ARROW_RIGHT, "下一步")
                                .clicked()
                        {
                            self.onboarding_step = 1;
                        }
                    }
                    1 => {
                        ui.label(RichText::new("添加账号并填入 API Key").size(17.0).strong());
                        ui.label(
                            RichText::new("保存后可直接测试连接。额度窗口与并发上限可稍后配置。")
                                .size(11.5)
                                .color(theme::muted(ui)),
                        );
                        ui.add_space(10.0);
                        if self.config.accounts.is_empty() {
                            if super::kit::primary_button(ui, icons::PLUS, "添加账号").clicked()
                            {
                                self.open_new_account();
                                self.show_onboarding = false;
                            }
                        } else if super::kit::primary_button(ui, icons::ARROW_RIGHT, "下一步")
                            .clicked()
                        {
                            self.onboarding_step = 2;
                        }
                    }
                    _ => {
                        ui.label(RichText::new("复制本地 Key").size(17.0).strong());
                        ui.label(
                            RichText::new(
                                "客户端使用这个 Key 访问本地端点；Key 只在本机加密保存。",
                            )
                            .size(11.5)
                            .color(theme::muted(ui)),
                        );
                        ui.add_space(10.0);
                        if let Some(key) = self.config.local_keys.first().cloned() {
                            let plain = self.store.decrypt(&key.key).unwrap_or_default();
                            super::kit::soft_panel(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(if plain.is_empty() {
                                            "（无法解密）".to_string()
                                        } else {
                                            crate::config::mask_secret(&plain)
                                        })
                                        .monospace()
                                        .size(11.5),
                                    );
                                    if super::kit::icon_button(ui, icons::COPY, "复制完整 Key")
                                        .clicked()
                                        && !plain.is_empty()
                                    {
                                        ui.ctx().copy_text(plain);
                                        self.toast(NoticeLevel::Success, "本地 Key 已复制");
                                    }
                                });
                            });
                        } else {
                            ui.label(
                                RichText::new("尚未创建本地 Key，可稍后在「本地 Key」页创建。")
                                    .size(11.5)
                                    .color(theme::muted(ui)),
                            );
                        }
                        ui.add_space(10.0);
                        if super::kit::primary_button(ui, icons::CHECK, "完成").clicked() {
                            finish = true;
                        }
                    }
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if self.onboarding_step > 0
                        && super::kit::secondary_button(ui, icons::ARROW_LEFT, "上一步").clicked()
                    {
                        self.onboarding_step -= 1;
                    }
                });
            },
            |ui| {
                if super::kit::ghost_button(ui, "", "跳过向导").clicked() {
                    skip = true;
                }
            },
        );
        if finish || skip || closed {
            self.show_onboarding = false;
            self.config.ui.onboarding_done = true;
            self.persist();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditorAction {
    Cancel,
    Save,
    SaveTest,
}

// ---------------------------------------------------------------------------
// Forms
// ---------------------------------------------------------------------------

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(title).size(12.5).strong());
    ui.add_space(2.0);
}

fn provider_form(app: &mut App, ui: &mut egui::Ui, draft: &mut ProviderDraft) {
    let provider = &mut draft.provider;
    if draft.is_new {
        ui.horizontal(|ui| {
            ui.label("从预设开始");
            let current_label = crate::config::presets::PRESETS
                .iter()
                .find(|p| p.key == draft.preset)
                .map(|p| p.name)
                .unwrap_or("自定义");
            ComboBox::from_id_salt("provider_preset")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    for preset in crate::config::presets::PRESETS {
                        if ui
                            .selectable_label(draft.preset == preset.key, preset.name)
                            .clicked()
                        {
                            let id = provider.id.clone();
                            let created = provider.created_at;
                            *provider = app.store.preset_provider(preset.key);
                            provider.id = id;
                            provider.created_at = created;
                            draft.preset = preset.key.to_string();
                        }
                    }
                });
        });
        if let Some(preset) = crate::config::presets::PRESETS
            .iter()
            .find(|p| p.key == draft.preset)
        {
            ui.label(
                RichText::new(preset.description)
                    .size(11.0)
                    .color(theme::muted(ui)),
            );
        }
        ui.add_space(6.0);
    }

    section(ui, "基本信息");
    egui::Grid::new("provider_basic")
        .num_columns(2)
        .spacing([14.0, 8.0])
        .show(ui, |ui| {
            ui.label("名称");
            ui.add(
                TextEdit::singleline(&mut provider.name)
                    .desired_width(380.0)
                    .hint_text("例如：OpenAI 官方"),
            );
            ui.end_row();
            ui.label("类型");
            ComboBox::from_id_salt("provider_kind")
                .selected_text(provider.kind.label())
                .show_ui(ui, |ui| {
                    for kind in [
                        ProviderKind::Official,
                        ProviderKind::Aggregator,
                        ProviderKind::Custom,
                    ] {
                        ui.selectable_value(&mut provider.kind, kind, kind.label());
                    }
                });
            ui.end_row();
            ui.label("Base URL");
            ui.add(
                TextEdit::singleline(&mut provider.base_url)
                    .desired_width(380.0)
                    .hint_text("https://api.openai.com/v1"),
            );
            ui.end_row();
            ui.label("测试模型");
            ui.add(
                TextEdit::singleline(
                    provider
                        .test_model
                        .get_or_insert_with(|| "gpt-4o-mini".into()),
                )
                .desired_width(380.0)
                .hint_text("用于供应商测试与兜底探测"),
            );
            ui.end_row();
            ui.label("超时（秒）");
            let mut timeout = provider.timeout_secs.unwrap_or(0);
            if ui
                .add(
                    egui::DragValue::new(&mut timeout)
                        .range(0..=600)
                        .suffix(" s"),
                )
                .changed()
            {
                provider.timeout_secs = (timeout > 0).then_some(timeout);
            }
            ui.end_row();
        });

    section(ui, "鉴权方式");
    egui::Grid::new("provider_auth")
        .num_columns(2)
        .spacing([14.0, 8.0])
        .show(ui, |ui| {
            ui.label("方式");
            ComboBox::from_id_salt("provider_auth_mode")
                .selected_text(provider.auth.mode.label())
                .show_ui(ui, |ui| {
                    for mode in [
                        AuthMode::Bearer,
                        AuthMode::XApiKey,
                        AuthMode::Custom,
                        AuthMode::None,
                    ] {
                        ui.selectable_value(&mut provider.auth.mode, mode, mode.label());
                    }
                });
            ui.end_row();
            if provider.auth.mode == AuthMode::Custom {
                ui.label("Header 名称");
                ui.add(
                    TextEdit::singleline(&mut provider.auth.header_name)
                        .desired_width(380.0)
                        .hint_text("x-api-key"),
                );
                ui.end_row();
                ui.label("Scheme 前缀");
                ui.add(
                    TextEdit::singleline(&mut provider.auth.scheme)
                        .desired_width(380.0)
                        .hint_text("例如 Bearer（可留空）"),
                );
                ui.end_row();
            }
        });

    section(ui, "协议解析");
    egui::Grid::new("provider_protocol")
        .num_columns(2)
        .spacing([14.0, 8.0])
        .show(ui, |ui| {
            ui.label("策略");
            ComboBox::from_id_salt("provider_strategy")
                .selected_text(provider.protocol_strategy.mode.label())
                .show_ui(ui, |ui| {
                    for mode in [
                        ProtocolStrategyMode::Static,
                        ProtocolStrategyMode::ModelPrefix,
                        ProtocolStrategyMode::Explicit,
                    ] {
                        ui.selectable_value(
                            &mut provider.protocol_strategy.mode,
                            mode,
                            mode.label(),
                        );
                    }
                });
            ui.end_row();
            ui.label("固定协议");
            ComboBox::from_id_salt("provider_static_protocol")
                .selected_text(provider.static_protocol.label())
                .show_ui(ui, |ui| {
                    for protocol in [
                        Protocol::OpenAiChat,
                        Protocol::AnthropicMessages,
                        Protocol::OpenAiResponses,
                    ] {
                        ui.selectable_value(
                            &mut provider.static_protocol,
                            protocol,
                            protocol.label(),
                        );
                    }
                });
            ui.end_row();
        });
    if provider.protocol_strategy.mode == ProtocolStrategyMode::ModelPrefix {
        ui.label(
            RichText::new("按上游模型前缀匹配，最长前缀优先。")
                .size(11.0)
                .color(theme::muted(ui)),
        );
        let mut remove = None;
        for (index, rule) in provider
            .protocol_strategy
            .prefix_rules
            .iter_mut()
            .enumerate()
        {
            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut rule.prefix)
                        .desired_width(170.0)
                        .hint_text("模型前缀"),
                );
                ComboBox::from_id_salt(("prefix_protocol", index))
                    .selected_text(rule.protocol.label())
                    .show_ui(ui, |ui| {
                        for protocol in [
                            Protocol::OpenAiChat,
                            Protocol::AnthropicMessages,
                            Protocol::OpenAiResponses,
                        ] {
                            ui.selectable_value(&mut rule.protocol, protocol, protocol.label());
                        }
                    });
                if ui.small_button("删除").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            provider.protocol_strategy.prefix_rules.remove(index);
        }
        if ui.button("+ 添加前缀规则").clicked() {
            provider.protocol_strategy.prefix_rules.push(PrefixRule {
                prefix: String::new(),
                protocol: Protocol::OpenAiChat,
            });
        }
    }

    section(ui, "额外 Header");
    let mut remove = None;
    for (index, header) in provider.extra_headers.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut header.name)
                    .desired_width(190.0)
                    .hint_text("Header 名称"),
            );
            ui.add(
                TextEdit::singleline(&mut header.value)
                    .desired_width(250.0)
                    .hint_text("值"),
            );
            if ui.small_button("删除").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        provider.extra_headers.remove(index);
    }
    if ui.button("+ 添加 Header").clicked() {
        provider.extra_headers.push(Default::default());
    }
    ui.label(
        RichText::new("Header 值不做加密，请勿在此填写密钥；密钥请放在账号中。")
            .size(10.5)
            .color(theme::muted(ui)),
    );
}

fn account_form(app: &mut App, ui: &mut egui::Ui, draft: &mut AccountDraft) {
    section(ui, "基本信息");
    egui::Grid::new("account_basic")
        .num_columns(2)
        .spacing([14.0, 8.0])
        .show(ui, |ui| {
            ui.label("供应商");
            let provider_name = app
                .config
                .provider(&draft.account.provider_id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "请选择".into());
            ComboBox::from_id_salt("account_provider")
                .selected_text(provider_name)
                .show_ui(ui, |ui| {
                    for provider in &app.config.providers {
                        ui.selectable_value(
                            &mut draft.account.provider_id,
                            provider.id.clone(),
                            &provider.name,
                        );
                    }
                });
            ui.end_row();
            ui.label("名称");
            ui.add(
                TextEdit::singleline(&mut draft.account.name)
                    .desired_width(380.0)
                    .hint_text("例如：主账号 5h"),
            );
            ui.end_row();
            ui.label("API Key");
            ui.add(
                TextEdit::singleline(&mut draft.api_key_input)
                    .password(true)
                    .desired_width(380.0)
                    .hint_text(if draft.is_new {
                        "粘贴账号 API Key"
                    } else {
                        "留空表示保持现有 Key"
                    }),
            );
            ui.end_row();
            ui.label("优先级");
            ui.add(egui::DragValue::new(&mut draft.account.priority).range(-100..=100));
            ui.label(
                RichText::new("数值越大越优先；同优先级轮询")
                    .size(10.5)
                    .color(theme::muted(ui)),
            );
            ui.end_row();
            ui.label("并发上限");
            ui.add(
                egui::DragValue::new(&mut draft.account.max_concurrency)
                    .range(0..=256)
                    .suffix(" 个"),
            );
            ui.label(
                RichText::new("0 表示不限；本地并发满时暂时跳过该账号")
                    .size(10.5)
                    .color(theme::muted(ui)),
            );
            ui.end_row();
            ui.label("冷却覆盖");
            let mut override_secs = draft.account.cooldown_override_secs.unwrap_or(0);
            if ui
                .add(
                    egui::DragValue::new(&mut override_secs)
                        .range(0..=3600)
                        .suffix(" s"),
                )
                .changed()
            {
                draft.account.cooldown_override_secs = (override_secs > 0).then_some(override_secs);
            }
            ui.label(
                RichText::new("0 使用全局冷却；失败后指数退避")
                    .size(10.5)
                    .color(theme::muted(ui)),
            );
            ui.end_row();
            ui.label("启用");
            ui.checkbox(&mut draft.account.enabled, "参与调度");
            ui.end_row();
        });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let running = app.snapshot.running_tests.contains("draft:account");
        if ui
            .add_enabled(!running, egui::Button::new("测试当前 Key"))
            .on_hover_text("用当前草稿中的 Key 发一个最小请求，不保存、不计入额度")
            .clicked()
        {
            let key = if draft.api_key_input.trim().is_empty() {
                app.store
                    .decrypt(&draft.account.api_key)
                    .unwrap_or_default()
            } else {
                draft.api_key_input.trim().to_string()
            };
            let model_hint = app
                .config
                .routes
                .iter()
                .find(|r| r.provider_id == draft.account.provider_id)
                .map(|r| r.upstream_model.clone())
                .or_else(|| {
                    app.config
                        .provider(&draft.account.provider_id)
                        .and_then(|p| p.test_model.clone())
                });
            if app
                .cmd_tx
                .send(crate::runtime::UiCommand::TestAccountDraft {
                    provider_id: draft.account.provider_id.clone(),
                    name: draft.account.name.clone(),
                    api_key: key,
                    model_hint,
                })
                .is_err()
            {
                app.toast(NoticeLevel::Error, "运行时已退出，无法测试。");
            }
        }
        if running {
            ui.spinner();
            ui.label(
                RichText::new("正在预检…")
                    .size(11.0)
                    .color(theme::muted(ui)),
            );
        } else if let Some(result) = app.snapshot.tests.get("draft:account") {
            super::kit::badge(ui, result.level.label(), widgets::level_color(result.level));
            ui.label(
                RichText::new(&result.message)
                    .size(11.0)
                    .color(widgets::level_color(result.level)),
            );
        }
        ui.label(
            RichText::new("建议保存前先测试；测试失败仍允许保存（上游可能只是暂时抖动）")
                .size(10.0)
                .color(theme::muted(ui)),
        );
    });
    if !app.snapshot.running_tests.contains("draft:account") {
        if let Some(result) = app.snapshot.tests.get("draft:account") {
            if !result.detail.is_empty() {
                ui.collapsing("查看预检详情", |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&result.detail).monospace().size(10.5))
                            .wrap(),
                    );
                });
            }
        }
    }

    section(ui, "本地额度窗口");
    ui.checkbox(&mut draft.quota_enabled, "启用额度上限（本地统计）");
    if draft.quota_enabled {
        let mut window = draft
            .account
            .quota
            .as_ref()
            .map(|q| q.window)
            .unwrap_or(QuotaWindow::Rolling);
        egui::Grid::new("account_quota")
            .num_columns(2)
            .spacing([14.0, 8.0])
            .show(ui, |ui| {
                ui.label("窗口");
                ComboBox::from_id_salt("quota_window")
                    .selected_text(window.label())
                    .show_ui(ui, |ui| {
                        for value in [
                            QuotaWindow::Rolling,
                            QuotaWindow::Daily,
                            QuotaWindow::Weekly,
                            QuotaWindow::Monthly,
                        ] {
                            ui.selectable_value(&mut window, value, value.label());
                        }
                    });
                ui.end_row();
                if window == QuotaWindow::Rolling {
                    ui.label("滚动小时");
                    ui.add(
                        egui::DragValue::new(&mut draft.rolling_hours)
                            .range(1..=720)
                            .suffix(" h"),
                    );
                    ui.end_row();
                }
                ui.label("Token 上限");
                ui.add(
                    egui::DragValue::new(&mut draft.limit_tokens)
                        .range(0..=u64::MAX)
                        .speed(1000.0)
                        .suffix(" tokens"),
                );
                ui.end_row();
                ui.label("请求数上限");
                ui.add(
                    egui::DragValue::new(&mut draft.limit_requests)
                        .range(0..=u64::MAX)
                        .suffix(" 次"),
                );
                ui.end_row();
            });
        ui.label(
            RichText::new("0 表示该项不限；用尽后账号进入 Exhausted，窗口重置后自动恢复。")
                .size(10.5)
                .color(theme::muted(ui)),
        );
        draft.account.quota = Some(QuotaConfig {
            window,
            rolling_hours: draft.rolling_hours.max(1),
            limit_tokens: (draft.limit_tokens > 0).then_some(draft.limit_tokens),
            limit_requests: (draft.limit_requests > 0).then_some(draft.limit_requests),
        });
    }

    section(ui, "余额");
    let balance = &mut draft.account.balance;
    egui::Grid::new("account_balance")
        .num_columns(2)
        .spacing([14.0, 8.0])
        .show(ui, |ui| {
            ui.label("模式");
            ComboBox::from_id_salt("balance_mode")
                .selected_text(balance.mode.label())
                .show_ui(ui, |ui| {
                    for mode in [
                        BalanceMode::Auto,
                        BalanceMode::Manual,
                        BalanceMode::Unavailable,
                    ] {
                        ui.selectable_value(&mut balance.mode, mode, mode.label());
                    }
                });
            ui.end_row();
            ui.label("币种");
            ui.add(
                TextEdit::singleline(&mut balance.currency)
                    .desired_width(90.0)
                    .hint_text("USD"),
            );
            ui.end_row();
            match balance.mode {
                BalanceMode::Auto => {
                    ui.label("查询 URL");
                    ui.add(
                        TextEdit::singleline(balance.url.get_or_insert_with(String::new))
                            .desired_width(380.0)
                            .hint_text("GET https://provider.example.com/api/balance"),
                    );
                    ui.end_row();
                    ui.label("JSON 路径");
                    ui.add(
                        TextEdit::singleline(&mut balance.json_path)
                            .desired_width(380.0)
                            .hint_text("data.balance 或 data[0].credits"),
                    );
                    ui.end_row();
                    ui.label("数值倍率");
                    ui.add(egui::DragValue::new(&mut balance.scale).speed(0.01));
                    ui.end_row();
                }
                BalanceMode::Manual => {
                    ui.label("余额");
                    ui.add(
                        egui::DragValue::new(&mut balance.manual_amount)
                            .speed(0.1)
                            .prefix("$ "),
                    );
                    ui.end_row();
                    ui.label("说明");
                    ui.label(
                        RichText::new("保存后从该时间点起的预估花费会递减此余额")
                            .size(10.5)
                            .color(theme::muted(ui)),
                    );
                    ui.end_row();
                }
                BalanceMode::Unavailable => {}
            }
        });

    if !draft.is_new {
        let masked = app.store.masked_secret(&draft.account.api_key);
        ui.label(
            RichText::new(format!("当前 Key：{masked}"))
                .monospace()
                .size(10.5)
                .color(theme::muted(ui)),
        );
    }
}

fn route_form(app: &mut App, ui: &mut egui::Ui, draft: &mut RouteDraft) {
    let route = &mut draft.route;
    section(ui, "模型映射");
    egui::Grid::new("route_basic")
        .num_columns(2)
        .spacing([14.0, 8.0])
        .show(ui, |ui| {
            ui.label("客户端模型名");
            ui.add(
                TextEdit::singleline(&mut route.client_model)
                    .desired_width(380.0)
                    .hint_text("客户端请求里看到的 model"),
            );
            ui.end_row();
            ui.label("供应商");
            let provider_name = app
                .config
                .provider(&route.provider_id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "请选择".into());
            ComboBox::from_id_salt("route_provider")
                .selected_text(provider_name)
                .show_ui(ui, |ui| {
                    for provider in &app.config.providers {
                        ui.selectable_value(
                            &mut route.provider_id,
                            provider.id.clone(),
                            &provider.name,
                        );
                    }
                });
            ui.end_row();
            ui.label("上游模型名");
            ui.add(
                TextEdit::singleline(&mut route.upstream_model)
                    .desired_width(380.0)
                    .hint_text("发给供应商的真实模型名"),
            );
            ui.end_row();
            ui.label("优先级");
            ui.add(egui::DragValue::new(&mut route.priority).range(-100..=100));
            ui.label(
                RichText::new("降序；同一供应商的多条路由视为同一层")
                    .size(10.5)
                    .color(theme::muted(ui)),
            );
            ui.end_row();
            ui.label("路由模式");
            ComboBox::from_id_salt("route_mode")
                .selected_text(route.mode.label())
                .show_ui(ui, |ui| {
                    for mode in [RouteMode::Adapt, RouteMode::Passthrough] {
                        ui.selectable_value(&mut route.mode, mode, mode.label());
                    }
                });
            ui.end_row();
            ui.label("协议覆盖");
            let current = route
                .protocol
                .map(|p| p.label())
                .unwrap_or("由供应商策略决定");
            ComboBox::from_id_salt("route_protocol")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(route.protocol.is_none(), "由供应商策略决定")
                        .clicked()
                    {
                        route.protocol = None;
                    }
                    for protocol in [
                        Protocol::OpenAiChat,
                        Protocol::AnthropicMessages,
                        Protocol::OpenAiResponses,
                    ] {
                        if ui
                            .selectable_label(route.protocol == Some(protocol), protocol.label())
                            .clicked()
                        {
                            route.protocol = Some(protocol);
                        }
                    }
                });
            ui.end_row();
            ui.label("启用");
            ui.checkbox(&mut route.enabled, "参与调度");
            ui.end_row();
        });

    if let Some(provider) = app.config.provider(&route.provider_id) {
        let resolved = provider.protocol_for(&route.upstream_model, route.protocol);
        ui.horizontal(|ui| {
            ui.label(RichText::new("实际协议").size(11.0).color(theme::muted(ui)));
            super::kit::badge(ui, resolved.label(), theme::BLUE_INFO);
            ui.label(
                RichText::new(format!(
                    "POST {}",
                    provider.join_url(match resolved {
                        Protocol::OpenAiChat => "chat/completions",
                        Protocol::AnthropicMessages => "messages",
                        Protocol::OpenAiResponses => "responses",
                    })
                ))
                .monospace()
                .size(10.5)
                .color(theme::muted(ui)),
            );
        });
    }

    section(ui, "能力与备选");
    egui::Grid::new("route_more")
        .num_columns(2)
        .spacing([14.0, 8.0])
        .show(ui, |ui| {
            ui.label("能力标签");
            ui.add(
                TextEdit::singleline(&mut draft.capabilities_text)
                    .desired_width(380.0)
                    .hint_text("chat,tools,vision（逗号分隔）"),
            );
            ui.end_row();
            ui.label("上下文上限");
            ui.horizontal(|ui| {
                ui.checkbox(&mut draft.max_context_enabled, "限制");
                ui.add_enabled(
                    draft.max_context_enabled,
                    egui::DragValue::new(&mut draft.max_context)
                        .range(0..=10_000_000)
                        .suffix(" tokens"),
                );
            });
            ui.end_row();
            ui.label("映射级兜底");
            ui.add(
                TextEdit::singleline(&mut draft.fallback_text)
                    .desired_width(380.0)
                    .hint_text("另一个客户端模型名（可选，默认关闭）"),
            );
            ui.end_row();
            ui.label("输入价格");
            ui.add(
                egui::DragValue::new(&mut route.input_price_per_mtok)
                    .speed(0.01)
                    .prefix("$ ")
                    .suffix(" / Mtok"),
            );
            ui.end_row();
            ui.label("输出价格");
            ui.add(
                egui::DragValue::new(&mut route.output_price_per_mtok)
                    .speed(0.01)
                    .prefix("$ ")
                    .suffix(" / Mtok"),
            );
            ui.end_row();
        });
    ui.label(
        RichText::new("价格用于概览页预估花费与手动余额递减；留 0 表示不估算。")
            .size(10.5)
            .color(theme::muted(ui)),
    );
}

fn local_key_form(ui: &mut egui::Ui, draft: &mut LocalKeyDraft) {
    section(ui, "基本信息");
    ui.label("名称");
    ui.add(
        TextEdit::singleline(&mut draft.name)
            .desired_width(f32::INFINITY)
            .hint_text("例如：Claude Code / 本地脚本"),
    );
    ui.add_space(6.0);
    ui.checkbox(&mut draft.enabled, "启用该 Key");
    ui.add_space(6.0);
    ui.label("模型白名单");
    ui.add(
        TextEdit::singleline(&mut draft.allowed_models_text)
            .desired_width(f32::INFINITY)
            .hint_text("留空 = 允许全部；多个模型用逗号分隔，或填 *"),
    );
    ui.label(
        RichText::new("白名单匹配客户端模型名。")
            .size(10.5)
            .color(theme::muted(ui)),
    );
    section(ui, "额度");
    egui::Grid::new("local_key_quota")
        .num_columns(2)
        .spacing([14.0, 8.0])
        .show(ui, |ui| {
            ui.label("Token 上限");
            ui.add(
                egui::DragValue::new(&mut draft.max_tokens)
                    .range(0..=u64::MAX)
                    .speed(1000.0)
                    .suffix(" tokens"),
            );
            ui.end_row();
            ui.label("请求数上限");
            ui.add(
                egui::DragValue::new(&mut draft.max_requests)
                    .range(0..=u64::MAX)
                    .suffix(" 次"),
            );
            ui.end_row();
        });
    ui.label(
        RichText::new("0 表示不限；额度按 Key 生命周期累计。")
            .size(10.5)
            .color(theme::muted(ui)),
    );
}

fn manual_balance_form(app: &App, ui: &mut egui::Ui, draft: &mut ManualBalanceDraft) {
    let label = app
        .config
        .account(&draft.account_id)
        .map(|a| {
            format!(
                "{} / {}",
                app.config
                    .provider(&a.provider_id)
                    .map(|p| p.name.as_str())
                    .unwrap_or("?"),
                a.name
            )
        })
        .unwrap_or_else(|| "未知账号".into());
    ui.label(RichText::new(label).strong());
    ui.add_space(8.0);
    ui.label("当前余额");
    ui.add(
        egui::DragValue::new(&mut draft.amount)
            .speed(0.1)
            .prefix("$ ")
            .range(0.0..=1_000_000_000.0),
    );
    ui.add_space(6.0);
    ui.label(
        RichText::new("保存后模式切换为「手动」，并从现在开始按路由价格估算递减。")
            .size(11.0)
            .color(theme::muted(ui)),
    );
}
