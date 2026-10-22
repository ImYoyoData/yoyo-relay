use egui::{ComboBox, Id, RichText, TextEdit};
use egui_phosphor::regular as icons;

use crate::config::{
    new_id, now_ts, Account, AuthMode, BalanceConfig, BalanceMode, ClientModelProfile, ModelRoute,
    PrefixRule, PriceCurrency, Protocol, ProtocolStrategyMode, ProviderKind, QuotaConfig,
    QuotaWindow, RouteMode,
};
use crate::snapshot::NoticeLevel;

use super::{brands, kit, theme, widgets};
use super::{
    AccountDraft, App, BalanceSetupDraft, ConfirmAction, Editor, LocalKeyDraft, ManualBalanceDraft,
    Page, ProviderDraft, RouteDraft, RouteModelDraft,
};

impl App {
    // -- openers ------------------------------------------------------------

    pub(crate) fn open_new_provider(&mut self) {
        self.open_new_provider_with_preset("openai");
    }

    pub(crate) fn open_new_provider_with_preset(&mut self, key: &str) {
        let provider = self.store.preset_provider(key);
        self.editor = Some(Editor::Provider(ProviderDraft {
            is_new: true,
            preset: key.to_string(),
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

    pub(crate) fn open_new_route_model(&mut self) {
        let profile = ClientModelProfile {
            created_at: now_ts(),
            ..ClientModelProfile::default()
        };
        self.editor = Some(Editor::RouteModel(RouteModelDraft {
            is_new: true,
            original_model: String::new(),
            reasoning_text: String::new(),
            profile,
        }));
    }

    pub(crate) fn open_edit_route_model(&mut self, client_model: &str) {
        let profile = self
            .config
            .model_profile(client_model)
            .cloned()
            .unwrap_or_else(|| {
                let mut profile = ClientModelProfile {
                    client_model: client_model.to_string(),
                    created_at: now_ts(),
                    ..ClientModelProfile::default()
                };
                if let Some(route) = self
                    .config
                    .routes
                    .iter()
                    .find(|r| r.client_model == client_model)
                {
                    profile.context_window = route.max_context_tokens;
                    profile.fallback_client_model = route.fallback_client_model.clone();
                    if route.capabilities.iter().any(|c| c == "vision") {
                        profile.input_modalities.push("image".into());
                    }
                    if route
                        .capabilities
                        .iter()
                        .any(|c| c == "reasoning" || c == "thinking")
                    {
                        profile.reasoning_levels =
                            vec!["low".into(), "medium".into(), "high".into()];
                    }
                }
                profile
            });
        let reasoning_text = profile.reasoning_levels.join(",");
        self.editor = Some(Editor::RouteModel(RouteModelDraft {
            is_new: false,
            original_model: client_model.to_string(),
            profile,
            reasoning_text,
        }));
    }

    /// Add an upstream model (provider + model id) inside an existing route model.
    pub(crate) fn open_new_route(&mut self, client_model: &str) {
        let Some(provider) = self.config.providers.first().cloned() else {
            self.toast(NoticeLevel::Warning, "请先添加供应商，再添加上游模型。");
            self.page = Page::Providers;
            return;
        };
        let route = ModelRoute {
            id: new_id(),
            client_model: client_model.to_string(),
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

    /// Open the shared balance configuration editor from the Balances page.
    pub(crate) fn open_balance_setup(&mut self, account_id: &str) {
        let Some(account) = self.config.account(account_id).cloned() else {
            self.toast(NoticeLevel::Error, "账号不存在。");
            return;
        };
        let provider_name = self
            .config
            .provider(&account.provider_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "—".into());
        self.editor = Some(Editor::BalanceSetup(BalanceSetupDraft {
            account_id: account.id.clone(),
            account_name: account.name.clone(),
            provider_name,
            percent_used: 0.0,
            balance: account.balance.clone(),
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
        if account.balance.mode == BalanceMode::Auto {
            let formula = account.balance.formula.trim();
            if !formula.is_empty() {
                if let Err(err) = crate::balance::validate_formula(formula) {
                    self.toast(NoticeLevel::Error, format!("余额公式无效：{err}"));
                    return false;
                }
            }
            if !account.balance.scale.is_finite() || account.balance.scale <= 0.0 {
                account.balance.scale = 1.0;
            }
            account.balance.formula = formula.to_string();
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

    fn save_route_model(&mut self, draft: &RouteModelDraft) -> bool {
        let mut profile = draft.profile.clone();
        profile.client_model = profile.client_model.trim().to_string();
        if profile.client_model.is_empty() {
            self.toast(NoticeLevel::Error, "路由模型名不能为空。");
            return false;
        }
        if profile.display_name.trim().is_empty() {
            profile.display_name = profile.client_model.clone();
        }
        if profile.owned_by.trim().is_empty() {
            profile.owned_by = "yoyo-relay".into();
        }
        profile.reasoning_levels = draft
            .reasoning_text
            .split(',')
            .map(|level| level.trim().to_ascii_lowercase())
            .filter(|level| !level.is_empty())
            .collect();
        profile.reasoning_levels.dedup();
        profile.input_modalities = profile
            .input_modalities
            .iter()
            .map(|m| m.trim().to_ascii_lowercase())
            .filter(|m| !m.is_empty())
            .collect();
        if profile.input_modalities.is_empty() {
            profile.input_modalities.push("text".into());
        }
        profile.input_modalities.dedup();
        profile.output_modalities = profile
            .output_modalities
            .iter()
            .map(|m| m.trim().to_ascii_lowercase())
            .filter(|m| !m.is_empty())
            .collect();
        if profile.output_modalities.is_empty() {
            profile.output_modalities.push("text".into());
        }
        profile.output_modalities.dedup();
        profile.fallback_client_model = profile
            .fallback_client_model
            .take()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty() && m != &profile.client_model);
        profile.context_window = profile.context_window.filter(|v| *v > 0);
        profile.max_output_tokens = profile.max_output_tokens.filter(|v| *v > 0);
        if let (Some(context), Some(max_output)) =
            (profile.context_window, profile.max_output_tokens)
        {
            if max_output > context {
                self.toast(NoticeLevel::Error, "最大输出不能大于上下文窗口。");
                return false;
            }
        }
        if draft.is_new
            && self
                .config
                .model_profiles
                .iter()
                .any(|p| p.client_model == profile.client_model)
        {
            self.toast(NoticeLevel::Error, "该路由模型已存在。");
            return false;
        }
        if profile.created_at == 0 {
            profile.created_at = now_ts();
        }

        let backup = self.config.clone();
        if !draft.is_new && !draft.original_model.is_empty() {
            let old = draft.original_model.clone();
            if old != profile.client_model
                && self
                    .config
                    .model_profiles
                    .iter()
                    .any(|p| p.client_model == profile.client_model && p.client_model != old)
            {
                self.toast(NoticeLevel::Error, "该路由模型名已被占用。");
                return false;
            }
            if let Some(existing) = self
                .config
                .model_profiles
                .iter_mut()
                .find(|p| p.client_model == old)
            {
                *existing = profile.clone();
            } else {
                self.config.model_profiles.push(profile.clone());
            }
            if old != profile.client_model {
                for route in &mut self.config.routes {
                    if route.client_model == old {
                        route.client_model = profile.client_model.clone();
                    }
                    if route.fallback_client_model.as_deref() == Some(old.as_str()) {
                        route.fallback_client_model = Some(profile.client_model.clone());
                    }
                }
                for other in &mut self.config.model_profiles {
                    if other.fallback_client_model.as_deref() == Some(old.as_str()) {
                        other.fallback_client_model = Some(profile.client_model.clone());
                    }
                }
                for key in &mut self.config.local_keys {
                    for allowed in &mut key.allowed_models {
                        if allowed == &old {
                            *allowed = profile.client_model.clone();
                        }
                    }
                }
            }
        } else {
            self.config.model_profiles.push(profile.clone());
        }
        if !self.persist() {
            self.config = backup;
            return false;
        }
        let has_upstream = self
            .config
            .routes
            .iter()
            .any(|r| r.client_model == profile.client_model);
        if has_upstream {
            self.toast(NoticeLevel::Success, "路由模型已保存");
        } else {
            self.toast(
                NoticeLevel::Info,
                "路由模型已创建，请继续为它添加上游模型。",
            );
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

    fn save_balance_setup(&mut self, draft: &BalanceSetupDraft) -> bool {
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
        let mut balance = draft.balance.clone();
        balance.currency = crate::config::normalize_currency(&balance.currency);
        balance.scale = if balance.scale == 0.0 {
            1.0
        } else {
            balance.scale
        };
        balance.quota_total = balance.quota_total.max(0.0);
        account.balance = balance;
        if !self.persist() {
            self.config = backup;
            return false;
        }
        self.toast(NoticeLevel::Success, "余额设置已保存");
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
            RouteModel,
            Key,
            Balance,
            BalanceSetup,
        }
        let balance_setup_subtitle = if let Editor::BalanceSetup(d) = &editor {
            format!("{} · {}", d.account_name, d.provider_name)
        } else {
            String::new()
        };
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
                    "添加上游模型"
                } else {
                    "编辑上游模型"
                },
                "选择供应商与真实模型名，并设置它在降级链中的优先级。",
            ),
            Editor::RouteModel(d) => (
                EditorKind::RouteModel,
                if d.is_new {
                    "新增路由模型"
                } else {
                    "编辑路由模型"
                },
                "先定义客户端看到的模型与能力，再在卡片里添加上游供应商模型。",
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
            Editor::BalanceSetup(_) => (
                EditorKind::BalanceSetup,
                "余额设置",
                balance_setup_subtitle.as_str(),
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
                        Editor::RouteModel(draft) => route_model_form(self, ui, draft),
                        Editor::LocalKey(draft) => local_key_form(ui, draft),
                        Editor::ManualBalance(draft) => manual_balance_form(self, ui, draft),
                        Editor::BalanceSetup(draft) => balance_setup_form(self, ui, draft),
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
                    EditorKind::RouteModel => {
                        if super::kit::primary_button(ui, icons::CHECK, "保存模型").clicked() {
                            action = Some(EditorAction::Save);
                        }
                    }
                    EditorKind::Balance => {
                        if super::kit::primary_button(ui, icons::CHECK, "保存").clicked() {
                            action = Some(EditorAction::Save);
                        }
                    }
                    EditorKind::BalanceSetup => {
                        if super::kit::primary_button(ui, icons::CHECK, "保存设置").clicked() {
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
                    Editor::RouteModel(draft) => self.save_route_model(draft),
                    Editor::LocalKey(draft) => self.create_local_key(draft),
                    Editor::ManualBalance(draft) => self.save_manual_balance(draft),
                    Editor::BalanceSetup(draft) => self.save_balance_setup(draft),
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
            ConfirmAction::DeleteRouteModel(model) => {
                let upstreams = self
                    .config
                    .routes
                    .iter()
                    .filter(|r| r.client_model == *model)
                    .count();
                (
                    "删除路由模型",
                    format!(
                        "将删除路由模型「{model}」及其 {upstreams} 个上游模型。历史用量记录会保留。"
                    ),
                    true,
                    "确认删除",
                )
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
            ConfirmAction::ClearCalls => (
                "清空调用明细",
                "将删除全部调用记录，用量统计、趋势与余额估算将随之归零。供应商、账号与路由配置不受影响。此操作不可撤销。".to_string(),
                true,
                "确认清空",
            ),
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
            ConfirmAction::DeleteRouteModel(model) => {
                let backup = self.config.clone();
                self.config.routes.retain(|r| r.client_model != model);
                self.config
                    .model_profiles
                    .retain(|p| p.client_model != model);
                for profile in &mut self.config.model_profiles {
                    if profile.fallback_client_model.as_deref() == Some(model.as_str()) {
                        profile.fallback_client_model = None;
                    }
                }
                if self.persist() {
                    self.toast(
                        NoticeLevel::Success,
                        format!("路由模型「{model}」及其上游已删除"),
                    );
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
            ConfirmAction::ClearCalls => {
                let _ = self.cmd_tx.send(crate::runtime::UiCommand::ClearUsage);
                self.toast(NoticeLevel::Info, "正在清空调用明细…");
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
                    if let Some(tps) = result.tokens_per_sec {
                        ui.add_space(6.0);
                        egui::Frame::new()
                            .fill(theme::soft_for(ui, theme::ACCENT))
                            .corner_radius(10)
                            .inner_margin(egui::Margin::symmetric(12, 9))
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        RichText::new(icons::GAUGE).size(18.0).color(theme::ACCENT),
                                    );
                                    ui.label(
                                        RichText::new(format!("{tps:.1}"))
                                            .size(22.0)
                                            .strong()
                                            .color(theme::ACCENT),
                                    );
                                    ui.label(
                                        RichText::new("tokens / 秒")
                                            .size(10.5)
                                            .color(theme::muted(ui)),
                                    );
                                    if let Some(ttft) = result.first_token_ms {
                                        ui.separator();
                                        ui.label(
                                            RichText::new(format!("首字 {ttft} ms"))
                                                .size(10.5)
                                                .color(theme::muted(ui)),
                                        );
                                    }
                                    if let Some(tokens) = result.tokens {
                                        ui.label(
                                            RichText::new(format!(
                                                "生成 {} tokens",
                                                widgets::format_count(tokens)
                                            ))
                                            .size(10.5)
                                            .color(theme::muted(ui)),
                                        );
                                    }
                                });
                            });
                    }
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

    /// Name and store the price table staged by the route editor.
    pub(crate) fn show_price_plan_modal(&mut self, ctx: &egui::Context) {
        let Some(mut plan) = self.saving_price_plan.clone() else {
            return;
        };
        let mut save = false;
        let mut cancel = false;
        let title = "存为价格方案";
        let closed = super::kit::modal_shell(
            ctx,
            Id::new("yoyo_price_plan"),
            title,
            "保存后可在任意上游的计费价格中一键套用，也可在设置里导出",
            520.0,
            |ui| {
                ui.label(RichText::new("方案名称").size(11.0).color(theme::muted(ui)));
                ui.add(
                    TextEdit::singleline(&mut plan.name)
                        .desired_width(f32::INFINITY)
                        .hint_text("例如：GLM-4 峰谷价"),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new("备注（可选）")
                        .size(11.0)
                        .color(theme::muted(ui)),
                );
                ui.add(
                    TextEdit::singleline(&mut plan.note)
                        .desired_width(f32::INFINITY)
                        .hint_text("价格来源、生效日期等"),
                );
                ui.add_space(10.0);
                let symbol = plan.currency.symbol();
                egui::Frame::new()
                    .fill(theme::subtle_fill(ui))
                    .corner_radius(9)
                    .inner_margin(egui::Margin::symmetric(11, 9))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} · 输入 {symbol}{:.4} · 命中 {symbol}{:.4} · 输出 {symbol}{:.4}",
                                plan.currency.label(),
                                plan.base.input_per_mtok,
                                plan.base.cached_input_per_mtok,
                                plan.base.output_per_mtok
                            ))
                            .size(10.5)
                            .color(theme::text(ui)),
                        );
                        if !plan.periods.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "{} 个峰谷时段：{}",
                                    plan.periods.len(),
                                    plan.periods
                                        .iter()
                                        .map(|p| format!("{} {}", p.name, p.schedule_label()))
                                        .collect::<Vec<_>>()
                                        .join(" · ")
                                ))
                                .size(10.0)
                                .color(theme::muted(ui)),
                            );
                        }
                    });
            },
            |ui| {
                if super::kit::primary_button(ui, icons::BOOKMARK_SIMPLE, "保存方案").clicked()
                {
                    save = true;
                }
                if super::kit::secondary_button(ui, "", "取消").clicked() {
                    cancel = true;
                }
            },
        );
        if closed || cancel {
            self.saving_price_plan = None;
            return;
        }
        if save {
            if plan.name.trim().is_empty() {
                self.toast(NoticeLevel::Warning, "请填写方案名称。");
                self.saving_price_plan = Some(plan);
                return;
            }
            let normalized = plan.normalized();
            let replaced = self
                .config
                .price_plans
                .iter_mut()
                .find(|existing| existing.name.eq_ignore_ascii_case(&normalized.name))
                .map(|existing| *existing = normalized.clone())
                .is_some();
            if !replaced {
                self.config.price_plans.push(normalized);
            }
            if self.persist() {
                self.toast(
                    NoticeLevel::Success,
                    if replaced {
                        "价格方案已更新"
                    } else {
                        "价格方案已保存"
                    },
                );
            }
            self.saving_price_plan = None;
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
        section(ui, "品牌预设");
        if let Some(key) = brands::preset_gallery(ui, &draft.preset) {
            let id = provider.id.clone();
            let created = provider.created_at;
            *provider = app.store.preset_provider(key);
            provider.id = id;
            provider.created_at = created;
            draft.preset = key.to_string();
        }
        if let Some(preset) = crate::config::presets::preset(&draft.preset) {
            ui.label(
                RichText::new(preset.description)
                    .size(10.5)
                    .color(theme::muted(ui)),
            );
        }
        ui.add_space(6.0);
    }

    section(ui, "基本信息");
    let brand = brands::for_preset(Some(&draft.preset));
    ui.horizontal(|ui| {
        brands::tile(ui, &brand, 36.0);
        ui.add_space(2.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(&provider.name).size(13.0).strong());
            ui.label(
                RichText::new(&provider.base_url)
                    .monospace()
                    .size(9.5)
                    .color(theme::muted(ui)),
            );
        });
    });
    ui.add_space(4.0);
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
                        ProtocolStrategyMode::Ingress,
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
                    ui.label("换算公式");
                    ui.add(
                        TextEdit::singleline(&mut balance.formula)
                            .desired_width(380.0)
                            .hint_text("可留空；例如 data.balance / 100"),
                    );
                    ui.end_row();
                    ui.label("数值倍率");
                    ui.add(
                        egui::DragValue::new(&mut balance.scale)
                            .speed(0.01)
                            .range(0.0001..=1_000_000.0),
                    );
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

    if balance.mode == BalanceMode::Auto {
        let path = if balance.json_path.trim().is_empty() {
            "balance".to_string()
        } else {
            balance.json_path.trim().to_string()
        };
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("快捷换算").size(10.0).color(theme::muted(ui)));
            if ui.small_button("原始值").clicked() {
                balance.formula.clear();
                balance.scale = 1.0;
            }
            if ui.small_button("除以 100").clicked() {
                balance.formula = format!("{path} / 100");
                balance.scale = 1.0;
            }
            if ui.small_button("除以 1000").clicked() {
                balance.formula = format!("{path} / 1000");
                balance.scale = 1.0;
            }
            if ui.small_button("除以 10000").clicked() {
                balance.formula = format!("{path} / 10000");
                balance.scale = 1.0;
            }
            if ui.small_button("乘以 100").clicked() {
                balance.formula = format!("{path} * 100");
                balance.scale = 1.0;
            }
        });
        ui.label(
            RichText::new(
                "公式中的变量直接写 JSON 路径，支持 + - * / 与括号，例如 data.total - data.used；留空时使用 JSON 路径 × 倍率。",
            )
            .size(10.0)
            .color(theme::muted(ui)),
        );
    }

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
    section(ui, "所属路由模型");
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("客户端模型")
                .size(11.0)
                .color(theme::muted(ui)),
        );
        super::kit::badge(ui, &route.client_model, theme::ACCENT);
        ui.label(
            RichText::new("在路由模型卡片右上角可修改名称与模型能力")
                .size(10.0)
                .color(theme::muted(ui)),
        );
    });
    ui.add_space(4.0);

    section(ui, "上游模型");
    egui::Grid::new("route_basic")
        .num_columns(2)
        .spacing([14.0, 8.0])
        .show(ui, |ui| {
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
            let fetching = app.snapshot.models_fetching.contains(&route.provider_id);
            let catalog = app
                .snapshot
                .provider_models
                .get(&route.provider_id)
                .cloned();
            if !route.provider_id.is_empty()
                && catalog.is_none()
                && !fetching
                && app.models_requested.insert(route.provider_id.clone())
            {
                let _ = app
                    .cmd_tx
                    .send(crate::runtime::UiCommand::FetchProviderModels(
                        route.provider_id.clone(),
                    ));
            }
            ui.label("上游模型名");
            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut route.upstream_model)
                        .desired_width(280.0)
                        .hint_text("发给该供应商的真实模型名"),
                );
                if ui
                    .add_enabled(
                        !fetching,
                        egui::Button::new(if fetching {
                            format!("{} 获取中…", icons::SPINNER)
                        } else {
                            format!("{} 拉取列表", icons::LIST_MAGNIFYING_GLASS)
                        }),
                    )
                    .on_hover_text("从该供应商的 /models 端点读取模型名，仍可手动输入")
                    .clicked()
                {
                    let _ = app
                        .cmd_tx
                        .send(crate::runtime::UiCommand::FetchProviderModels(
                            route.provider_id.clone(),
                        ));
                }
                if let Some(catalog) = &catalog {
                    if !catalog.models.is_empty() {
                        ComboBox::from_id_salt("route_upstream_model_pick")
                            .selected_text(format!("{} 个可选", catalog.models.len()))
                            .width(170.0)
                            .show_ui(ui, |ui| {
                                for model in &catalog.models {
                                    if ui
                                        .selectable_label(route.upstream_model == *model, model)
                                        .clicked()
                                    {
                                        route.upstream_model = model.clone();
                                    }
                                }
                            });
                    }
                }
            });
            ui.end_row();
            ui.label("优先级");
            ui.add(egui::DragValue::new(&mut route.priority).range(-100..=100));
            ui.label(
                RichText::new("降序；同一供应商的多条上游视为同一层")
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

    if let Some(catalog) = app
        .snapshot
        .provider_models
        .get(&route.provider_id)
        .filter(|catalog| catalog.provider_id == route.provider_id)
    {
        ui.horizontal_wrapped(|ui| {
            super::kit::badge(
                ui,
                if catalog.ok {
                    "供应商模型列表"
                } else {
                    "获取失败"
                },
                if catalog.ok {
                    theme::GREEN
                } else {
                    theme::ORANGE
                },
            );
            ui.label(
                RichText::new(format!("{} · {}", catalog.provider_name, catalog.message))
                    .size(10.0)
                    .color(theme::muted(ui)),
            );
            ui.label(
                RichText::new(widgets::time_ago(catalog.at, crate::config::now_ts()))
                    .size(9.5)
                    .color(theme::muted(ui)),
            );
        });
    }

    if let Some(provider) = app.config.provider(&route.provider_id) {
        let resolved = provider.protocol_for(&route.upstream_model, route.protocol);
        ui.horizontal_wrapped(|ui| {
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
        ui.label(
            RichText::new("客户端使用三种协议中的任意一种，YoyoRelay 会自动转换成上游协议。")
                .size(10.0)
                .color(theme::muted(ui)),
        );
    }

    section(ui, "能力标签");
    ui.add(
        TextEdit::singleline(&mut draft.capabilities_text)
            .desired_width(360.0)
            .hint_text("chat,tools,vision（逗号分隔）"),
    );
    ui.label(
        RichText::new(
            "能力标签会出现在 /v1/models 与路由卡片上；上下文窗口与思考等级在「路由模型」中设置。",
        )
        .size(10.5)
        .color(theme::muted(ui)),
    );

    ui.add_space(6.0);
    pricing_section(app, ui, route);
}

/// Price editor for one upstream. Handles currency, the base tier and
/// peak/off-peak windows. Kept separate from the route form because it is the
/// most detail-heavy part of the configuration.
fn pricing_section(app: &mut App, ui: &mut egui::Ui, route: &mut ModelRoute) {
    let mut pricing = route.effective_pricing();
    let mut changed = false;

    section(ui, "计费价格");
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("计价币种").size(11.0).color(theme::muted(ui)));
        for currency in [PriceCurrency::Usd, PriceCurrency::Cny] {
            let active = pricing.currency == currency;
            let label = if active {
                RichText::new(currency.label())
                    .size(10.5)
                    .color(egui::Color32::WHITE)
            } else {
                RichText::new(currency.label()).size(10.5)
            };
            let button = egui::Button::new(label)
                .fill(if active {
                    theme::ACCENT
                } else {
                    theme::tokens(ui).card
                })
                .stroke(egui::Stroke::new(
                    1.0_f32,
                    if active {
                        theme::ACCENT
                    } else {
                        theme::tokens(ui).card_border
                    },
                ))
                .corner_radius(7);
            if ui.add(button).clicked() && !active {
                pricing.currency = currency;
                changed = true;
            }
        }
        ui.label(
            RichText::new("总花费始终折算为 USD 统计，币种只影响录入与展示口径")
                .size(10.0)
                .color(theme::muted(ui)),
        );
    });

    ui.add_space(6.0);
    let symbol = pricing.currency.symbol();
    egui::Grid::new("route_price_base")
        .num_columns(3)
        .spacing([12.0, 7.0])
        .show(ui, |ui| {
            ui.label(RichText::new("计费项").size(10.5).color(theme::muted(ui)));
            ui.label(
                RichText::new(format!("基础价（{symbol}/百万 tokens）"))
                    .size(10.5)
                    .color(theme::muted(ui)),
            );
            ui.label(RichText::new("说明").size(10.5).color(theme::muted(ui)));
            ui.end_row();

            ui.label("输入（未命中缓存）");
            if price_input(ui, &mut pricing.base.input_per_mtok, symbol) {
                changed = true;
            }
            ui.label(
                RichText::new("常规输入单价")
                    .size(10.0)
                    .color(theme::muted(ui)),
            );
            ui.end_row();

            ui.label("输入（命中缓存）");
            if price_input(ui, &mut pricing.base.cached_input_per_mtok, symbol) {
                changed = true;
            }
            ui.label(
                RichText::new("留空或 0 表示按常规输入价计费")
                    .size(10.0)
                    .color(theme::muted(ui)),
            );
            ui.end_row();

            ui.label("输出");
            if price_input(ui, &mut pricing.base.output_per_mtok, symbol) {
                changed = true;
            }
            ui.label(
                RichText::new("生成 token 单价")
                    .size(10.0)
                    .color(theme::muted(ui)),
            );
            ui.end_row();
        });

    ui.add_space(8.0);
    kit::section_label(ui, "峰谷时段（可选）");
    ui.label(
        RichText::new(
            "为不同时间段设置独立价格。未落入任何时段时使用上面的基础价；跨午夜的时段把结束小时设得比开始小时小即可。",
        )
        .size(10.0)
        .color(theme::muted(ui)),
    );
    ui.add_space(4.0);

    let mut remove: Option<usize> = None;
    for (index, period) in pricing.periods.iter_mut().enumerate() {
        egui::Frame::new()
            .fill(theme::subtle_fill(ui))
            .corner_radius(9)
            .inner_margin(egui::Margin::symmetric(11, 9))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        TextEdit::singleline(&mut period.name)
                            .desired_width(120.0)
                            .hint_text("时段名称"),
                    );
                    ui.label(RichText::new("时间").size(10.0).color(theme::muted(ui)));
                    if hour_input(ui, &mut period.start_hour) {
                        changed = true;
                    }
                    ui.label(RichText::new("→").size(10.0).color(theme::muted(ui)));
                    if hour_input(ui, &mut period.end_hour) {
                        changed = true;
                    }
                    ui.label(
                        RichText::new(period.schedule_label())
                            .size(9.5)
                            .color(theme::muted(ui)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if kit::danger_icon_button(ui, icons::TRASH_SIMPLE, "删除该时段").clicked()
                        {
                            remove = Some(index);
                        }
                    });
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("适用").size(10.0).color(theme::muted(ui)));
                    for (day, name) in ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
                        .iter()
                        .enumerate()
                    {
                        let day = day as u8;
                        let mut active = period.days.contains(&day);
                        let text = RichText::new(*name).size(10.0);
                        let button = egui::Button::new(if active {
                            text.color(egui::Color32::WHITE)
                        } else {
                            text.color(theme::text(ui))
                        })
                        .fill(if active {
                            theme::ACCENT
                        } else {
                            theme::tokens(ui).card
                        })
                        .stroke(egui::Stroke::new(
                            1.0_f32,
                            if active {
                                theme::ACCENT
                            } else {
                                theme::border_color(ui)
                            },
                        ))
                        .corner_radius(6)
                        .min_size(egui::Vec2::new(38.0, 21.0));
                        if ui.add(button).clicked() {
                            active = !active;
                            if active {
                                period.days.push(day);
                                period.days.sort_unstable();
                                period.days.dedup();
                            } else {
                                period.days.retain(|d| *d != day);
                            }
                            changed = true;
                        }
                    }
                    ui.label(
                        RichText::new(if period.days.is_empty() {
                            "（未选择 = 每天）"
                        } else {
                            ""
                        })
                        .size(9.5)
                        .color(theme::muted(ui)),
                    );
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("输入").size(10.0).color(theme::muted(ui)));
                    if price_input(ui, &mut period.tier.input_per_mtok, symbol) {
                        changed = true;
                    }
                    ui.label(RichText::new("命中").size(10.0).color(theme::muted(ui)));
                    if price_input(ui, &mut period.tier.cached_input_per_mtok, symbol) {
                        changed = true;
                    }
                    ui.label(RichText::new("输出").size(10.0).color(theme::muted(ui)));
                    if price_input(ui, &mut period.tier.output_per_mtok, symbol) {
                        changed = true;
                    }
                });
            });
        ui.add_space(4.0);
    }
    if let Some(index) = remove {
        pricing.periods.remove(index);
        changed = true;
    }

    ui.horizontal_wrapped(|ui| {
        if kit::secondary_labeled(ui, icons::PLUS, "添加时段").clicked() {
            let mut period = crate::config::PricePeriod::default();
            // Start new windows from the base tier so the user only edits the
            // numbers that differ between peak and off-peak.
            period.tier = pricing.base;
            pricing.periods.push(period);
            changed = true;
        }
        if !app.config.price_plans.is_empty() {
            let mut apply: Option<crate::config::PricePlan> = None;
            ComboBox::from_id_salt("price_plan_pick")
                .selected_text(format!("{} 套用方案", icons::BOOKMARK_SIMPLE))
                .width(160.0)
                .show_ui(ui, |ui| {
                    for plan in &app.config.price_plans {
                        if ui.selectable_label(false, &plan.name).clicked() {
                            apply = Some(plan.clone());
                        }
                    }
                });
            if let Some(plan) = apply {
                pricing = crate::config::RoutePricing {
                    currency: plan.currency,
                    base: plan.base,
                    periods: plan.periods.clone(),
                };
                changed = true;
            }
        }
        if kit::secondary_labeled(ui, icons::BOOKMARK_SIMPLE, "存为方案").clicked() {
            app.saving_price_plan = Some(crate::config::PricePlan {
                name: format!("{} · {}", route.client_model, pricing.currency.code()),
                note: String::new(),
                currency: pricing.currency,
                base: pricing.base,
                periods: pricing.periods.clone(),
            });
        }
    });

    if changed {
        let normalized = pricing.normalized();
        // Keep the legacy USD fields in sync so older builds and exports that
        // only read them still show a plausible price.
        if normalized.currency == PriceCurrency::Usd {
            route.input_price_per_mtok = normalized.base.input_per_mtok;
            route.output_price_per_mtok = normalized.base.output_per_mtok;
        } else {
            route.input_price_per_mtok = 0.0;
            route.output_price_per_mtok = 0.0;
        }
        route.pricing = normalized;
    }

    ui.add_space(4.0);
    let period_summary = if pricing.periods.is_empty() {
        "全天使用基础价".to_string()
    } else {
        format!(
            "{} 个时段：{}",
            pricing.periods.len(),
            pricing
                .periods
                .iter()
                .map(|p| p.schedule_label())
                .collect::<Vec<_>>()
                .join(" · ")
        )
    };
    kit::info_row(ui, icons::CLOCK, &period_summary);
}

