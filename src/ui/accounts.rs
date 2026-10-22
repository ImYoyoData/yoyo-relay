use armas::components::Progress;
use egui::RichText;
use egui_phosphor::regular as icons;

use crate::config::{now_ts, Account};
use crate::scheduler::AccountState;
use crate::snapshot::NoticeLevel;

use super::{brands, theme, widgets, App, ConfirmAction, Page};

impl App {
    pub(crate) fn page_accounts(&mut self, ui: &mut egui::Ui) {
        super::kit::page_header(
            ui,
            Page::Accounts.eyebrow(),
            "账号",
            "账号池、额度窗口、并发与冷却状态",
            |ui| {
                if super::kit::primary_button(ui, icons::PLUS, "新增账号").clicked() {
                    self.open_new_account();
                }
            },
        );

        if self.config.accounts.is_empty() {
            if super::kit::empty_state(
                ui,
                icons::USERS,
                "还没有账号",
                "账号是调度的基本单位。添加 API Key 后可设置额度窗口与并发上限。",
                icons::PLUS,
                "添加账号",
            ) {
                self.open_new_account();
            }
            return;
        }

        let total = self.config.accounts.len();
        let healthy = self
            .snapshot
            .accounts
            .iter()
            .filter(|a| a.state == AccountState::Healthy)
            .count();
        let cooling = self
            .snapshot
            .accounts
            .iter()
            .filter(|a| {
                matches!(
                    a.state,
                    AccountState::Cooling { .. } | AccountState::HalfOpen
                )
            })
            .count();
        let exhausted = self
            .snapshot
            .accounts
            .iter()
            .filter(|a| matches!(a.state, AccountState::Exhausted { .. }))
            .count();
        super::kit::soft_panel(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{total} 个账号")).size(11.5).strong());
                super::kit::status_pill(ui, &format!("健康 {healthy}"), theme::GREEN);
                super::kit::status_pill(ui, &format!("冷却 {cooling}"), theme::ORANGE);
                super::kit::status_pill(ui, &format!("额度用尽 {exhausted}"), theme::RED);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if super::kit::ghost_button(ui, icons::HARD_DRIVES, "管理供应商").clicked()
                    {
                        self.page = Page::Providers;
                    }
                });
            });
        });
        ui.add_space(12.0);

        let providers = self.config.providers.clone();
        let mut any = false;
        for provider in providers {
            let accounts: Vec<Account> = self
                .config
                .accounts
                .iter()
                .filter(|a| a.provider_id == provider.id)
                .filter(|a| self.matches_search(&format!("{} {}", provider.name, a.name)))
                .cloned()
                .collect();
            if accounts.is_empty() {
                continue;
            }
            any = true;
            let health = self.snapshot.provider(&provider.id).cloned();
            let healthy = health.as_ref().map(|h| h.healthy).unwrap_or(0);
            let balance_text = health
                .as_ref()
                .filter(|h| h.balance_known())
                .map(|h| widgets::format_currency_totals(&h.balances));
            let mut add_for_provider = None;
            let provider_id = provider.id.clone();
            let provider_brand = brands::for_provider(&provider);
            super::kit::surface(ui, |ui| {
                ui.horizontal(|ui| {
                    brands::tile(ui, &provider_brand, 34.0).on_hover_text(&provider.name);
                    ui.add_space(2.0);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(&provider.name)
                                    .size(14.0)
                                    .strong()
                                    .color(theme::text(ui)),
                            );
                            super::kit::badge(
                                ui,
                                &format!("{} 个账号", accounts.len()),
                                theme::MUTED,
                            );
                            if healthy > 0 {
                                super::kit::status_pill(
                                    ui,
                                    &format!("{healthy} 健康"),
                                    theme::GREEN,
                                );
                            }
                        });
                        ui.label(
                            RichText::new(&provider.base_url)
                                .monospace()
                                .size(9.5)
                                .color(theme::muted(ui)),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.set_height(34.0);
                        if let Some(balance) = &balance_text {
                            ui.label(RichText::new(balance).size(13.0).strong());
                            super::kit::plain_badge(ui, "余额");
                        }
                        if super::kit::icon_button(ui, icons::PLUS, "为该供应商添加账号").clicked()
                        {
                            add_for_provider = Some(provider_id.clone());
                        }
                    });
                });
                ui.add_space(8.0);
                super::kit::divider(ui);
                ui.add_space(2.0);
                for (index, account) in accounts.iter().enumerate() {
                    self.account_row(ui, account);
                    if index + 1 < accounts.len() {
                        ui.add_space(6.0);
                    }
                }
            });
            if let Some(id) = add_for_provider {
                self.open_new_account_for(&id);
            }
            ui.add_space(10.0);
        }
        if !any {
            ui.label(
                RichText::new("没有匹配的账号。")
                    .size(11.5)
                    .color(theme::muted(ui)),
            );
        }
    }

    fn account_row(&mut self, ui: &mut egui::Ui, account: &Account) {
        let view = self.snapshot.account(&account.id).cloned();
        let state = view
            .as_ref()
            .map(|v| v.state.clone())
            .unwrap_or(AccountState::Disabled);
        let color = widgets::state_color(&state);
        let state_icon = state_icon(&state);
        let test_key = format!("account:{}", account.id);
        let running = self.snapshot.running_tests.contains(&test_key);
        let mut action: Option<RowAction> = None;
        egui::Frame::new()
            .fill(theme::subtle_fill(ui))
            .corner_radius(10)
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::Vec2::splat(30.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, 8.0, theme::soft_for(ui, color));
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        state_icon,
                        egui::FontId::proportional(15.0),
                        color,
                    );
                    ui.add_space(2.0);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(&account.name)
                                    .size(13.0)
                                    .strong()
                                    .color(theme::text(ui)),
                            );
                            if !account.enabled {
                                super::kit::plain_badge(ui, "已禁用");
                            }
                            if account.priority != 0 {
                                super::kit::badge(
                                    ui,
                                    &format!("P{}", account.priority),
                                    theme::BLUE_INFO,
                                );
                            }
                            ui.label(
                                RichText::new(widgets::state_short(&state, now_ts()))
                                    .size(9.5)
                                    .color(color),
                            )
                            .on_hover_text(widgets::state_text(&state, now_ts()));
                        });
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(
                                    view.as_ref()
                                        .map(|v| v.masked_key.clone())
                                        .unwrap_or_else(|| "未设置".into()),
                                )
                                .monospace()
                                .size(9.5)
                                .color(theme::muted(ui)),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "今日 {} tokens · {} 次",
                                    widgets::format_count(
                                        view.as_ref().map(|v| v.today_tokens).unwrap_or(0)
                                    ),
                                    view.as_ref().map(|v| v.today_requests).unwrap_or(0)
                                ))
                                .size(9.0)
                                .color(theme::muted(ui)),
                            );
                        });
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.set_height(30.0);
                        if super::kit::danger_icon_button(ui, icons::TRASH_SIMPLE, "删除账号")
                            .clicked()
                        {
                            action = Some(RowAction::Delete(account.id.clone()));
                        }
                        if super::kit::icon_button(
                            ui,
                            if account.enabled {
                                icons::POWER
                            } else {
                                icons::PLAY
                            },
                            if account.enabled {
                                "禁用账号"
                            } else {
                                "启用账号"
                            },
                        )
                        .clicked()
                        {
                            action = Some(RowAction::Toggle(account.id.clone(), !account.enabled));
                        }
                        if super::kit::icon_button(ui, icons::PENCIL_SIMPLE, "编辑账号").clicked()
                        {
                            action = Some(RowAction::Edit(account.id.clone()));
                        }
                        if super::kit::icon_button(
                            ui,
                            if running {
                                icons::SPINNER
                            } else {
                                icons::FLASK
                            },
                            "测试账号",
                        )
                        .clicked()
                            && !running
                        {
                            action = Some(RowAction::Test(account.id.clone()));
                        }
                        if !running {
                            widgets::test_badge(ui, &self.snapshot, &test_key);
                        }
                    });
                });

                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if let Some(view) = &view {
                        if let Some(quota) = &view.quota {
                            widgets::quota_bar(
                                ui,
                                quota.used_tokens,
                                quota.limit_tokens,
                                quota.used_requests,
                                quota.limit_requests,
                                quota.reset_at,
                                now_ts(),
                            );
                            super::kit::plain_badge(ui, &quota.window_label);
                        } else {
                            super::kit::plain_badge(ui, "不限额度");
                        }
                        ui.label(
                            RichText::new(format!(
                                "并发 {}/{}",
                                view.inflight,
                                if view.max_concurrency == 0 {
                                    "∞".to_string()
                                } else {
                                    view.max_concurrency.to_string()
                                }
                            ))
                            .size(9.5)
                            .color(theme::muted(ui)),
                        );
                        if let Some(amount) = view.balance {
                            ui.label(
                                RichText::new(widgets::format_money(
                                    amount,
                                    &view.balance_currency,
                                ))
                                .size(11.5)
                                .strong(),
                            );
                            if view.balance_estimated {
                                super::kit::badge(ui, "估算", theme::ORANGE);
                            }
                        } else {
                            ui.label(RichText::new("余额 —").size(9.5).color(theme::muted(ui)));
                        }
                        if let Some(quota) = &view.quota {
                            if let Some(limit) = quota.limit_tokens {
                                if limit > 0 {
                                    let pct = (quota.used_tokens as f32 / limit as f32 * 100.0)
                                        .min(100.0);
                                    Progress::new(pct).width(120.0).height(5.0).show(ui);
                                }
                            }
                        }
                    }
                });
            });
        match action {
            Some(RowAction::Test(id)) => self.start_test("account", &id),
            Some(RowAction::Edit(id)) => self.open_edit_account(&id),
            Some(RowAction::Toggle(id, enabled)) => self.set_account_enabled(&id, enabled),
            Some(RowAction::Delete(id)) => self.confirm = Some(ConfirmAction::DeleteAccount(id)),
            None => {}
        }
    }

    fn set_account_enabled(&mut self, id: &str, enabled: bool) {
        let backup = self.config.clone();
        if let Some(account) = self.config.accounts.iter_mut().find(|a| a.id == id) {
            account.enabled = enabled;
            if self.persist() {
                self.toast(
                    NoticeLevel::Success,
                    if enabled {
                        "账号已启用"
                    } else {
                        "账号已禁用"
                    },
                );
            } else {
                self.config = backup;
            }
        }
    }
}

enum RowAction {
    Test(String),
    Edit(String),
    Toggle(String, bool),
    Delete(String),
}

fn state_icon(state: &AccountState) -> &'static str {
    match state {
        AccountState::Healthy => icons::CHECK_CIRCLE,
        AccountState::Cooling { .. } => icons::CLOCK_COUNTER_CLOCKWISE,
        AccountState::HalfOpen => icons::PULSE,
        AccountState::Exhausted { .. } => icons::HOURGLASS_MEDIUM,
        AccountState::Disabled => icons::PROHIBIT,
    }
}
