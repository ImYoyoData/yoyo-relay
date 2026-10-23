use egui::RichText;
use egui_extras::{Column, TableBuilder};
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
            let table_width = ui.available_width().max(1100.0);
            egui::ScrollArea::horizontal()
                .id_salt("calls_table_h")
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_min_width(table_width);
                    TableBuilder::new(ui)
                        .striped(true)
                        .resizable(true)
                        .vscroll(false)
                        .auto_shrink([false, true])
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .column(Column::initial(68.0).at_least(60.0))
                        .column(Column::initial(86.0).at_least(76.0))
                        .column(Column::initial(170.0).at_least(130.0).clip(true))
                        .column(Column::initial(130.0).at_least(100.0).clip(true))
                        .column(Column::initial(110.0).at_least(90.0).clip(true))
                        .column(Column::initial(68.0).at_least(60.0))
                        .column(Column::initial(68.0).at_least(60.0))
                        .column(Column::initial(82.0).at_least(70.0))
                        .column(Column::initial(118.0).at_least(96.0))
                        .column(Column::initial(88.0).at_least(74.0))
                        .column(Column::remainder().at_least(74.0))
                        .header(26.0, |mut header| {
                            for label in [
                                "状态",
                                "时间",
                                "客户端模型 / 上游",
                                "供应商 / 账号",
                                "协议",
                                "输入",
                                "输出",
                                "总 Tokens",
                                "首字 / 总耗时",
                                "花费",
                                "操作",
                            ] {
                                header.col(|ui| {
                                    ui.label(
                                        RichText::new(label).size(9.0).strong().color(t.muted),
                                    );
                                });
                            }
                        })
                        .body(|mut body| {
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
                                body.row(40.0, |mut table_row| {
                                    table_row.col(|ui| {
                                        kit::status_badge(ui, status_text, status_color);
                                    });
                                    table_row.col(|ui| {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(widgets::format_call_time(row.ts))
                                                    .monospace()
                                                    .size(9.0)
                                                    .color(t.muted),
                                            )
                                            .truncate(),
                                        );
                                    });
                                    table_row.col(|ui| {
                                        ui.vertical(|ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&row.client_model)
                                                        .monospace()
                                                        .size(10.0)
                                                        .strong()
                                                        .color(t.text),
                                                )
                                                .truncate(),
                                            );
                                            if row.upstream_model != row.client_model {
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(format!(
                                                            "→ {}",
                                                            row.upstream_model
                                                        ))
                                                        .monospace()
                                                        .size(8.5)
                                                        .color(t.muted),
                                                    )
                                                    .truncate(),
                                                );
                                            }
                                        });
                                    });
                                    table_row.col(|ui| {
                                        ui.vertical(|ui| {
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
                                        });
                                    });
                                    table_row.col(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&row.protocol)
                                                        .monospace()
                                                        .size(8.5)
                                                        .color(t.text),
                                                )
                                                .truncate(),
                                            );
                                            if row.stream {
                                                ui.label(
                                                    RichText::new("流").size(8.0).color(t.muted),
                                                );
                                            }
                                        });
                                    });
                                    for value in
                                        [row.prompt_tokens, row.completion_tokens, row.total_tokens]
                                    {
                                        table_row.col(|ui| {
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(widgets::format_count(value))
                                                            .monospace()
                                                            .size(9.5)
                                                            .color(t.text),
                                                    );
                                                },
                                            );
                                        });
                                    }
                                    table_row.col(|ui| {
                                        ui.vertical(|ui| {
                                            if row.first_token_ms > 0 {
                                                ui.label(
                                                    RichText::new(format!(
                                                        "首字 {}",
                                                        widgets::format_duration_ms(
                                                            row.first_token_ms
                                                        )
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
                                        });
                                    });
                                    table_row.col(|ui| {
                                        ui.label(
                                            RichText::new(widgets::format_cost_with(
                                                row.cost_micros,
                                                &self.config.ui,
                                            ))
                                            .size(9.5)
                                            .color(theme::ORANGE),
                                        );
                                    });
                                    table_row.col(|ui| {
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if kit::danger_icon_button(
                                                    ui,
                                                    icons::TRASH_SIMPLE,
                                                    "删除该记录",
                                                )
                                                .clicked()
                                                {
                                                    delete = Some(row.id);
                                                }
                                                ui.label(
                                                    RichText::new(row.status.to_string())
                                                        .monospace()
                                                        .size(8.5)
                                                        .color(if row.ok {
                                                            t.muted
                                                        } else {
                                                            theme::RED
                                                        }),
                                                );
                                            },
                                        );
                                    });
                                });
                            }
                        });
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