/// Price input with a currency prefix and a Mtok suffix.
fn price_input(ui: &mut egui::Ui, value: &mut f64, symbol: &str) -> bool {
    ui.add(
        egui::DragValue::new(value)
            .speed(0.01)
            .range(0.0..=1_000_000.0)
            .fixed_decimals(4)
            .prefix(format!("{symbol} "))
            .suffix(" / Mtok"),
    )
    .changed()
}

fn hour_input(ui: &mut egui::Ui, hour: &mut u8) -> bool {
    ui.add(
        egui::DragValue::new(hour)
            .range(0..=24)
            .speed(1.0)
            .suffix(":00"),
    )
    .changed()
}

fn modality_toggle(ui: &mut egui::Ui, values: &mut Vec<String>, value: &str, label: &str) -> bool {
    let mut enabled = values.iter().any(|m| m == value);
    if ui.checkbox(&mut enabled, label).changed() {
        if enabled {
            if !values.iter().any(|m| m == value) {
                values.push(value.to_string());
            }
        } else {
            values.retain(|m| m != value);
        }
        return true;
    }
    false
}

fn apply_fetched_model(draft: &mut RouteModelDraft, fetched: &crate::snapshot::FetchedModelView) {
    if let Some(context) = fetched.context_window {
        draft.profile.context_window = Some(context);
    }
    if let Some(output) = fetched.max_output_tokens {
        draft.profile.max_output_tokens = Some(output);
    }
    if let (Some(context), Some(output)) = (
        draft.profile.context_window,
        draft.profile.max_output_tokens,
    ) {
        if output > context {
            draft.profile.max_output_tokens = Some(context);
        }
    }
    if !fetched.input_modalities.is_empty() {
        draft.profile.input_modalities = fetched.input_modalities.clone();
    }
    if !fetched.output_modalities.is_empty() {
        draft.profile.output_modalities = fetched.output_modalities.clone();
    }
    if !fetched.reasoning_levels.is_empty() {
        draft.reasoning_text = fetched.reasoning_levels.join(",");
    }
    if let Some(tools) = fetched.supports_tools {
        draft.profile.supports_tools = tools;
    }
    if let Some(streaming) = fetched.supports_streaming {
        draft.profile.supports_streaming = streaming;
    }
}

