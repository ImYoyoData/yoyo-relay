use armas::components::{Alert, AlertVariant};
use armas::ext::ArmasContextExt;
use egui::{RichText, Vec2};
use egui_phosphor::regular as icons;

use crate::config::now_ts;
use crate::snapshot::{AlertLevel, ProxyStatus};

use super::{theme, widgets, App};

impl App {
    pub(crate) fn page_overview(&mut self, ui: &mut egui::Ui) {
        super::kit::page_header(
            ui,
            super::Page::Overview.eyebrow(),
            "概览",
            "代理健康、今日流量、供应商状态与最近请求",
            |ui| {
                if super::kit::secondary_labeled(ui, icons::CHART_BAR, "查看用量").clicked() {
                    self.page = super::Page::Usage;
                }
            },
        );

        super::kit::kpi_cards_from_snapshot(ui, &self.snapshot);
        ui.add_space(4.0);

        if ui.available_width() < 900.0 {
            self.proxy_card(ui);
            ui.add_space(10.0);
            self.alert_card(ui);
        } else {
            ui.columns(2, |cols| {
                self.proxy_card(&mut cols[0]);
                self.alert_card(&mut cols[1]);
            });
        }
        ui.add_space(12.0);

        let points = super::kit::filled_chart_points(
            &self.snapshot.today.series,
            crate::db::today_start(),
            crate::config::now_ts(),
            3600,
        );
        super::kit::section_card(
            ui,
            icons::CHART_LINE,
            "今日用量趋势",
            "按小时统计 · tokens",
            |ui| {
                super::kit::badge(ui, "今日", theme::ACCENT);
            },
            |ui| {
                super::kit::interactive_chart(
                    ui,
                    &points,
                    theme::ACCENT,
                    170.0,
                    super::kit::ChartMetric::Tokens,
                    3600,
                );
            },
        );
        ui.add_space(12.0);

        let provider_count = self.snapshot.providers.len();
        let mut go_providers = false;
        super::kit::section_card(
            ui,
            icons::HARD_DRIVES,
            "供应商健康度",
            &format!("{provider_count} 个供应商 · 账号池与余额"),
            |ui| {
                if super::kit::ghost_labeled(ui, icons::ARROW_RIGHT, "管理").clicked() {
                    go_providers = true;
                }
            },
            |ui| {
                if self.snapshot.providers.is_empty() {
                    if super::kit::empty_state(
                        ui,
                        icons::HARD_DRIVES,
                        "尚未配置供应商",
                        "添加供应商与账号后，这里会显示账号池健康度。",
                        icons::PLUS,
                        "添加供应商",
                    ) {
                        self.page = super::Page::Providers;
                        self.open_new_provider();
                    }
                    return;
                }
                for provider in &self.snapshot.providers {
                    self.provider_row(ui, provider);
                }
            },
        );
        if go_providers {
            self.page = super::Page::Providers;
        }
        ui.add_space(12.0);

        let recent_count = self
            .snapshot
            .calls
            .total
            .max(self.snapshot.recent.len() as u64);
        let mut go_calls = false;
        super::kit::section_card(
            ui,
            icons::LIST_BULLETS,
            "最近请求",
            &format!("共 {recent_count} 条 · 最新 12 条 · 点击「调用明细」查看分页记录"),
            |ui| {
                if super::kit::ghost_labeled(ui, icons::LIST_BULLETS, "调用明细").clicked() {
                    go_calls = true;
                }
            },
            |ui| {
                if self.snapshot.recent.is_empty() {
                    super::kit::empty_state(
                        ui,
                        icons::PAPER_PLANE_TILT,
                        "还没有请求",
                        "把客户端指向本地端点并发起一次请求即可在这里看到。",
                        icons::KEY,
                        "查看本地 Key",
                    );
                    if ui.small_button("查看本地 Key").clicked() {
                        self.page = super::Page::LocalKeys;
                    }
                    return;
                }
                let rows: Vec<_> = self.snapshot.recent.iter().take(12).cloned().collect();
                for request in &rows {
                    egui::Frame::new()
                        .fill(theme::subtle_fill(ui))
                        .corner_radius(9)
                        .inner_margin(egui::Margin::symmetric(10, 8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(widgets::time_ago(request.ts, now_ts()))
                                        .size(9.5)
                                        .color(theme::muted(ui)),
                                );
                                ui.label(
                                    RichText::new(if request.client_model.is_empty() {
                                        "—"
                                    } else {
                                        &request.client_model
                                    })
                                    .monospace()
                                    .size(11.0)
                                    .strong(),
                                );
                                let (text, color) = if request.cancelled {
                                    ("取消", theme::ORANGE)
                                } else if request.ok {
                                    ("成功", theme::GREEN)
                                } else {
                                    ("失败", theme::RED)
                                };
                                super::kit::status_badge(ui, text, color);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new(icons::COPY)
                                                        .size(12.0)
                                                        .color(theme::muted(ui)),
                                                )
                                                .frame(false)
                                                .min_size(Vec2::splat(20.0)),
                                            )
                                            .on_hover_text("复制 curl 命令")
                                            .clicked()
                                        {
                                            let curl = self.curl_for(request);
                                            self.copy(ui.ctx(), &curl, "curl 命令");
                                        }
                                        ui.label(
                                            RichText::new(format!("{} ms", request.latency_ms))
                                                .monospace()
                                                .size(9.5)
                                                .color(theme::muted(ui)),
                                        );
                                        ui.label(
                                            RichText::new(widgets::format_count(
                                                request.total_tokens,
                                            ))
                                            .monospace()
                                            .size(10.0),
                                        );
                                        ui.label(
                                            RichText::new(widgets::format_cost(
                                                request.cost_micros,
                                            ))
                                            .size(9.5)
                                            .color(theme::ORANGE),
                                        );
                                    },
                                );
                            });
                            ui.horizontal(|ui| {
                                let provider = self
                                    .config
                                    .provider(&request.provider_id)
                                    .map(|p| p.name.as_str())
                                    .unwrap_or("—");
                                let account = self
                                    .config
                                    .account(&request.account_id)
                                    .map(|a| a.name.as_str())
                                    .unwrap_or("—");
                                ui.label(
                                    RichText::new(format!("{provider} / {account}"))
                                        .size(9.5)
                                        .color(theme::muted(ui)),
                                );
                                super::kit::badge(ui, &request.protocol, theme::MUTED);
                                if !request.upstream_model.is_empty()
                                    && request.upstream_model != request.client_model
                                {
                                    ui.label(
                                        RichText::new(format!(
                                            "{} → {}",
                                            request.client_model, request.upstream_model
                                        ))
                                        .monospace()
                                        .size(9.0)
                                        .color(theme::muted(ui)),
                                    );
                                }
                                if request.cached_tokens > 0 {
                                    super::kit::badge(
                                        ui,
                                        &format!(
                                            "缓存 {}",
                                            widgets::format_count(request.cached_tokens)
                                        ),
                                        theme::PURPLE,
                                    );
                                }
                                ui.label(
                                    RichText::new(format!(
                                        "入 {} · 出 {}",
                                        widgets::format_count(request.prompt_tokens),
                                        widgets::format_count(request.completion_tokens)
                                    ))
                                    .size(9.0)
                                    .color(theme::muted(ui)),
                                );
                            });
                        });
                    ui.add_space(4.0);
                }
            },
        );
        if go_calls {
            self.page = super::Page::Calls;
        }
    }

    fn provider_row(&self, ui: &mut egui::Ui, provider: &crate::snapshot::ProviderHealth) {
        egui::Frame::new()
            .fill(theme::subtle_fill(ui))
            .corner_radius(9)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    super::kit::icon_tile(ui, icons::HARD_DRIVES, theme::ACCENT, 28.0);
                    ui.add_space(2.0);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&provider.name).size(12.5).strong());
                            super::kit::badge(ui, &provider.kind, theme::MUTED);
                        });
                        ui.horizontal(|ui| {
                            widgets::dot(ui, theme::GREEN);
                            ui.label(
                                RichText::new(format!("健康 {}", provider.healthy))
                                    .size(9.5)
                                    .color(theme::muted(ui)),
                            );
                            widgets::dot(ui, theme::ORANGE);
                            ui.label(
                                RichText::new(format!(
                                    "冷却 {}",
                                    provider.cooling + provider.half_open
                                ))
                                .size(9.5)
                                .color(theme::muted(ui)),
                            );
                            widgets::dot(ui, theme::RED);
                            ui.label(
                                RichText::new(format!("耗尽 {}", provider.exhausted))
                                    .size(9.5)
                                    .color(theme::muted(ui)),
                            );
                        });
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if provider.all_unavailable() {
                            super::kit::status_badge(ui, "不可用", theme::RED);
                        } else if provider.account_count == 0 {
                            super::kit::status_badge(ui, "无账号", theme::MUTED);
                        } else {
                            super::kit::status_badge(ui, "可用", theme::GREEN);
                        }
                        if provider.balance_known() {
                            ui.label(
                                RichText::new(widgets::format_currency_totals(&provider.balances))
                                    .size(12.0)
                                    .strong(),
                            );
                        }
                        ui.label(
                            RichText::new(format!("{} 个账号", provider.account_count))
                                .size(9.5)
                                .color(theme::muted(ui)),
                        );
                    });
                });
            });
        ui.add_space(5.0);
    }

    fn proxy_card(&mut self, ui: &mut egui::Ui) {
        let theme = ui.ctx().armas_theme();
        let (proxy_color, proxy_label) = match &self.snapshot.proxy {
            ProxyStatus::Running => (theme::GREEN, "运行中"),
            ProxyStatus::Starting => (theme::ORANGE, "启动中"),
            ProxyStatus::Stopped => (theme::MUTED, "已停止"),
            ProxyStatus::Error(_) => (theme::RED, "异常"),
        };
        super::kit::section_card(
            ui,
            icons::DESKTOP_TOWER,
            "代理状态",
            "本地端点与运行控制",
            |ui| {
                super::kit::status_badge(ui, proxy_label, proxy_color);
            },
            |ui| {
                if let ProxyStatus::Error(err) = &self.snapshot.proxy {
                    Alert::new(err).variant(AlertVariant::Destructive).show(ui);
                    ui.add_space(8.0);
                }
                let endpoint = self.snapshot.endpoint.clone();
                egui::Frame::new()
                    .fill(theme.muted())
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(10, 7))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(icons::PLUGS_CONNECTED)
                                    .size(14.0)
                                    .color(theme.primary()),
                            );
                            ui.label(
                                RichText::new(if endpoint.is_empty() {
                                    "—".to_string()
                                } else {
                                    endpoint.clone()
                                })
                                .monospace()
                                .size(11.0),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if super::kit::icon_button(ui, icons::COPY, "复制端点")
                                        .clicked()
                                    {
                                        ui.ctx().copy_text(endpoint.clone());
                                        self.toast(
                                            crate::snapshot::NoticeLevel::Success,
                                            "端点已复制",
                                        );
                                    }
                                },
                            );
                        });
                    });
                let lan_endpoints: Vec<_> = self
                    .snapshot
                    .endpoints
                    .iter()
                    .filter(|endpoint| endpoint.lan)
                    .cloned()
                    .collect();
                for view in &lan_endpoints {
                    ui.add_space(3.0);
                    ui.horizontal(|ui| {
                        super::kit::badge(ui, &view.label, theme::BLUE_INFO);
                        ui.label(
                            RichText::new(&view.url)
                                .monospace()
                                .size(10.0)
                                .color(theme::text(ui)),
                        );
                        if super::kit::icon_button(ui, icons::COPY, "复制局域网端点").clicked()
                        {
                            ui.ctx().copy_text(view.url.clone());
                            self.toast(crate::snapshot::NoticeLevel::Success, "局域网端点已复制");
                        }
                    });
                }
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("三协议入口")
                            .size(10.0)
                            .color(theme::muted(ui)),
                    );
                    super::kit::plain_badge(ui, "/chat/completions");
                    super::kit::plain_badge(ui, "/messages");
                    super::kit::plain_badge(ui, "/responses");
                    super::kit::plain_badge(ui, "/models");
                });
                ui.add_space(8.0);
                ui.columns(3, |cols| {
                    super::kit::info_line(
                        &mut cols[0],
                        "运行时长",
                        &crate::config::fmt_duration_secs(self.snapshot.uptime_secs),
                    );
                    super::kit::info_line(
                        &mut cols[1],
                        "活跃请求",
                        &self.snapshot.active_requests.to_string(),
                    );
                    super::kit::info_line(
                        &mut cols[2],
                        "排队",
                        &self.snapshot.queued_requests.to_string(),
                    );
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let running = self.snapshot.proxy.is_running();
                    if running {
                        if super::kit::secondary_labeled(ui, icons::STOP, "停止").clicked() {
                            let _ = self.cmd_tx.send(crate::runtime::UiCommand::StopProxy);
                        }
                        if super::kit::ghost_labeled(ui, icons::ARROWS_CLOCKWISE, "重启").clicked()
                        {
                            let _ = self.cmd_tx.send(crate::runtime::UiCommand::RestartProxy);
                        }
                    } else if super::kit::primary_labeled(ui, icons::PLAY, "启动代理").clicked()
                    {
                        let _ = self.cmd_tx.send(crate::runtime::UiCommand::StartProxy);
                    }
                    ui.label(
                        RichText::new(format!(
                            "并发上限 {} · 队列 {}",
                            self.config.server.max_global_concurrency,
                            self.config.server.queue_capacity
                        ))
                        .size(9.5)
                        .color(theme::muted(ui)),
                    );
                });
            },
        );
    }

    fn alert_card(&mut self, ui: &mut egui::Ui) {
        let alert_count = self.snapshot.alerts.len();
        let rows: Vec<_> = self.snapshot.alerts.iter().take(5).cloned().collect();
        super::kit::section_card(
            ui,
            icons::WARNING_CIRCLE,
            "告警",
            "供应商、余额与配置检查",
            |ui| {
                if alert_count > 0 {
                    super::kit::badge(ui, &format!("{alert_count} 项"), theme::ORANGE);
                } else {
                    super::kit::status_badge(ui, "正常", theme::GREEN);
                }
            },
            |ui| {
                if rows.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(icons::CHECK_CIRCLE)
                                .size(14.0)
                                .color(theme::GREEN),
                        );
                        ui.label(
                            RichText::new("当前没有需要处理的告警。")
                                .size(11.5)
                                .color(theme::muted(ui)),
                        );
                    });
                } else {
                    for alert in &rows {
                        let color = match alert.level {
                            AlertLevel::Info => theme::BLUE_INFO,
                            AlertLevel::Warning => theme::ORANGE,
                            AlertLevel::Critical => theme::RED,
                        };
                        let variant = if alert.level == AlertLevel::Critical {
                            AlertVariant::Destructive
                        } else {
                            AlertVariant::Info
                        };
                        Alert::new(&alert.text)
                            .variant(variant)
                            .color(color)
                            .show(ui);
                    }
                    if alert_count > rows.len() {
                        ui.label(
                            RichText::new(format!("另有 {} 项见设置页", alert_count - rows.len()))
                                .size(10.0)
                                .color(theme::muted(ui)),
                        );
                    }
                }
                if self.snapshot.breaker.open {
                    ui.add_space(4.0);
                    super::kit::tooltip_text(
                        ui,
                        icons::SHIELD_WARNING,
                        &format!(
                            "全局熔断中，剩余 {} 秒",
                            (self.snapshot.breaker.open_until - now_ts()).max(0)
                        ),
                    );
                }
            },
        );
    }

    fn curl_for(&self, request: &crate::db::RecentRequest) -> String {
        let path = match request.protocol.as_str() {
            "anthropic-messages" => "/v1/messages",
            "openai-responses" => "/v1/responses",
            _ => "/v1/chat/completions",
        };
        let model = if request.client_model.is_empty() {
            "your-model"
        } else {
            &request.client_model
        };
        let body = if path == "/v1/messages" {
            format!(
                "{{\"model\":\"{model}\",\"max_tokens\":1024,\"messages\":[{{\"role\":\"user\",\"content\":\"hi\"}}]}}"
            )
        } else if path == "/v1/responses" {
            format!("{{\"model\":\"{model}\",\"input\":\"hi\"}}")
        } else {
            format!(
                "{{\"model\":\"{model}\",\"messages\":[{{\"role\":\"user\",\"content\":\"hi\"}}],\"stream\":true}}"
            )
        };
        let url = format!(
            "{}{}",
            self.snapshot.endpoint,
            path.trim_start_matches("/v1")
        );
        if path == "/v1/messages" {
            format!(
                "curl -N {url} -H 'x-api-key: yr-REPLACE_WITH_YOUR_KEY' -H 'anthropic-version: 2023-06-01' -H 'Content-Type: application/json' -d '{}'",
                body
            )
        } else {
            format!(
                "curl -N {url} -H 'Authorization: Bearer yr-REPLACE_WITH_YOUR_KEY' -H 'Content-Type: application/json' -d '{}'",
                body
            )
        }
    }
}
