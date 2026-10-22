use egui::RichText;
use egui_phosphor::regular as icons;

use crate::runtime::UiCommand;
use crate::snapshot::UsageDimension;

use super::{theme, widgets, App};

impl App {
    pub(crate) fn page_usage(&mut self, ui: &mut egui::Ui) {
        super::kit::page_header(
            ui,
            super::Page::Usage.eyebrow(),
            "用量",
            "按供应商、账号与客户端模型查看请求、Token 与花费",
            |ui| {
                let _ = ui;
            },
        );
        let narrow = ui.available_width() < 900.0;
        if narrow {
            ui.horizontal(|ui| {
                if let Some(days) = widgets::segmented(
                    ui,
                    &[(1u32, "今日"), (7, "近 7 天"), (30, "近 30 天")],
                    self.usage_days,
                ) {
                    self.usage_days = days;
                    self.usage_pinned = None;
                    self.send_usage_view();
                }
            });
            ui.horizontal(|ui| {
                if let Some(dim) = widgets::segmented(
                    ui,
                    &[
                        (UsageDimension::Global, "全局"),
                        (UsageDimension::Provider, "供应商"),
                        (UsageDimension::Account, "账号"),
                        (UsageDimension::Model, "模型"),
                    ],
                    self.usage_dim,
                ) {
                    self.usage_dim = dim;
                    self.usage_pinned = None;
                    self.send_usage_view();
                }
            });
        } else {
            ui.horizontal(|ui| {
                if let Some(days) = widgets::segmented(
                    ui,
                    &[(1u32, "今日"), (7, "近 7 天"), (30, "近 30 天")],
                    self.usage_days,
                ) {
                    self.usage_days = days;
                    self.usage_pinned = None;
                    self.send_usage_view();
                }
                ui.separator();
                if let Some(dim) = widgets::segmented(
                    ui,
                    &[
                        (UsageDimension::Global, "全局"),
                        (UsageDimension::Provider, "供应商"),
                        (UsageDimension::Account, "账号"),
                        (UsageDimension::Model, "客户端模型"),
                    ],
                    self.usage_dim,
                ) {
                    self.usage_dim = dim;
                    self.usage_pinned = None;
                    self.send_usage_view();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!(
                            "统计自 {}",
                            widgets::format_day(self.snapshot.usage.range_start)
                        ))
                        .size(10.0)
                        .color(theme::muted(ui)),
                    );
                });
            });
        }
        ui.add_space(10.0);

        let usage = self.snapshot.usage.clone();
        let totals = &usage.totals;
        let cards: Vec<(&str, &str, String, String, egui::Color32)> = vec![
            (
                icons::PAPER_PLANE_TILT,
                "请求数",
                widgets::format_count(totals.requests),
                format!("成功 {}", widgets::format_count(totals.ok)),
                theme::ACCENT,
            ),
            (
                icons::COINS,
                "Token",
                widgets::format_count(totals.total_tokens),
                format!(
                    "入 {} · 出 {}",
                    widgets::format_count(totals.prompt_tokens),
                    widgets::format_count(totals.completion_tokens)
                ),
                theme::BLUE_INFO,
            ),
            (
                icons::CHECK_CIRCLE,
                "成功率",
                if totals.requests == 0 {
                    "—".to_string()
                } else {
                    format!("{:.1}%", totals.success_rate() * 100.0)
                },
                format!("平均延迟 {} ms", totals.avg_latency_ms),
                if totals.requests > 0 && totals.success_rate() >= 0.95 {
                    theme::GREEN
                } else {
                    theme::ORANGE
                },
            ),
            (
                icons::CURRENCY_DOLLAR,
                "预估花费",
                widgets::format_cost(totals.cost_micros),
                if usage.days == 1 {
                    "今日 · 按路由价格".to_string()
                } else {
                    "所选区间 · 按路由价格".to_string()
                },
                theme::ORANGE,
            ),
        ];
        super::kit::kpi_grid(ui, &cards);
        ui.add_space(12.0);

        super::kit::surface(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icons::CHART_LINE)
                        .size(15.0)
                        .color(theme::ACCENT),
                );
                ui.label(RichText::new("用量趋势").size(14.0).strong());
                ui.label(
                    RichText::new(if usage.bucket_secs >= 86_400 {
                        "按天统计"
                    } else {
                        "按小时统计"
                    })
                    .size(10.0)
                    .color(theme::muted(ui)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(metric) = super::kit::segmented(
                        ui,
                        &[
                            (super::UsageMetric::Tokens, "Tokens"),
                            (super::UsageMetric::Requests, "请求"),
                            (super::UsageMetric::Cost, "花费"),
                        ],
                        self.usage_metric,
                    ) {
                        self.usage_metric = metric;
                    }
                });
            });
            ui.add_space(8.0);
            let points = super::kit::filled_chart_points(
                &usage.series,
                usage.range_start,
                crate::config::now_ts(),
                usage.bucket_secs,
            );
            let metric = match self.usage_metric {
                super::UsageMetric::Tokens => super::kit::ChartMetric::Tokens,
                super::UsageMetric::Requests => super::kit::ChartMetric::Requests,
                super::UsageMetric::Cost => super::kit::ChartMetric::Cost,
            };
            #[cfg(debug_assertions)]
            if self.usage_pinned.is_none()
                && std::env::var("YOYO_RELAY_DEMO").ok().as_deref() == Some("pin")
            {
                self.usage_pinned = points
                    .iter()
                    .rposition(|point| point.tokens > 0 || point.requests > 0);
            }
            if let Some(index) = super::kit::interactive_chart(
                ui,
                &points,
                theme::ACCENT,
                190.0,
                metric,
                usage.bucket_secs,
            ) {
                if self.usage_pinned == Some(index) {
                    self.usage_pinned = None;
                } else {
                    self.usage_pinned = Some(index);
                }
            }
            if let Some(index) = self.usage_pinned {
                if let Some(point) = points.get(index) {
                    ui.add_space(6.0);
                    super::kit::soft_panel(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            super::kit::badge(ui, "已选中", theme::ACCENT);
                            ui.label(
                                RichText::new(widgets::format_day(point.bucket))
                                    .size(10.5)
                                    .color(theme::muted(ui)),
                            );
                            let text = match metric {
                                super::kit::ChartMetric::Tokens => {
                                    format!("{} tokens", widgets::format_count(point.tokens))
                                }
                                super::kit::ChartMetric::Requests => {
                                    format!("{} 次请求", point.requests)
                                }
                                super::kit::ChartMetric::Cost => {
                                    widgets::format_cost(point.cost_micros)
                                }
                            };
                            ui.label(RichText::new(text).size(11.0).strong());
                            ui.label(
                                RichText::new("再次点击图表可取消选中")
                                    .size(9.5)
                                    .color(theme::muted(ui)),
                            );
                        });
                    });
                }
            }
        });
        ui.add_space(12.0);

        super::kit::surface(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icons::LIST_BULLETS)
                        .size(15.0)
                        .color(theme::ACCENT),
                );
                ui.label(
                    RichText::new(match usage.dimension {
                        UsageDimension::Global => "全局（按客户端模型拆分）",
                        UsageDimension::Provider => "按供应商",
                        UsageDimension::Account => "按账号",
                        UsageDimension::Model => "按客户端模型（可展开上游拆分）",
                    })
                    .size(14.0)
                    .strong(),
                );
                ui.label(
                    RichText::new(format!("{} 组", usage.groups.len()))
                        .size(10.5)
                        .color(theme::muted(ui)),
                );
            });
            ui.add_space(8.0);
            if usage.groups.is_empty() {
                ui.label(
                    RichText::new("所选时间范围内没有用量记录。")
                        .size(11.5)
                        .color(theme::muted(ui)),
                );
                return;
            }
            let mut toggled: Option<String> = None;
            for group in &usage.groups {
                let expanded = self.usage_expanded.contains(&group.key);
                let pairs: Vec<_> = usage
                    .model_pairs
                    .iter()
                    .filter(|(client, _, _)| client == &group.key)
                    .cloned()
                    .collect();
                egui::Frame::new()
                    .fill(theme::subtle_fill(ui))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::symmetric(12, 9))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if usage.dimension == UsageDimension::Model {
                                if super::kit::icon_button(
                                    ui,
                                    if expanded {
                                        icons::CARET_DOWN
                                    } else {
                                        icons::CARET_RIGHT
                                    },
                                    if expanded {
                                        "收起上游拆分"
                                    } else {
                                        "展开上游拆分"
                                    },
                                )
                                .clicked()
                                {
                                    toggled = Some(group.key.clone());
                                }
                            }
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(&group.label)
                                        .size(12.5)
                                        .strong()
                                        .color(theme::text(ui)),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "{} 请求 · {} 成功 · 平均 {} ms",
                                        group.totals.requests,
                                        group.totals.ok,
                                        group.totals.avg_latency_ms
                                    ))
                                    .size(9.5)
                                    .color(theme::muted(ui)),
                                );
                            });
                            let metrics_width = 4.0 * 82.0 + 3.0 * 10.0;
                            let remaining = ui.available_width();
                            if remaining > metrics_width + 16.0 {
                                ui.add_space(remaining - metrics_width);
                            }
                            metric_column(
                                ui,
                                "请求 / 成功",
                                &format!("{} / {}", group.totals.requests, group.totals.ok),
                                false,
                            );
                            metric_column(
                                ui,
                                "Tokens",
                                &widgets::format_count(group.totals.total_tokens),
                                true,
                            );
                            metric_column(
                                ui,
                                "花费",
                                &widgets::format_cost(group.totals.cost_micros),
                                false,
                            );
                            metric_column(
                                ui,
                                "平均延迟",
                                &format!("{} ms", group.totals.avg_latency_ms),
                                false,
                            );
                        });
                        if expanded {
                            ui.add_space(6.0);
                            for (_, upstream, totals) in pairs {
                                egui::Frame::new()
                                    .fill(theme::tokens(ui).card)
                                    .corner_radius(8)
                                    .inner_margin(egui::Margin::symmetric(10, 7))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(format!("↳ {upstream}"))
                                                    .monospace()
                                                    .size(10.0)
                                                    .color(theme::muted(ui)),
                                            );
                                            let metrics_width = 3.0 * 82.0 + 2.0 * 10.0;
                                            let remaining = ui.available_width();
                                            if remaining > metrics_width + 16.0 {
                                                ui.add_space(remaining - metrics_width);
                                            }
                                            metric_column(
                                                ui,
                                                "请求 / 成功",
                                                &format!("{} / {}", totals.requests, totals.ok),
                                                false,
                                            );
                                            metric_column(
                                                ui,
                                                "Tokens",
                                                &widgets::format_count(totals.total_tokens),
                                                true,
                                            );
                                            metric_column(
                                                ui,
                                                "花费",
                                                &widgets::format_cost(totals.cost_micros),
                                                false,
                                            );
                                        });
                                    });
                                ui.add_space(4.0);
                            }
                        }
                    });
                ui.add_space(6.0);
            }
            if let Some(key) = toggled {
                if !self.usage_expanded.remove(&key) {
                    self.usage_expanded.insert(key);
                }
            }
        });
        ui.add_space(10.0);
        ui.label(
            RichText::new(
                "统计只包含正式请求；测试按钮的探测请求不计入。非聊天模型单独计为 non_chat。",
            )
            .size(10.5)
            .color(theme::muted(ui)),
        );
    }

    fn send_usage_view(&mut self) {
        let _ = self.cmd_tx.send(UiCommand::SetUsageView {
            days: self.usage_days,
            dimension: self.usage_dim,
        });
    }
}

fn metric_column(ui: &mut egui::Ui, label: &str, value: &str, emphasize: bool) {
    ui.allocate_ui_with_layout(
        egui::Vec2::new(82.0, 34.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(82.0);
            ui.label(RichText::new(label).size(8.5).color(theme::muted(ui)));
            let text = RichText::new(value).size(11.0).monospace();
            ui.label(if emphasize {
                text.strong().color(theme::text(ui))
            } else {
                text.color(theme::text(ui))
            });
        },
    );
    ui.add_space(10.0);
}