fn route_model_form(app: &mut App, ui: &mut egui::Ui, draft: &mut RouteModelDraft) {
    let lookup = if draft.is_new {
        draft.profile.client_model.trim().to_string()
    } else {
        draft.original_model.clone()
    };
    if let Some(fetched) = app.snapshot.fetched_model.clone() {
        if app.last_applied_fetch != Some(fetched.id)
            && !lookup.is_empty()
            && fetched.client_model == lookup
        {
            if fetched.ok {
                apply_fetched_model(draft, &fetched);
                app.toast(
                    NoticeLevel::Success,
                    format!(
                        "已从 {} 读取「{}」的模型信息，可检查后保存",
                        fetched.provider_name, fetched.upstream_model
                    ),
                );
            } else {
                app.toast(NoticeLevel::Warning, fetched.message.clone());
            }
            app.last_applied_fetch = Some(fetched.id);
        }
    }

    let upstreams: Vec<(String, String, String)> = app
        .config
        .routes
        .iter()
        .filter(|route| route.client_model == lookup)
        .filter_map(|route| {
            app.config.provider(&route.provider_id).map(|provider| {
                (
                    route.provider_id.clone(),
                    provider.name.clone(),
                    route.upstream_model.clone(),
                )
            })
        })
        .collect();
    let mut request: Option<(String, String)> = None;

    section(ui, "模型标识与上下文");
    ui.columns(2, |cols| {
        egui::Grid::new("route_model_basic")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(&mut cols[0], |ui| {
                ui.label("客户端模型名");
                ui.add(
                    TextEdit::singleline(&mut draft.profile.client_model)
                        .desired_width(f32::INFINITY)
                        .hint_text("例如 gpt-5-codex"),
                );
                ui.end_row();
                ui.label("显示名称");
                ui.add(
                    TextEdit::singleline(&mut draft.profile.display_name)
                        .desired_width(f32::INFINITY)
                        .hint_text("模型列表展示名称"),
                );
                ui.end_row();
                ui.label("归属");
                ui.add(
                    TextEdit::singleline(&mut draft.profile.owned_by)
                        .desired_width(f32::INFINITY)
                        .hint_text("yoyo-relay"),
                );
                ui.end_row();
                ui.label("描述");
                ui.add(
                    TextEdit::singleline(&mut draft.profile.description)
                        .desired_width(f32::INFINITY)
                        .hint_text("一句话说明（可选）"),
                );
                ui.end_row();
                ui.label("启用");
                ui.checkbox(&mut draft.profile.enabled, "在模型列表与路由页展示");
                ui.end_row();
            });

        const DEFAULT_CONTEXT: u64 = 128_000;
        const DEFAULT_MAX_OUTPUT: u64 = 4_096;
        let mut context_enabled = draft.profile.context_window.is_some();
        let mut context = draft.profile.context_window.unwrap_or(DEFAULT_CONTEXT);
        let mut output_enabled = draft.profile.max_output_tokens.is_some();
        let mut max_output = draft
            .profile
            .max_output_tokens
            .unwrap_or(DEFAULT_MAX_OUTPUT);
        egui::Grid::new("route_model_limits")
            .num_columns(2)
            .spacing([10.0, 8.0])
            .show(&mut cols[1], |ui| {
                ui.label("上下文窗口");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut context_enabled, "设置");
                    ui.add_enabled(
                        context_enabled,
                        egui::DragValue::new(&mut context)
                            .range(1..=10_000_000)
                            .speed(1000.0)
                            .suffix(" tokens"),
                    );
                });
                ui.end_row();
                ui.label("最大输出");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut output_enabled, "设置");
                    ui.add_enabled(
                        output_enabled,
                        egui::DragValue::new(&mut max_output)
                            .range(1..=10_000_000)
                            .speed(100.0)
                            .suffix(" tokens"),
                    );
                });
                ui.end_row();
            });
        // Keep the value while the switch is on; filtering zero made the
        // checkbox appear to snap back off immediately.
        draft.profile.context_window = context_enabled.then_some(context.max(1));
        draft.profile.max_output_tokens = output_enabled.then_some(max_output.max(1));
        if let (Some(context), Some(output)) = (
            draft.profile.context_window,
            draft.profile.max_output_tokens,
        ) {
            if output > context {
                draft.profile.max_output_tokens = Some(context);
            }
        }

        route_model_fetch_ui(&mut cols[1], app, &upstreams, &mut request);
    });

    section(ui, "能力（模型列表会返回）");
    ui.label(RichText::new("输入模态").size(10.5).color(theme::muted(ui)));
    ui.horizontal_wrapped(|ui| {
        modality_toggle(ui, &mut draft.profile.input_modalities, "text", "文本");
        modality_toggle(ui, &mut draft.profile.input_modalities, "image", "图片");
        modality_toggle(ui, &mut draft.profile.input_modalities, "audio", "音频");
        modality_toggle(ui, &mut draft.profile.input_modalities, "file", "文件");
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.checkbox(&mut draft.profile.supports_tools, "支持工具调用");
        ui.checkbox(&mut draft.profile.supports_streaming, "支持流式输出");
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label("思考等级");
        ui.add(
            TextEdit::singleline(&mut draft.reasoning_text)
                .desired_width(f32::INFINITY)
                .hint_text("off,low,medium,high；留空表示不支持"),
        );
    });
    ui.horizontal(|ui| {
        ui.label("映射级兜底");
        ui.add(
            TextEdit::singleline(
                draft
                    .profile
                    .fallback_client_model
                    .get_or_insert_with(String::new),
            )
            .desired_width(f32::INFINITY)
            .hint_text("所有上游都失败后改用另一个客户端模型（可选）"),
        );
    });
    ui.label(
        RichText::new("没有上游的路由模型不会出现在 /v1/models。")
            .size(10.0)
            .color(theme::muted(ui)),
    );

    if let Some((provider_id, upstream_model)) = request {
        let _ = app.cmd_tx.send(crate::runtime::UiCommand::FetchModelInfo {
            provider_id,
            upstream_model,
            client_model: lookup,
        });
    }
}

