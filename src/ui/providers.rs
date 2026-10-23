use egui::RichText;
use egui_phosphor::regular as icons;

use crate::config::ProviderKind;
use crate::runtime::UiCommand;

use super::{brands, theme, widgets, App, ConfirmAction, Page};

impl App {
    pub(crate) fn page_providers(&mut self, ui: &mut egui::Ui) {
        super::kit::page_header(
            ui,
            super::Page::Providers.eyebrow(),
            "供应商",
            "上游端点、品牌预设、协议策略、凭据测试与账号入口",
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
        let temporary: Vec<_> = self
            .snapshot
            .temporary_providers()
            .into_iter()
            .filter(|p| {
                self.matches_search(&format!(
                    "{} {} {} {}",
                    p.name,
                    p.plugin_name,
                    p.protocol,
                    p.models.join(" ")
                ))
            })
            .collect();
        if self.config.providers.is_empty() && temporary.is_empty() && self.search.trim().is_empty()
        {
            super::kit::section_card(
                ui,
                icons::HARD_DRIVES,
                "从品牌预设开始",
                "内置 12 家常用上游，选择后会自动填好端点、鉴权与协议",
                |ui| {
                    super::kit::badge(ui, "推荐", theme::ACCENT);
                },
                |ui| {
                    if let Some(key) = brands::preset_gallery(ui, "") {
                        self.open_new_provider_with_preset(key);
                    }
                },
            );
            ui.add_space(12.0);
            return;
        }
        if providers.is_empty() && temporary.is_empty() {
            if super::kit::empty_state(
                ui,
                icons::MAGNIFYING_GLASS,
                "没有匹配的供应商",
                "换一个关键词，或清空右上角搜索框。",
                icons::PLUS,
                "新增供应商",
            ) {
                self.open_new_provider();
            }
            if super::kit::ghost_button(ui, icons::X, "清空搜索").clicked() {
                self.search.clear();
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

        if !temporary.is_empty() {
            ui.horizontal(|ui| {
                super::kit::section_label(ui, "临时供应商");
                ui.label(
                    RichText::new(format!("{}", temporary.len()))
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
            ui.add_space(8.0);
            for provider in &temporary {
                self.temporary_provider_card(ui, provider);
                ui.add_space(10.0);
            }
        }
    }

    fn temporary_provider_card(
        &mut self,
        ui: &mut egui::Ui,
        provider: &crate::snapshot::TemporaryProvider,
    ) {
        super::kit::surface(ui, |ui| {
            ui.horizontal(|ui| {
                super::kit::icon_chip(ui, icons::PUZZLE_PIECE, theme::PURPLE, 42.0);
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&provider.name).size(15.5).strong());
                        super::kit::badge(ui, "临时", theme::PURPLE);
                        super::kit::badge(ui, &provider.protocol, theme::BLUE_INFO);
                        if provider.streaming {
                            super::kit::plain_badge(ui, "流式");
                        }
                    });
                    ui.label(
                        RichText::new(format!(
                            "由插件「{}」注入 · {}",
                            provider.plugin_name, provider.id
                        ))
                        .monospace()
                        .size(10.0)
                        .color(theme::muted(ui)),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    super::kit::status_pill(
                        ui,
                        if provider.running {
                            "插件运行中"
                        } else {
                            "插件已停止"
                        },
                        if provider.running {
                            theme::GREEN
                        } else {
                            theme::MUTED
                        },
                    );
                    let supports_refresh = self
                        .snapshot
                        .plugins
                        .iter()
                        .find(|plugin| plugin.id == provider.plugin_id)
                        .map(|plugin| plugin.events.iter().any(|event| event == "onListModels"))
                        .unwrap_or(false);
                    if provider.running
                        && supports_refresh
                        && super::kit::ghost_button(ui, icons::ARROWS_CLOCKWISE, "刷新模型")
                            .on_hover_text("调用插件的 onListModels() 重新查询模型列表")
                            .clicked()
                    {
                        let _ = self
                            .cmd_tx
                            .send(UiCommand::RefreshPluginModels(provider.plugin_id.clone()));
                    }
                });
            });
            if !provider.description.trim().is_empty() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(&provider.description)
                        .size(10.5)
                        .color(theme::muted(ui)),
                );
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("模型").size(10.0).color(theme::muted(ui)));
                for model in provider.models.iter().take(12) {
                    super::kit::plain_badge(ui, model);
                }
                if provider.models.len() > 12 {
                    super::kit::plain_badge(ui, &format!("+{}", provider.models.len() - 12));
                }
            });
            ui.add_space(5.0);
            ui.label(
                RichText::new(
                    "临时供应商、虚拟账号与自动路由只在插件运行时存在，不会写入配置文件；插件禁用或卸载后自动移除。可在「路由」页手动把它选为上游。",
                )
                .size(10.0)
                .color(theme::muted(ui)),
            );
        });
    }

    fn provider_card(&mut self, ui: &mut egui::Ui, provider: &crate::config::Provider) {
        let health = self.snapshot.provider(&provider.id).cloned();
        let account_count = self
            .config
            .accounts
            .iter()
            .filter(|a| a.provider_id == provider.id)
            .count();
        let catalog = self.snapshot.provider_models.get(&provider.id);
        let test_key = format!("provider:{}", provider.id);
        let running = self.snapshot.running_tests.contains(&test_key);
        let speed_key = format!("speed:{}", provider.id);
        let speed_running = self.snapshot.running_tests.contains(&speed_key);
        let brand = brands::for_provider(provider);
        let mut action: Option<CardAction> = None;
        super::kit::surface(ui, |ui| {
            ui.horizontal(|ui| {
                brands::tile(ui, &brand, 42.0).on_hover_text(brand.label);
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&provider.name).size(15.5).strong());
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
                    if super::kit::icon_button(
                        ui,
                        if speed_running {
                            icons::SPINNER
                        } else {
                            icons::GAUGE
                        },
                        "测速：依次测量该供应商全部启用的账号（tok/s）",
                    )
                    .clicked()
                        && !speed_running
                    {
                        action = Some(CardAction::SpeedTest);
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
                if let Some(catalog) = catalog {
                    super::kit::badge(
                        ui,
                        &if catalog.ok {
                            format!("{} 个模型", catalog.models.len())
                        } else {
                            "模型列表不可用".to_string()
                        },
                        if catalog.ok {
                            theme::GREEN
                        } else {
                            theme::ORANGE
                        },
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(health) = &health {
                        if health.balance_known() {
                            ui.label(
                                RichText::new(widgets::format_currency_totals(&health.balances))
                                    .size(13.0)
                                    .strong(),
                            );
                            super::kit::badge(ui, "余额", theme::MUTED);
                        }
                    }
                    if let Some(speed) = self.snapshot.tests.get(&speed_key) {
                        if let Some(tps) = speed.tokens_per_sec {
                            super::kit::badge(ui, &format!("{tps:.0} tok/s"), theme::GREEN);
                        }
                    }
                    if !running {
                        widgets::test_badge(ui, &self.snapshot, &test_key);
                    }
                    if speed_running {
                        ui.spinner();
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
                CardAction::SpeedTest => self.start_test("speed_provider", &provider.id),
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
    SpeedTest,
    Accounts,
}
