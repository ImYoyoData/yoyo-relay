use egui::{RichText, Vec2};
use egui_phosphor::regular as icons;

use crate::config::now_ts;
use crate::snapshot::{AlertLevel, ProxyStatus};

use super::{theme, widgets, App};

fn inspection_row(
    ui: &mut egui::Ui,
    dot: egui::Color32,
    label: &str,
    value: &str,
    value_color: egui::Color32,
    pill: bool,
) {
    let t = theme::tokens(ui);
    egui::Frame::new()
        .fill(if t.dark_mode {
            t.hover
        } else {
            egui::Color32::from_rgb(248, 250, 253)
        })
        .stroke(egui::Stroke::new(1.0_f32, t.card_border))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(11, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(7.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 3.5, dot);
                ui.add_space(2.0);
                ui.label(RichText::new(label).size(10.8).color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if pill {
                        super::kit::status_badge(ui, value, value_color);
                    } else {
                        ui.label(RichText::new(value).size(10.5).strong().color(value_color));
                    }
                });
            });
        });
    ui.add_space(5.0);
}

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

        let snapshot = self.snapshot.clone();
        if super::kit::kpi_cards_from_snapshot(ui, &snapshot, &mut self.config.ui) {
            self.persist();
        }
        ui.add_space(4.0);

        if ui.available_width() < 900.0 {
            self.proxy_card(ui);
            ui.add_space(10.0);
            self.alert_card(ui);
        } else {
            let total = ui.available_width();
            let gap = 12.0;
            let left = (total * 0.58).max(430.0);
            let right = (total - left - gap).max(320.0);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(left, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(left);
                        self.proxy_card(ui);
                    },
                );
                ui.add_space(gap);
                ui.allocate_ui_with_layout(
                    Vec2::new(right, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(right);
                        self.alert_card(ui);
                    },
                );
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
                let currency = self.config.ui.active_currency();
                super::kit::interactive_chart(
                    ui,
                    &points,
                    theme::ACCENT,
                    170.0,
                    super::kit::ChartMetric::Tokens,
                    3600,
                    &currency,
                    self.config.ui.active_fx_rate(),
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
                    if super::kit::empty_state(
                        ui,
                        icons::PAPER_PLANE_TILT,
                        "还没有请求",
                        "把客户端指向本地端点并发起一次请求即可在这里看到。",
                        icons::KEY,
                        "查看本地 Key",
                    ) {
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
                                    RichText::new(widgets::format_call_time(request.ts))
                                        .monospace()
                                        .size(9.0)
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
                                        let duration = if request.first_token_ms > 0 {
                                            format!(
                                                "首字 {} · 总 {}",
                                                widgets::format_duration_ms(request.first_token_ms),
                                                widgets::format_duration_ms(request.latency_ms)
                                            )
                                        } else {
                                            format!(
                                                "总 {}",
                                                widgets::format_duration_ms(request.latency_ms)
                                            )
                                        };
                                        ui.label(
                                            RichText::new(duration)
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
                                            RichText::new(widgets::format_cost_with(
                                                request.cost_micros,
                                                &self.config.ui,
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
        let (proxy_color, proxy_label) = match &self.snapshot.proxy {
            ProxyStatus::Running => (theme::GREEN, "运行中"),
            ProxyStatus::Starting => (theme::ORANGE, "启动中"),
            ProxyStatus::Stopped => (theme::MUTED, "已停止"),
            ProxyStatus::Error(_) => (theme::RED, "异常"),
        };
        let port = self.config.server.port;
        let lan_enabled = self.snapshot.lan_enabled;
        let endpoint = self.snapshot.endpoint.clone();
        let lan_endpoints: Vec<_> = self
            .snapshot
            .endpoints
            .iter()
            .filter(|endpoint| endpoint.lan)
            .cloned()
            .collect();
        let uptime = crate::config::fmt_duration_secs(self.snapshot.uptime_secs);
        let active = self.snapshot.active_requests.to_string();
        let queued = self.snapshot.queued_requests.to_string();
        let max_concurrency = self.config.server.max_global_concurrency.to_string();
        let queue_capacity = self.config.server.queue_capacity.to_string();
        let mut stop = false;
        let mut restart = false;
        let mut start = false;
        super::kit::section_card(
            ui,
            icons::HARD_DRIVES,
            "代理服务与本地端点",
            "本地端点分流、请求映射与运行控制器",
            |ui| {
                super::kit::status_badge(ui, &format!("{proxy_label} (Port: {port})"), proxy_color);
                if lan_enabled {
                    super::kit::badge(ui, "局域网", theme::BLUE_INFO);
                }
            },
            |ui| {
                let t = theme::tokens(ui);
                if let ProxyStatus::Error(err) = &self.snapshot.proxy {
                    egui::Frame::new()
                        .fill(theme::soft_for(ui, theme::RED))
                        .stroke(egui::Stroke::new(1.0_f32, theme::RED.gamma_multiply(0.28)))
                        .corner_radius(10)
                        .inner_margin(egui::Margin::symmetric(11, 9))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal_top(|ui| {
                                ui.label(
                                    RichText::new(icons::WARNING_OCTAGON)
                                        .size(14.0)
                                        .color(theme::RED),
                                );
                                ui.add(
                                    egui::Label::new(RichText::new(err).size(10.8).color(t.text))
                                        .wrap(),
                                );
                            });
                        });
                    ui.add_space(8.0);
                }

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("基础请求端点 (Base URL)")
                            .size(10.5)
                            .strong()
                            .color(t.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("兼容 OpenAI / Claude API 规范")
                                .size(9.5)
                                .color(t.muted),
                        );
                    });
                });
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(if t.dark_mode {
                        t.hover
                    } else {
                        egui::Color32::from_rgb(248, 250, 253)
                    })
                    .stroke(egui::Stroke::new(1.0_f32, t.card_border))
                    .corner_radius(12)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(icons::LIGHTNING)
                                    .size(16.0)
                                    .color(theme::ACCENT),
                            );
                            ui.label(
                                RichText::new(if endpoint.is_empty() {
                                    "—"
                                } else {
                                    endpoint.as_str()
                                })
                                .monospace()
                                .size(12.5)
                                .strong(),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if super::kit::secondary_button(ui, icons::COPY, "复制")
                                        .clicked()
                                        && !endpoint.is_empty()
                                    {
                                        ui.ctx().copy_text(endpoint.clone());
                                        self.toast(
                                            crate::snapshot::NoticeLevel::Success,
                                            "端点已复制",
                                        );
                                    }
                                    if super::kit::secondary_button(ui, "", "cURL 示例")
                                        .clicked()
                                        && !endpoint.is_empty()
                                    {
                                        let curl = format!(
                                            "curl -N {endpoint}/chat/completions -H 'Authorization: Bearer yr-REPLACE_WITH_YOUR_KEY' -H 'Content-Type: application/json' -d '{{\"model\":\"your-model\",\"messages\":[{{\"role\":\"user\",\"content\":\"hi\"}}],\"stream\":true}}'"
                                        );
                                        ui.ctx().copy_text(curl);
                                        self.toast(
                                            crate::snapshot::NoticeLevel::Success,
                                            "cURL 示例已复制",
                                        );
                                    }
                                },
                            );
                        });
                    });
                if !lan_endpoints.is_empty() {
                    ui.add_space(5.0);
                    for view in &lan_endpoints {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("局域网").size(9.5).color(t.muted));
                            ui.label(RichText::new(&view.url).monospace().size(9.5).color(t.text));
                            if super::kit::icon_button(ui, icons::COPY, "复制局域网端点").clicked()
                            {
                                ui.ctx().copy_text(view.url.clone());
                                self.toast(
                                    crate::snapshot::NoticeLevel::Success,
                                    "局域网端点已复制",
                                );
                            }
                        });
                    }
                }

                ui.add_space(9.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("标准协议入口快捷测试与映射:")
                            .size(10.0)
                            .color(t.muted),
                    );
                    for path in [
                        "/chat/completions",
                        "/messages",
                        "/responses",
                        "/models",
                        "/embeddings",
                    ] {
                        if super::kit::path_pill(ui, path)
                            .on_hover_text("点击复制完整入口")
                            .clicked()
                            && !endpoint.is_empty()
                        {
                            ui.ctx().copy_text(format!("{endpoint}{path}"));
                            self.toast(
                                crate::snapshot::NoticeLevel::Success,
                                format!("已复制 {path}"),
                            );
                        }
                    }
                });

                ui.add_space(10.0);
                ui.columns(5, |cols| {
                    super::kit::mini_stat(&mut cols[0], "持续时长", &uptime);
                    super::kit::mini_stat(&mut cols[1], "活跃请求", &active);
                    super::kit::mini_stat(&mut cols[2], "排队中", &queued);
                    super::kit::mini_stat(&mut cols[3], "并发上限", &max_concurrency);
                    super::kit::mini_stat(&mut cols[4], "队列容量", &queue_capacity);
                });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let running = self.snapshot.proxy.is_running();
                    if running {
                        if super::kit::danger_button(ui, icons::STOP, "停止代理").clicked() {
                            stop = true;
                        }
                        if super::kit::secondary_button(ui, icons::ARROWS_CLOCKWISE, "重启网关")
                            .clicked()
                        {
                            restart = true;
                        }
                    } else if super::kit::primary_button(ui, icons::PLAY, "启动代理").clicked()
                    {
                        start = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("并发保护策略：动态自适应排队")
                                .size(9.5)
                                .color(t.muted),
                        );
                    });
                });
            },
        );
        if stop {
            let _ = self.cmd_tx.send(crate::runtime::UiCommand::StopProxy);
        }
        if restart {
            let _ = self.cmd_tx.send(crate::runtime::UiCommand::RestartProxy);
        }
        if start {
            let _ = self.cmd_tx.send(crate::runtime::UiCommand::StartProxy);
        }
    }

    fn alert_card(&mut self, ui: &mut egui::Ui) {
        let alert_count = self.snapshot.alerts.len();
        let rows: Vec<_> = self.snapshot.alerts.iter().take(1).cloned().collect();
        let healthy: usize = self
            .snapshot
            .providers
            .iter()
            .map(|provider| provider.healthy + provider.half_open)
            .sum();
        let total: usize = self
            .snapshot
            .providers
            .iter()
            .map(|provider| provider.account_count)
            .sum();
        let low_balances = self
            .snapshot
            .balances
            .iter()
            .filter(|balance| {
                balance
                    .amount
                    .is_some_and(|amount| amount < self.config.ui.threshold_for(&balance.currency))
            })
            .count();
        let breaker_open = self.snapshot.breaker.open;
        let breaker_secs = (self.snapshot.breaker.open_until - now_ts()).max(0);
        let last_diagnostic = self.last_diagnostic_at.map(|at| at.elapsed().as_secs());
        let provider_ids: Vec<String> = self
            .snapshot
            .providers
            .iter()
            .map(|provider| provider.id.clone())
            .collect();
        let mut diagnose = false;
        let t = theme::tokens(ui);
        super::kit::section_card(
            ui,
            icons::WARNING_CIRCLE,
            "系统告警与巡检",
            "供应商连通性、余额健康度与配置检查",
            |ui| {
                if alert_count > 0 {
                    super::kit::status_badge(ui, &format!("{alert_count} 项告警"), theme::ORANGE);
                } else {
                    super::kit::status_badge(ui, "正常 (0 告警)", theme::GREEN);
                }
            },
            |ui| {
                if rows.is_empty() {
                    egui::Frame::new()
                        .fill(theme::soft_for(ui, theme::GREEN))
                        .stroke(egui::Stroke::new(
                            1.0_f32,
                            theme::GREEN.gamma_multiply(0.25),
                        ))
                        .corner_radius(12)
                        .inner_margin(egui::Margin::symmetric(12, 10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal_top(|ui| {
                                ui.label(
                                    RichText::new(icons::CHECK_CIRCLE)
                                        .size(16.0)
                                        .color(theme::GREEN),
                                );
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new("当前没有需要处理的告警")
                                            .size(11.5)
                                            .strong()
                                            .color(t.text),
                                    );
                                    ui.label(
                                        RichText::new(
                                            "已检测所有上游 API 节点，均能顺畅握手响应。",
                                        )
                                        .size(9.5)
                                        .color(t.muted),
                                    );
                                });
                            });
                        });
                } else if let Some(alert) = rows.first() {
                    let (color, icon) = match alert.level {
                        AlertLevel::Info => (theme::BLUE_INFO, icons::INFO),
                        AlertLevel::Warning => (theme::ORANGE, icons::WARNING),
                        AlertLevel::Critical => (theme::RED, icons::WARNING_OCTAGON),
                    };
                    let inner = ui
                        .scope(|ui| {
                            egui::Frame::new()
                                .fill(theme::soft_for(ui, color))
                                .stroke(egui::Stroke::new(1.0_f32, color.gamma_multiply(0.28)))
                                .corner_radius(10)
                                .inner_margin(egui::Margin::symmetric(11, 9))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.set_min_height(34.0);
                                    ui.horizontal_top(|ui| {
                                        ui.label(RichText::new(icon).size(14.0).color(color));
                                        ui.add_space(2.0);
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&alert.text).size(10.8).color(t.text),
                                            )
                                            .wrap(),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Min),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(icons::ARROW_RIGHT)
                                                        .size(11.0)
                                                        .color(color),
                                                );
                                            },
                                        );
                                    });
                                });
                        })
                        .response;
                    let click = ui.interact(
                        inner.rect,
                        egui::Id::new("overview_latest_alert"),
                        egui::Sense::click(),
                    );
                    if click.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if click.clicked() {
                        self.alerts_open = true;
                    }
                    ui.add_space(5.0);
                    if alert_count > 1 {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("共 {alert_count} 项告警"))
                                    .size(9.5)
                                    .color(t.muted),
                            );
                            if super::kit::ghost_labeled(ui, icons::ARROW_RIGHT, "查看全部")
                                .clicked()
                            {
                                self.alerts_open = true;
                            }
                        });
                    }
                }

                ui.add_space(6.0);
                inspection_row(
                    ui,
                    theme::GREEN,
                    "上游供应商心跳检测",
                    &if total == 0 {
                        "未配置".to_string()
                    } else {
                        format!("{healthy}/{total} 可用")
                    },
                    if total == 0 {
                        t.muted
                    } else if healthy == total {
                        theme::GREEN
                    } else if healthy > 0 {
                        theme::ORANGE
                    } else {
                        theme::RED
                    },
                    false,
                );
                let balance_text = if low_balances == 0 {
                    "充足".to_string()
                } else {
                    format!("{low_balances} 个低余额")
                };
                inspection_row(
                    ui,
                    if low_balances == 0 {
                        theme::GREEN
                    } else {
                        theme::ORANGE
                    },
                    "账号余额与配额状态",
                    &balance_text,
                    if low_balances == 0 {
                        theme::GREEN
                    } else {
                        theme::ORANGE
                    },
                    true,
                );
                let breaker_text = if breaker_open {
                    format!("熔断中 · 剩余 {breaker_secs} 秒")
                } else {
                    "待命 · 0 熔断".to_string()
                };
                inspection_row(
                    ui,
                    if breaker_open {
                        theme::RED
                    } else {
                        theme::BLUE_INFO
                    },
                    "动态熔断器 (Circuit Breaker)",
                    &breaker_text,
                    if breaker_open {
                        theme::RED
                    } else {
                        theme::BLUE_INFO
                    },
                    false,
                );

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let text = match last_diagnostic {
                        Some(secs) => format!("最近巡检: {secs} 秒前"),
                        None => "最近巡检: 尚未执行".to_string(),
                    };
                    ui.label(RichText::new(text).size(9.5).color(t.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if super::kit::ghost_labeled(ui, icons::ARROW_RIGHT, "立即执行诊断")
                            .clicked()
                            && !provider_ids.is_empty()
                        {
                            diagnose = true;
                        }
                    });
                });
            },
        );
        if diagnose {
            for provider_id in &provider_ids {
                let _ = self
                    .cmd_tx
                    .send(crate::runtime::UiCommand::TestProvider(provider_id.clone()));
            }
            self.last_diagnostic_at = Some(std::time::Instant::now());
            self.toast(
                crate::snapshot::NoticeLevel::Info,
                format!("已开始诊断 {} 个供应商", provider_ids.len()),
            );
        }
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