fn route_model_fetch_ui(
    ui: &mut egui::Ui,
    app: &App,
    upstreams: &[(String, String, String)],
    request: &mut Option<(String, String)>,
) {
    ui.add_space(10.0);
    ui.label(
        RichText::new("从上游读取")
            .size(11.5)
            .strong()
            .color(theme::text(ui)),
    );
    ui.label(
        RichText::new("通过上游 /models 读取上下文、输出、模态与思考等级并自动填入。")
            .size(9.5)
            .color(theme::muted(ui)),
    );
    if upstreams.is_empty() {
        ui.label(
            RichText::new("先保存模型并添加上游模型，再回来获取。")
                .size(9.5)
                .color(theme::muted(ui)),
        );
        return;
    }
    let running = app.snapshot.model_fetch_running;
    for (provider_id, provider_name, upstream_model) in upstreams {
        egui::Frame::new()
            .fill(theme::subtle_fill(ui))
            .corner_radius(7)
            .inner_margin(egui::Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{provider_name} → {upstream_model}"))
                            .size(9.8)
                            .color(theme::text(ui)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if super::kit::secondary_button(
                            ui,
                            if running {
                                icons::SPINNER
                            } else {
                                icons::DOWNLOAD_SIMPLE
                            },
                            if running { "获取中" } else { "获取" },
                        )
                        .clicked()
                            && !running
                        {
                            *request = Some((provider_id.clone(), upstream_model.clone()));
                        }
                    });
                });
            });
        ui.add_space(3.0);
    }
    if let Some(fetched) = &app.snapshot.fetched_model {
        if upstreams
            .iter()
            .any(|(_, _, model)| model == &fetched.upstream_model)
        {
            let color = if fetched.ok {
                theme::GREEN
            } else {
                theme::ORANGE
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(if fetched.ok {
                        icons::CHECK_CIRCLE
                    } else {
                        icons::WARNING
                    })
                    .size(11.0)
                    .color(color),
                );
                ui.label(
                    RichText::new(&fetched.message)
                        .size(9.5)
                        .color(theme::text(ui)),
                );
            });
            if fetched.ok {
                let mut facts = Vec::new();
                if let Some(context) = fetched.context_window {
                    facts.push(format!("上下文 {}", widgets::format_count(context)));
                }
                if let Some(output) = fetched.max_output_tokens {
                    facts.push(format!("最大输出 {}", widgets::format_count(output)));
                }
                if !fetched.input_modalities.is_empty() {
                    facts.push(format!("输入 {}", fetched.input_modalities.join("/")));
                }
                if !facts.is_empty() {
                    ui.label(
                        RichText::new(facts.join(" · "))
                            .size(9.0)
                            .color(theme::muted(ui)),
                    );
                }
                if fetched.models_seen > 0 {
                    ui.label(
                        RichText::new(format!("上游列表共 {} 个模型", fetched.models_seen))
                            .size(9.0)
                            .color(theme::muted(ui)),
                    );
                }
            }
        }
    }
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
    let currency = app
        .config
        .account(&draft.account_id)
        .map(|a| crate::config::normalize_currency(&a.balance.currency))
        .unwrap_or_else(|| "USD".into());
    let prefix = if currency == "CNY" {
        "¥ "
    } else if currency == "USD" {
        "$ "
    } else {
        ""
    };
    ui.label(format!("当前余额（{currency}）"));
    ui.add(
        egui::DragValue::new(&mut draft.amount)
            .speed(0.1)
            .prefix(prefix)
            .range(0.0..=1_000_000_000.0),
    );
    ui.add_space(6.0);
    ui.label(
        RichText::new(
            "保存后模式切换为「手动」，并从现在开始按路由价格（USD）与设置中的汇率折算递减。",
        )
        .size(11.0)
        .color(theme::muted(ui)),
    );
}

