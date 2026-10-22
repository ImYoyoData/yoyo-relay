use egui::RichText;
use egui_phosphor::regular as icons;

use crate::config::ProviderKind;

use super::{theme, widgets, App, ConfirmAction, Page};

impl App {
    pub(crate) fn page_providers(&mut self, ui: &mut egui::Ui) {
        super::kit::page_header(
            ui,
            super::Page::Providers.eyebrow(),
            "供应商",
            "上游端点、协议策略、凭据测试与账号入口",
            |ui| {
                if super::kit::primary_labeled(ui, icons::PLUS, "新增供应商").clicked() {
                    self.open_new_provider();
                }
            },
        );

        let providers: Vec<_> = self
            .config
            .providers
            .iter()
            .filter(|p| {
                self.matches_search(&format!("{} {} {}", p.name, p.base_url, p.kind.label()))
            })
            .cloned()
            .collect();
        if providers.is_empty() {
            if super::kit::empty_state(
                ui,
                icons::HARD_DRIVES,
                "还没有供应商",
                "先添加一个官方直连、聚合平台或自定义端点，再配置账号。",
                icons::PLUS,
                "添加供应商",
            ) {
                self.open_new_provider();
            }
            return;
        }

        for kind in [
            ProviderKind::Official,
            ProviderKind::Aggregator,
            ProviderKind::Custom,
        ] {
            let group: Vec<_> = providers
                .iter()
                .filter(|p| p.kind == kind)
                .cloned()
                .collect();
            if group.is_empty() {
                continue;
            }
            ui.horizontal(|ui| {
                super::kit::section_label(ui, kind.label());
                ui.label(
                    RichText::new(format!("{}", group.len()))
                        .size(10.0)
                        .color(theme::muted(ui)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let line = ui.available_width();
                    let y = ui.cursor().center().y;
                    ui.painter().line_segment(
                        [
                            egui::pos2(ui.cursor().left(), y),
                            egui::pos2(ui.cursor().left() + line, y),
                        ],
                        egui::Stroke::new(1.0_f32, theme::border_color(ui)),
                    );
                });
            });
            for provider in group {
                self.provider_card(ui, &provider);
                ui.add_space(10.0);
            }
            ui.add_space(4.0);
        }
    }

    fn provider_card(&mut self, ui: &mut egui::Ui, provider: &crate::config::Provider) {
        let health = self.snapshot.provider(&provider.id).cloned();
        let account_count = self
            .config
            .accounts
            .iter()
            .filter(|a| a.provider_id == provider.id)
            .count();
        let test_key = format!("provider:{}", provider.id);
        let running = self.snapshot.running_tests.contains(&test_key);
        let mut action: Option<CardAction> = None;
        super::kit::surface(ui, |ui| {
            ui.horizontal(|ui| {
                super::kit::icon_tile(ui, icons::HARD_DRIVES, theme::ACCENT, 38.0);
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&provider.name).size(15.0).strong());
                        super::kit::badge(ui, provider.kind.label(), theme::BLUE_INFO);
                        if provider.protocol_strategy.mode
                            == crate::config::ProtocolStrategyMode::ModelPrefix
                        {
                            super::kit::badge(ui, "前缀解析", theme::ORANGE);
                        }
                    });
                    ui.label(
                        RichText::new(&provider.base_url)
                            .monospace()
                            .size(10.0)
                            .color(theme::muted(ui)),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if super::kit::danger_icon_button(ui, icons::TRASH_SIMPLE, "删除供应商")
                        .clicked()
                    {
                        action = Some(CardAction::Delete);
                    }
                    if super::kit::icon_button(ui, icons::PENCIL_SIMPLE, "编辑供应商").clicked()
                    {
                        action = Some(CardAction::Edit);
                    }
                    if super::kit::icon_button(ui, icons::USERS, "管理该供应商的账号").clicked()
                    {
                        action = Some(CardAction::Accounts);
                    }
                    if super::kit::labeled_button(
                        ui,
                        if running {
                            icons::SPINNER
                        } else {
                            icons::FLASK
                        },
                        "测试连接",
                        armas::ButtonVariant::Outline,
                        armas::ButtonSize::Small,
                    )
                    .clicked()
                        && !running
                    {
                        action = Some(CardAction::Test);
                    }
                });
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if account_count == 0 {
                    super::kit::status_badge(ui, "未配置账号", theme::MUTED);
                } else if health
                    .as_ref()
                    .map(|h| h.all_unavailable())
                    .unwrap_or(false)
                {
                    super::kit::status_badge(ui, "不可用", theme::RED);
                } else {
                    super::kit::status_badge(ui, "可用", theme::GREEN);
                }
                if let Some(health) = &health {
                    ui.label(
                        RichText::new(format!(
                            "{} 个账号 · 健康 {} · 冷却 {} · 额度用尽 {}",
                            health.account_count,
                            health.healthy,
                            health.cooling + health.half_open,
                            health.exhausted
                        ))
                        .size(10.0)
                        .color(theme::muted(ui)),
                    );
                } else {
                    ui.label(
                        RichText::new(format!("{account_count} 个账号"))
                            .size(10.0)
                            .color(theme::muted(ui)),
                    );
                }
                let auth = match provider.auth.mode {
                    crate::config::AuthMode::Bearer => "Bearer",
                    crate::config::AuthMode::XApiKey => "x-api-key",
                    crate::config::AuthMode::Custom => "自定义 Header",
                    crate::config::AuthMode::None => "无鉴权",
                };
                super::kit::badge(ui, provider.static_protocol.label(), theme::MUTED);
                super::kit::badge(ui, auth, theme::MUTED);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(health) = &health {
                        if health.balance_known {
                            ui.label(
                                RichText::new(widgets::format_money(
                                    health.balance,
                                    &health.currency,
                                ))
                                .size(13.0)
                                .strong(),
                            );
                            super::kit::badge(ui, "余额", theme::MUTED);
                        }
                    }
                    if !running {
                        widgets::test_badge(ui, &self.snapshot, &test_key);
                    }
                });
            });
        });
        if let Some(action) = action {
            match action {
                CardAction::Edit => self.open_edit_provider(&provider.id),
                CardAction::Delete => {
                    self.confirm = Some(ConfirmAction::DeleteProvider(provider.id.clone()))
                }
                CardAction::Test => self.start_test("provider", &provider.id),
                CardAction::Accounts => {
                    self.page = Page::Accounts;
                    self.search.clear();
                }
            }
        }
    }
}

enum CardAction {
    Edit,
    Delete,
    Test,
    Accounts,
}
