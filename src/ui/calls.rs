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
            "按设置中的保留天数清理 · 每页 20 条 · 可按模型、账号与结果筛选",
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
        let t = theme::tokens(ui);
        kit::surface(ui, |ui| {
            // Fixed column widths scaled to the available width keep every
            // header and value on the same vertical line.
            let base: [f32; 11] = [
                64.0, 98.0, 180.0, 150.0, 112.0, 76.0, 76.0, 86.0, 122.0, 92.0, 72.0,
            ];
            let base_total: f32 = base.iter().sum::<f32>() + 10.0 * 10.0;
            let scale = (ui.available_width() / base_total).clamp(0.72, 1.35);
            let widths: Vec<f32> = base.iter().map(|w| w * scale).collect();
            egui::Grid::new("calls_table")
                .num_columns(11)
                .striped(true)
                .spacing([10.0 * scale, 6.0])
                .show(ui, |ui| {
                    for (label, width) in [
                        ("状态", widths[0]),
                        ("时间", widths[1]),
                        ("客户端模型 / 上游", widths[2]),
                        ("供应商 / 账号", widths[3]),
                        ("协议", widths[4]),
                        ("输入", widths[5]),
                        ("输出", widths[6]),
                        ("总 Tokens", widths[7]),
                        ("首字 / 总耗时", widths[8]),
                        ("花费", widths[9]),
                        ("操作", widths[10]),
                    ] {
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, 22.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_width(width);
                                ui.label(RichText::new(label).size(9.0).strong().color(t.muted));
                            },
                        );
                    }
                    ui.end_row();

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

                        // 状态
                        ui.allocate_ui_with_layout(
                            egui::vec2(widths[0], 34.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_width(widths[0]);
                                kit::status_badge(ui, status_text, status_color);
                            },
                        );
                        // 时间
                        ui.allocate_ui_with_layout(
                            egui::vec2(widths[1], 34.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_width(widths[1]);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(widgets::format_call_time(row.ts))
                                            .monospace()
                                            .size(9.0)
                                            .color(t.muted),
                                    )
                                    .truncate(),
                                );
                            },
                        );
                        // 模型
                        ui.allocate_ui_with_layout(
                            egui::vec2(widths[2], 34.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(widths[2]);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&row.client_model)
                                            .monospace()
                                            .size(10.5)
                                            .strong()
                                            .color(t.text),
                                    )
                                    .truncate(),
                                );
                                if row.upstream_model != row.client_model {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!("→ {}", row.upstream_model))
                                                .monospace()
                                                .size(8.5)
                                                .color(t.muted),
                                        )
                                        .truncate(),
                                    );
                                }
                            },
                        );
                        // 供应商 / 账号
                        ui.allocate_ui_with_layout(
                            egui::vec2(widths[3], 34.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(widths[3]);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(provider).size(9.5).color(t.text),
                                    )
                                    .truncate(),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(account).size(8.5).color(t.muted),
                                    )
                                    .truncate(),
                                );
                            },
                        );
                        // 协议
                        ui.allocate_ui_with_layout(
                            egui::vec2(widths[4], 34.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_width(widths[4]);
                                ui.horizontal_wrapped(|ui| {
                                    kit::plain_badge(ui, &row.protocol);
                                    if row.stream {
                                        kit::plain_badge(ui, "流式");
                                    }
                                    if row.kind != "chat" {
                                        kit::plain_badge(ui, &row.kind);
                                    }
                                });
                            },
                        );
                        // 输入 / 输出 / 总
                        for (width, value) in [
                            (widths[5], row.prompt_tokens),
                            (widths[6], row.completion_tokens),
                            (widths[7], row.total_tokens),
                        ] {
                            ui.allocate_ui_with_layout(
                                egui::vec2(width, 34.0),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.set_width(width);
                                    ui.label(
                                        RichText::new(widgets::format_count(value))
                                            .monospace()
                                            .size(10.0)
                                            .color(t.text),
                                    );
                                },
                            );
                        }
                        // 首字 / 总耗时
                        ui.allocate_ui_with_layout(
                            egui::vec2(widths[8], 34.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(widths[8]);
                                if row.first_token_ms > 0 {
                                    ui.label(
                                        RichText::new(format!(
                                            "首字 {}",
                                            widgets::format_duration_ms(row.first_token_ms)
                                        ))
                                        .monospace()
                                        .size(8.5)
                                        .color(t.muted),
                                    );
                                }
                                ui.label(
                                    RichText::new(format!(
                                        "总 {}",
                                        widgets::format_duration_ms(row.latency_ms)
                                    ))
                                    .monospace()
                                    .size(9.0)
                                    .color(t.muted),
                                );
                            },
                        );
                        // 花费
                        ui.allocate_ui_with_layout(
                            egui::vec2(widths[9], 34.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_width(widths[9]);
                                ui.label(
                                    RichText::new(widgets::format_cost_with(
                                        row.cost_micros,
                                        &self.config.ui,
                                    ))
                                    .size(10.0)
                                    .color(theme::ORANGE),
                                );
                            },
                        );
                        // HTTP + 操作
                        ui.allocate_ui_with_layout(
                            egui::vec2(widths[10], 34.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_width(widths[10]);
                                ui.label(
                                    RichText::new(row.status.to_string())
                                        .monospace()
                                        .size(8.5)
                                        .color(if row.ok { t.muted } else { theme::RED }),
                                );
                                if kit::danger_icon_button(ui, icons::TRASH_SIMPLE, "删除该记录")
                                    .clicked()
                                {
                                    delete = Some(row.id);
                                }
                            },
                        );
                        ui.end_row();
                    }
                });
        });
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
                    RichText::new(format!("每页 {} 条 · 历史按保留天数清理", calls.page_size))
                        .size(9.5)
                        .color(theme::muted(ui)),
                );
            });
        });
        if let Some(page) = goto {
            let _ = self.cmd_tx.send(UiCommand::SetCallsPage(page));
        }
        ui.add_space(6.0);
        super::kit::info_row(
            ui,
            icons::INFO,
            "缓存命中来自上游返回的 usage；上游未上报该字段时显示 0。耗时统一换算为秒并保留两位小数。",
        );
    }
}