/// Full balance editor shared with the account editor. Both write the same
/// `Account.balance`, so changes are visible in either place immediately.
fn balance_setup_form(app: &App, ui: &mut egui::Ui, draft: &mut BalanceSetupDraft) {
    let _ = app;
    let balance = &mut draft.balance;
    balance.currency = crate::config::normalize_currency(&balance.currency);

    section(ui, "查询方式");
    egui::Grid::new("balance_setup_mode")
        .num_columns(2)
        .spacing([14.0, 8.0])
        .show(ui, |ui| {
            ui.label("模式");
            ComboBox::from_id_salt("balance_setup_kind")
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
                    .hint_text("USD / CNY"),
            );
            ui.label(
                RichText::new("余额、总额度与阈值都使用该币种")
                    .size(10.0)
                    .color(theme::muted(ui)),
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
                    ui.label("换算公式");
                    ui.add(
                        TextEdit::singleline(&mut balance.formula)
                            .desired_width(380.0)
                            .hint_text("可留空；例如 data.balance / 100"),
                    );
                    ui.end_row();
                    ui.label("数值倍率");
                    ui.add(
                        egui::DragValue::new(&mut balance.scale)
                            .speed(0.01)
                            .range(0.0001..=1_000_000.0),
                    );
                    ui.end_row();
                }
                BalanceMode::Manual => {
                    ui.label("手动余额");
                    let symbol = match balance.currency.as_str() {
                        "CNY" => "¥ ",
                        "USD" => "$ ",
                        _ => "",
                    };
                    ui.add(
                        egui::DragValue::new(&mut balance.manual_amount)
                            .speed(0.1)
                            .prefix(symbol)
                            .range(0.0..=1_000_000_000.0),
                    );
                    ui.end_row();
                }
                BalanceMode::Unavailable => {}
            }
        });

    if balance.mode == BalanceMode::Auto {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if super::kit::toggle_switch(ui, &mut balance.percent_used).changed() {
                if balance.percent_used && balance.quota_total <= 0.0 {
                    balance.quota_total = 100.0;
                }
            }
            ui.label(
                RichText::new("接口返回「已用百分比」而不是金额")
                    .size(11.0)
                    .strong()
                    .color(theme::text(ui)),
            );
        });
        if balance.percent_used {
            ui.label(
                RichText::new("自动刷新时会按 (100 − 已用%) ÷ 100 × 总额度 换算为剩余金额。")
                    .size(10.0)
                    .color(theme::muted(ui)),
            );
        }
        ui.label(
            RichText::new("公式与路径修改后会立即用于下一次「刷新余额」。")
                .size(10.0)
                .color(theme::muted(ui)),
        );
    }

    section(ui, "百分比快捷换算");
    ui.label(
        RichText::new("接口只给百分比时，填总额度和已用百分比，这里直接算出剩余金额。")
            .size(10.5)
            .color(theme::muted(ui)),
    );
    ui.add_space(4.0);
    let symbol = match balance.currency.as_str() {
        "CNY" => "¥ ",
        "USD" => "$ ",
        _ => "",
    };
    let mut percent = draft.percent_used.clamp(0.0, 100.0);
    egui::Grid::new("balance_percent_calc")
        .num_columns(2)
        .spacing([14.0, 8.0])
        .show(ui, |ui| {
            ui.label("总额度");
            if ui
                .add(
                    egui::DragValue::new(&mut balance.quota_total)
                        .speed(1.0)
                        .prefix(symbol)
                        .range(0.0..=1_000_000_000.0),
                )
                .changed()
            {
                balance.quota_total = balance.quota_total.max(0.0);
            }
            ui.end_row();
            ui.label("已用百分比");
            ui.add(
                egui::DragValue::new(&mut percent)
                    .speed(0.5)
                    .range(0.0..=100.0)
                    .suffix(" %"),
            );
            ui.end_row();
            ui.label("剩余金额");
            let remaining = (100.0 - percent) / 100.0 * balance.quota_total.max(0.0);
            ui.label(
                RichText::new(format!("{symbol}{remaining:.2}"))
                    .size(14.0)
                    .strong()
                    .color(theme::GREEN),
            );
            ui.end_row();
        });
    let remaining = (100.0 - percent) / 100.0 * balance.quota_total.max(0.0);
    draft.percent_used = percent;
    ui.horizontal_wrapped(|ui| {
        if super::kit::secondary_button(ui, icons::CALCULATOR, "填入手动余额").clicked() {
            balance.mode = BalanceMode::Manual;
            balance.manual_amount = remaining;
            balance.manual_updated_at = now_ts();
        }
        ui.label(
            RichText::new("会切换到「手动」模式，之后按请求花费递减。")
                .size(10.0)
                .color(theme::muted(ui)),
        );
    });
    ui.add_space(4.0);
    super::kit::info_row(
        ui,
        icons::INFO,
        "余额设置的改动与「账号 → 编辑 → 余额」完全互通，保存后立即生效。",
    );
}
