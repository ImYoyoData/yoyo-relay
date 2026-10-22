use egui::RichText;
use egui_phosphor::regular as icons;

use crate::db::UsageFilter;
use crate::runtime::UiCommand;
use crate::snapshot::NoticeLevel;

use super::{kit, theme, widgets, App, ConfirmAction, Page};

impl App {
    pub(crate) fn page_calls(&mut self, ui: &mut egui::Ui) {
        let calls = self.snapshot.calls.clone();
        let mut clear = false;
        kit::page_header(
            ui,
            Page::Calls.eyebrow(),
            "调用明细",
            "最近 1000 条请求 · 每页 20 条 · 可按模型、账号与结果筛选",
            |ui| {
                if kit::danger_button(ui, icons::TRASH_SIMPLE, "清空明细").clicked() {
                    clear = true;
                }
            },
        );
        if clear {
            self.confirm = Some(ConfirmAction::ClearCalls);
        }

        let mut filter = calls.filter.clone();
        let mut filter_changed = false;
        kit::surface(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("模型").size(10.5).color(theme::muted(ui)));
                let selected = filter
                    .client_model
                    .clone()
                    .unwrap_or_else(|| "全部模型".to_string());
                egui::ComboBox::from_id_salt("calls_model_filter")
                    .selected_text(selected)
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(filter.client_model.is_none(), "全部模型")
                            .clicked()
                        {
                            filter.client_model = None;
                            filter_changed = true;
                        }
                        for model in self.config.client_models() {
                            let active = filter.client_model.as_deref() == Some(model.as_str());
                            if ui.selectable_label(active, &model).clicked() {
                                filter.client_model = Some(model);
                                filter_changed = true;
                            }
                        }
                    });

                ui.label(RichText::new("账号").size(10.5).color(theme::muted(ui)));
                let selected = filter
                    .account_id
                    .as_deref()
                    .and_then(|id| self.config.account(id))
                    .map(|account| account.name.clone())
                    .unwrap_or_else(|| "全部账号".to_string());
                egui::ComboBox::from_id_salt("calls_account_filter")
                    .selected_text(selected)
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(filter.account_id.is_none(), "全部账号")
                            .clicked()
                        {
                            filter.account_id = None;
                            filter_changed = true;
                        }
                        for account in &self.config.accounts {
                            let provider = self
                                .config
                                .provider(&account.provider_id)
                                .map(|p| p.name.as_str())
                                .unwrap_or("—");
                            let active = filter.account_id.as_deref() == Some(account.id.as_str());
                            if ui
                                .selectable_label(
                                    active,
                                    format!("{} · {}", account.name, provider),
                                )
                                .clicked()
                            {
                                filter.account_id = Some(account.id.clone());
                                filter_changed = true;
                            }
                        }
                    });

                ui.label(RichText::new("结果").size(10.5).color(theme::muted(ui)));
                if let Some(ok) = widgets::segmented(
                    ui,
                    &[(None, "全部"), (Some(true), "成功"), (Some(false), "失败")],
                    filter.ok,
                ) {
                    filter.ok = ok;
                    filter_changed = true;
                }

                if filter != UsageFilter::default() {
                    if kit::ghost_button(ui, icons::X, "重置筛选").clicked() {
                        filter = UsageFilter::default();
                        filter_changed = true;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("共 {} 条", calls.total))
                            .size(10.5)
                            .color(theme::muted(ui)),
                    );
                });
            });
        });
        if filter_changed {
            let _ = self.cmd_tx.send(UiCommand::SetCallsFilter(filter));
        }
        ui.add_space(10.0);

        if calls.rows.is_empty() {
            if kit::empty_state(
                ui,
                icons::LIST_BULLETS,
                "没有匹配的调用记录",
                "发起一次请求后这里会自动出现明细；也可以调整上方的筛选条件。",
                icons::ARROWS_CLOCKWISE,
                "全部刷新",
            ) {
                let _ = self.cmd_tx.send(UiCommand::RefreshBalance(None));
            }
            return;
        }

        let mut delete: Option<i64> = None;
        let now = crate::config::now_ts();
        for row in &calls.rows {
            let status_color = if row.cancelled {
                theme::ORANGE
            } else if row.ok {
                theme::GREEN
            } else {
                theme::RED
            };
            let status_text = if row.cancelled {
                "已取消"
            } else if row.ok {
                "成功"
            } else {
                "失败"
            };
            let provider = self
                .config
                .provider(&row.provider_id)
                .map(|p| p.name.as_str())
                .unwrap_or("—");
            let account = self
                .config
                .account(&row.account_id)
                .map(|a| a.name.as_str())
                .unwrap_or("—");
            egui::Frame::new()
                .fill(theme::subtle_fill(ui))
                .corner_radius(9)
                .inner_margin(egui::Margin::symmetric(11, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        kit::status_badge(ui, status_text, status_color);
                        ui.label(
                            RichText::new(widgets::format_time(row.ts))
                                .monospace()
                                .size(9.5)
                                .color(theme::muted(ui)),
                        );
                        ui.label(
                            RichText::new(&row.client_model)
                                .monospace()
                                .size(11.5)
                                .strong(),
                        );
                        if row.upstream_model != row.client_model {
                            ui.label(
                                RichText::new(icons::ARROW_RIGHT)
                                    .size(10.0)
                                    .color(theme::muted(ui)),
                            );
                            ui.label(
                                RichText::new(&row.upstream_model)
                                    .monospace()
                                    .size(10.0)
                                    .color(theme::muted(ui)),
                            );
                        }
                        if row.stream {
                            kit::plain_badge(ui, "流式");
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if kit::danger_icon_button(ui, icons::TRASH_SIMPLE, "删除该记录")
                                .clicked()
                            {
                                delete = Some(row.id);
                            }
                            ui.label(
                                RichText::new(widgets::format_cost(row.cost_micros))
                                    .size(10.5)
                                    .color(theme::ORANGE),
                            );
                            ui.label(
                                RichText::new(format!("{} ms", row.latency_ms))
                                    .monospace()
                                    .size(9.5)
                                    .color(theme::muted(ui)),
                            );
                            ui.label(
                                RichText::new(widgets::time_ago(row.ts, now))
                                    .size(9.0)
                                    .color(theme::muted(ui)),
                            );
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(format!("{provider} / {account}"))
                                .size(9.5)
                                .color(theme::muted(ui)),
                        );
                        kit::plain_badge(ui, &row.protocol);
                        ui.label(
                            RichText::new(format!(
                                "入 {}（命中 {}） · 出 {} · 总 {}",
                                widgets::format_count(row.prompt_tokens),
                                widgets::format_count(row.cached_tokens),
                                widgets::format_count(row.completion_tokens),
                                widgets::format_count(row.total_tokens)
                            ))
                            .size(9.5)
                            .color(theme::muted(ui)),
                        );
                        ui.label(
                            RichText::new(format!("HTTP {}", row.status))
                                .monospace()
                                .size(9.0)
                                .color(if row.ok { theme::muted(ui) } else { theme::RED }),
                        );
                        if row.kind != "chat" {
                            kit::plain_badge(ui, &row.kind);
                        }
                    });
                });
            ui.add_space(5.0);
        }
        if let Some(id) = delete {
            let _ = self.cmd_tx.send(UiCommand::DeleteUsage(id));
            self.toast(NoticeLevel::Info, "正在删除记录…");
        }

        ui.add_space(6.0);
        let pages = ((calls.total as usize).saturating_sub(1)) / calls.page_size.max(1) + 1;
        let mut goto: Option<usize> = None;
        ui.horizontal(|ui| {
            if kit::secondary_button(ui, icons::CARET_LEFT, "上一页").clicked() && calls.page > 0
            {
                goto = Some(calls.page - 1);
            }
            ui.label(
                RichText::new(format!("第 {} / {} 页", calls.page + 1, pages))
                    .size(10.5)
                    .color(theme::muted(ui)),
            );
            if kit::secondary_button(ui, icons::CARET_RIGHT, "下一页").clicked()
                && calls.page + 1 < pages
            {
                goto = Some(calls.page + 1);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("每页 {} 条 · 仅保留最近 1000 条", calls.page_size))
                        .size(9.5)
                        .color(theme::muted(ui)),
                );
            });
        });
        if let Some(page) = goto {
            let _ = self.cmd_tx.send(UiCommand::SetCallsPage(page));
        }
    }
}
