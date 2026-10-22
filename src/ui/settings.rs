use std::collections::BTreeSet;

use egui::{Align, Layout, RichText, Ui, Vec2};
use egui_phosphor::regular as icons;

use crate::config::ThemeMode;
use crate::runtime::UiCommand;
use crate::snapshot::{NoticeLevel, ProxyStatus};

use super::{kit, theme, App, ConfirmAction, Page, SettingsTab};

impl App {
    pub(crate) fn page_settings(&mut self, ui: &mut egui::Ui) {
        kit::page_header(
            ui,
            Page::Settings.eyebrow(),
            "设置",
            "启动、代理、调度、安全与数据策略",
            |ui| {
                if kit::primary_button(ui, icons::FLOPPY_DISK, "保存设置").clicked() {
                    self.persist_with_notice("设置已保存");
                }
            },
        );

        let mut switch_tab = None;
        let total_width = ui.available_width();
        let tab_width = 196.0_f32.min((total_width * 0.3).max(150.0));
        let content_width = (total_width - tab_width - 14.0).max(240.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(tab_width, 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(tab_width);
                    kit::surface(ui, |ui| {
                        for tab in SettingsTab::all() {
                            if kit::nav_item(
                                ui,
                                tab.icon(),
                                tab.label(),
                                self.settings_tab == tab,
                                None,
                            )
                            .clicked()
                            {
                                switch_tab = Some(tab);
                            }
                            ui.add_space(2.0);
                        }
                    });
                },
            );
            ui.add_space(14.0);
            ui.allocate_ui_with_layout(
                Vec2::new(content_width, 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(content_width);
                    match self.settings_tab {
                        SettingsTab::General => self.settings_general(ui),
                        SettingsTab::Proxy => self.settings_proxy(ui),
                        SettingsTab::Scheduler => self.settings_scheduler(ui),
                        SettingsTab::Security => self.settings_security(ui),
                        SettingsTab::Data => self.settings_data(ui),
                        SettingsTab::About => self.settings_about(ui),
                    }
                    ui.add_space(24.0);
                },
            );
        });
        if let Some(tab) = switch_tab {
            self.settings_tab = tab;
        }
    }

    // -- General ------------------------------------------------------------

    fn balance_currencies(&self) -> Vec<String> {
        let mut currencies: BTreeSet<String> = BTreeSet::new();
        currencies.insert("USD".into());
        currencies.insert("CNY".into());
        for account in &self.config.accounts {
            currencies.insert(crate::config::normalize_currency(&account.balance.currency));
        }
        currencies.into_iter().collect()
    }

    fn settings_general(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let mut manage_keys = false;
        let mut test_key = false;
        let close_label = if cfg!(windows) {
            "关闭窗口时隐藏到托盘"
        } else {
            "关闭窗口时最小化"
        };
        let close_desc = if cfg!(windows) {
            "点击关闭按钮时把窗口和任务栏图标一起隐藏，代理继续运行；托盘菜单可重新打开或退出"
        } else {
            "当前平台会最小化到任务栏；托盘菜单仍可退出应用"
        };

        kit::section_card(
            ui,
            icons::ROCKET_LAUNCH,
            "启动与托盘",
            "登录时自动启动、启动方式与关闭行为",
            |ui| {
                kit::badge(ui, "当前用户", theme::MUTED);
            },
            |ui| {
                setting_row(
                    ui,
                    icons::POWER,
                    "开机自启",
                    "登录系统后自动启动 YoyoRelay，写入当前用户启动项，不需要管理员权限",
                    |ui| {
                        if kit::toggle_switch(ui, &mut self.config.ui.autostart).changed() {
                            changed = true;
                        }
                    },
                );
                setting_row(
                    ui,
                    icons::APP_WINDOW,
                    "启动时最小化到托盘",
                    "启动后不显示主窗口，直接驻留系统托盘，代理在后台运行；适合配合开机自启",
                    |ui| {
                        if kit::toggle_switch(ui, &mut self.config.ui.start_minimized).changed() {
                            changed = true;
                        }
                    },
                );
                setting_row(ui, icons::X, close_label, close_desc, |ui| {
                    if kit::toggle_switch(ui, &mut self.config.ui.close_to_tray).changed() {
                        changed = true;
                    }
                });
                setting_row_last(
                    ui,
                    icons::BROWSER,
                    "任务栏显示请求信息",
                    "窗口标题显示活跃请求、今日 Token 与代理状态，悬停任务栏按钮即可查看",
                    |ui| {
                        if kit::toggle_switch(ui, &mut self.config.ui.show_taskbar_stats).changed()
                        {
                            changed = true;
                        }
                    },
                );
                ui.add_space(5.0);
                kit::info_row(
                    ui,
                    icons::INFO,
                    if cfg!(windows) {
                        "托盘图标始终使用 YoyoRelay 标志：左键单击恢复主窗口，右键菜单提供「显示主窗口」与「退出」。重复启动程序会直接唤出已有窗口，不会报错。"
                    } else {
                        "托盘图标始终使用 YoyoRelay 标志，右键菜单可显示主窗口或退出。重复启动程序会直接唤出已有窗口，不会报错。"
                    },
                );
            },
        );
        ui.add_space(12.0);

        kit::section_card(
            ui,
            icons::PAINT_BRUSH,
            "外观",
            "主题、DPI 缩放与提醒",
            |ui| {
                kit::badge(ui, "即时预览", theme::ACCENT);
            },
            |ui| {
                ui.horizontal(|ui| {
                    setting_label(ui, icons::MOON, "主题");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let mut theme_mode = self.config.ui.theme;
                        if let Some(mode) = kit::segmented(
                            ui,
                            &[
                                (ThemeMode::Light, "浅色"),
                                (ThemeMode::Dark, "深色"),
                                (ThemeMode::System, "跟随系统"),
                            ],
                            theme_mode,
                        ) {
                            theme_mode = mode;
                        }
                        if theme_mode != self.config.ui.theme {
                            self.config.ui.theme = theme_mode;
                            self.last_dark = None;
                            changed = true;
                        }
                    });
                });
                setting_desc(ui, "浅色、深色或跟随系统");
                kit::divider(ui);
                ui.horizontal(|ui| {
                    setting_label(ui, icons::ARROWS_OUT, "界面缩放");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let current = if self.config.ui.ui_scale <= 0.05 {
                            0.0
                        } else if (self.config.ui.ui_scale - 1.0).abs() < 0.01 {
                            1.0
                        } else if (self.config.ui.ui_scale - 0.9).abs() < 0.01 {
                            0.9
                        } else if (self.config.ui.ui_scale - 0.8).abs() < 0.01 {
                            0.8
                        } else {
                            self.config.ui.ui_scale
                        };
                        if let Some(scale) = kit::segmented(
                            ui,
                            &[(0.0f32, "自动"), (1.0, "100%"), (0.9, "90%"), (0.8, "80%")],
                            current,
                        ) {
                            self.config.ui.ui_scale = scale;
                            self.last_zoom = 0.0;
                            changed = true;
                        }
                    });
                });
                setting_desc(ui, "自动按显示器 DPI 调整；窗口过密时可手动降低");
                kit::divider(ui);
                setting_row(
                    ui,
                    icons::BELL,
                    "应用内通知",
                    "测试结果、账号恢复与余额刷新等事件在右下角显示提示",
                    |ui| {
                        if kit::toggle_switch(ui, &mut self.config.ui.notifications).changed() {
                            changed = true;
                        }
                    },
                );
                ui.horizontal(|ui| {
                    setting_label(ui, icons::WARNING_CIRCLE, "低余额阈值与汇率");
                });
                setting_desc(
                    ui,
                    "按币种分别设置；余额合计不会把 CNY 与 USD 直接相加。汇率用于把按 USD 计价的花费折算到手动余额。",
                );
                let currencies = self.balance_currencies();
                let mut thresholds = self.config.ui.low_balance_thresholds.clone();
                let mut rates = self.config.ui.fx_rates.clone();
                let mut fx_changed = false;
                egui::Grid::new("balance_currency_settings")
                    .num_columns(3)
                    .spacing([16.0, 7.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("币种").size(10.5).color(theme::muted(ui)));
                        ui.label(
                            RichText::new("低余额阈值")
                                .size(10.5)
                                .color(theme::muted(ui)),
                        );
                        ui.label(
                            RichText::new("汇率（1 USD =）")
                                .size(10.5)
                                .color(theme::muted(ui)),
                        );
                        ui.end_row();
                        for currency in &currencies {
                            ui.label(RichText::new(currency).monospace().size(11.0));
                            let default_threshold = if currency == "CNY" { 35.0 } else { 5.0 };
                            let threshold = thresholds
                                .entry(currency.clone())
                                .or_insert(default_threshold);
                            let prefix = match currency.as_str() {
                                "CNY" => "¥ ",
                                "USD" => "$ ",
                                _ => "",
                            };
                            if ui
                                .add(
                                    egui::DragValue::new(threshold)
                                        .speed(0.1)
                                        .range(0.0..=1_000_000.0)
                                        .prefix(prefix),
                                )
                                .changed()
                            {
                                fx_changed = true;
                            }
                            if currency == "USD" {
                                ui.label(RichText::new("1.00").size(10.5).color(theme::muted(ui)));
                            } else {
                                let rate = rates
                                    .entry(currency.clone())
                                    .or_insert(if currency == "CNY" { 7.2 } else { 1.0 });
                                if ui
                                    .add(
                                        egui::DragValue::new(rate)
                                            .speed(0.01)
                                            .range(0.0001..=10_000.0)
                                            .suffix(format!(" {currency}")),
                                    )
                                    .changed()
                                {
                                    fx_changed = true;
                                }
                            }
                            ui.end_row();
                        }
                    });
                if fx_changed {
                    self.config.ui.low_balance_thresholds = thresholds;
                    self.config.ui.fx_rates = rates;
                    changed = true;
                }
                ui.horizontal(|ui| {
                    setting_label(ui, icons::ARROWS_CLOCKWISE, "余额刷新间隔");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::DragValue::new(&mut self.config.ui.balance_refresh_mins)
                                    .range(1..=1440)
                                    .suffix(" 分钟"),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    });
                });
                setting_desc(ui, "自动查询余额的频率");
            },
        );
        ui.add_space(12.0);

        kit::section_card(
            ui,
            icons::SHIELD_CHECK,
            "快捷操作",
            "常用入口与本地状态",
            |_ui| {},
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    if kit::secondary_button(ui, icons::KEY, "管理本地 Key").clicked() {
                        manage_keys = true;
                    }
                    if kit::secondary_button(ui, icons::FLASK, "测试本地 Key").clicked() {
                        test_key = true;
                    }
                    ui.label(
                        RichText::new(format!(
                            "{} 个本地 Key · 鉴权{}",
                            self.config.local_keys.len(),
                            if self.config.server.auth_enabled {
                                "已开启"
                            } else {
                                "已关闭"
                            }
                        ))
                        .size(10.5)
                        .color(theme::muted(ui)),
                    );
                });
            },
        );

        if manage_keys {
            self.page = Page::LocalKeys;
        }
        if test_key {
            if let Some(key) = self.config.local_keys.first() {
                let id = key.id.clone();
                self.start_test("local_key", &id);
            }
        }
        if changed {
            self.persist();
        }
    }

    // -- Proxy --------------------------------------------------------------

    fn settings_proxy(&mut self, ui: &mut egui::Ui) {
        let (proxy_color, proxy_label) = match &self.snapshot.proxy {
            ProxyStatus::Running => (theme::GREEN, "运行中"),
            ProxyStatus::Starting => (theme::ORANGE, "启动中"),
            ProxyStatus::Stopped => (theme::MUTED, "已停止"),
            ProxyStatus::Error(_) => (theme::RED, "异常"),
        };
        let endpoint = self.snapshot.endpoint.clone();
        let running = self.snapshot.proxy.is_running();
        let auth_enabled = self.config.server.auth_enabled;
        let mut changed = false;
        let mut start = false;
        let mut stop = false;
        let mut restart = false;
        let mut copy_endpoint = false;

        kit::section_card(
            ui,
            icons::DESKTOP_TOWER,
            "本地监听",
            "监听地址、端口、并发与超时",
            |ui| {
                kit::status_pill(ui, proxy_label, proxy_color);
            },
            |ui| {
                ui.columns(2, |cols| {
                    changed |= proxy_fields_a(&mut cols[0], &mut self.config.server);
                    changed |= proxy_fields_b(&mut cols[1], &mut self.config.server);
                });
                ui.add_space(10.0);
                egui::Frame::new()
                    .fill(theme::subtle_fill(ui))
                    .corner_radius(9)
                    .inner_margin(egui::Margin::symmetric(11, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(icons::PLUGS_CONNECTED)
                                    .size(14.0)
                                    .color(theme::ACCENT),
                            );
                            ui.label(
                                RichText::new(if endpoint.is_empty() {
                                    "—".to_string()
                                } else {
                                    endpoint.clone()
                                })
                                .monospace()
                                .size(10.5),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if kit::icon_button(ui, icons::COPY, "复制端点").clicked() {
                                    copy_endpoint = true;
                                }
                            });
                        });
                    });
                ui.label(
                    RichText::new(format!(
                        "Base URL 以 /v1 结尾；OpenAI Chat、Anthropic Messages、Responses 三种客户端都填这一地址，模型列表：{endpoint}/models"
                    ))
                    .size(9.5)
                    .color(theme::muted(ui)),
                );
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    if running {
                        if kit::secondary_button(ui, icons::STOP, "停止代理").clicked() {
                            stop = true;
                        }
                        if kit::ghost_button(ui, icons::ARROWS_CLOCKWISE, "重启代理").clicked()
                        {
                            restart = true;
                        }
                    } else if kit::primary_button(ui, icons::PLAY, "启动代理").clicked() {
                        start = true;
                    }
                    ui.label(
                        RichText::new(format!(
                            "并发上限 {} · 排队 {} · 队列超时 {}s",
                            self.config.server.max_global_concurrency,
                            self.config.server.queue_capacity,
                            self.config.server.queue_timeout_secs
                        ))
                        .size(9.5)
                        .color(theme::muted(ui)),
                    );
                });
            },
        );
        ui.add_space(12.0);

        kit::section_card(
            ui,
            icons::SHIELD_CHECK,
            "安全与跨域",
            "鉴权与浏览器访问策略",
            |ui| {
                kit::badge(
                    ui,
                    if auth_enabled {
                        "鉴权开启"
                    } else {
                        "鉴权关闭"
                    },
                    if auth_enabled {
                        theme::GREEN
                    } else {
                        theme::ORANGE
                    },
                );
            },
            |ui| {
                setting_row(
                    ui,
                    icons::KEY,
                    "要求客户端提供本地 Key",
                    "关闭后任何本机进程都可以访问本地端点，不建议在共享环境中关闭",
                    |ui| {
                        if kit::toggle_switch(ui, &mut self.config.server.auth_enabled).changed() {
                            changed = true;
                        }
                    },
                );
                setting_row_last(
                    ui,
                    icons::BROWSER,
                    "允许浏览器跨域访问（CORS）",
                    "浏览器 Web 应用直连本地代理时需要开启",
                    |ui| {
                        if kit::toggle_switch(ui, &mut self.config.server.cors).changed() {
                            changed = true;
                        }
                    },
                );
            },
        );

        if copy_endpoint {
            ui.ctx().copy_text(endpoint);
            self.toast(NoticeLevel::Success, "端点已复制");
        }
        if start {
            let _ = self.cmd_tx.send(UiCommand::StartProxy);
        }
        if stop {
            let _ = self.cmd_tx.send(UiCommand::StopProxy);
        }
        if restart {
            let _ = self.cmd_tx.send(UiCommand::RestartProxy);
        }
        if changed {
            self.persist();
        }
    }

    // -- Scheduler ----------------------------------------------------------

    fn settings_scheduler(&mut self, ui: &mut egui::Ui) {
        let breaker_open = self.snapshot.breaker.open;
        let failures = self.snapshot.breaker.consecutive_failures;
        let threshold = self.snapshot.breaker.threshold;
        let mut changed = false;
        kit::section_card(
            ui,
            icons::ACTIVITY,
            "调度与熔断",
            "失败冷却、重试机会与全局熔断保护",
            |ui| {
                kit::badge(
                    ui,
                    &format!("连续失败 {failures}/{threshold}"),
                    if breaker_open {
                        theme::RED
                    } else {
                        theme::MUTED
                    },
                );
            },
            |ui| {
                ui.columns(2, |cols| {
                    changed |= scheduler_fields_a(&mut cols[0], &mut self.config.scheduler);
                    changed |= scheduler_fields_b(&mut cols[1], &mut self.config.scheduler);
                });
                ui.add_space(8.0);
                kit::info_row(
                    ui,
                    icons::INFO,
                    "首次失败使用基础冷却，之后按 2 倍指数退避并封顶；半开探测成功即恢复。",
                );
            },
        );
        ui.add_space(12.0);

        kit::section_card(
            ui,
            icons::SHIELD_WARNING,
            "状态机说明",
            "理解调度行为有助于配置合适的额度与冷却",
            |_ui| {},
            |ui| {
                for (icon, title, desc) in [
                    (
                        icons::CHECK_CIRCLE,
                        "Healthy",
                        "正常参与调度；同供应商内按优先级与轮询顺序选择。",
                    ),
                    (
                        icons::CLOCK_COUNTER_CLOCKWISE,
                        "Cooling",
                        "429、5xx、超时或鉴权失败后进入冷却，到期后半开探测。",
                    ),
                    (
                        icons::HOURGLASS_MEDIUM,
                        "Exhausted",
                        "本地额度窗口用尽；窗口重置后自动恢复，不会误判为账号损坏。",
                    ),
                    (
                        icons::USERS,
                        "并发满",
                        "账号只是暂时移出候选集，请求完成释放槽位后立即恢复。",
                    ),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon).size(13.0).color(theme::ACCENT));
                        ui.label(RichText::new(title).size(11.5).strong());
                        ui.label(RichText::new(desc).size(10.5).color(theme::muted(ui)));
                    });
                    ui.add_space(5.0);
                }
            },
        );
        if changed {
            self.persist();
        }
    }

    // -- Security -----------------------------------------------------------

    fn settings_security(&mut self, ui: &mut egui::Ui) {
        let key_count = self.config.local_keys.len();
        let auth_enabled = self.config.server.auth_enabled;
        let mut changed = false;
        let mut create_key = false;
        let mut manage_keys = false;

        kit::section_card(
            ui,
            icons::SHIELD_CHECK,
            "访问控制",
            "本地 Key、鉴权与调试开关",
            |ui| {
                if auth_enabled {
                    kit::status_pill(ui, "鉴权开启", theme::GREEN);
                } else {
                    kit::status_pill(ui, "鉴权关闭", theme::RED);
                }
            },
            |ui| {
                setting_row(
                    ui,
                    icons::LOCK,
                    "要求客户端鉴权",
                    "支持 Authorization: Bearer 与 x-api-key；关闭后任何客户端都可访问",
                    |ui| {
                        if kit::toggle_switch(ui, &mut self.config.server.auth_enabled).changed() {
                            changed = true;
                        }
                    },
                );
                setting_row_last(
                    ui,
                    icons::BUG,
                    "调试模式",
                    "v1 仅保留开关；后续版本会记录脱敏报文并按 24 小时清理",
                    |ui| {
                        if kit::toggle_switch(ui, &mut self.config.ui.debug_mode).changed() {
                            changed = true;
                        }
                    },
                );
            },
        );
        ui.add_space(12.0);

        kit::section_card(
            ui,
            icons::KEY,
            "本地 Key 与加密",
            &format!("{key_count} 个本地 Key · AES-256-GCM 加密存储"),
            |_ui| {},
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    if kit::secondary_button(ui, icons::KEY, "管理本地 Key").clicked() {
                        manage_keys = true;
                    }
                    if kit::primary_button(ui, icons::PLUS, "新建本地 Key").clicked() {
                        create_key = true;
                    }
                    ui.label(
                        RichText::new(
                            "账号 API Key 与本地 Key 均加密落盘；界面只显示前 4 后 4 位。",
                        )
                        .size(10.5)
                        .color(theme::muted(ui)),
                    );
                });
                ui.add_space(8.0);
                kit::info_row(
                    ui,
                    icons::LOCK,
                    "导出配置不会包含任何密钥；导入后需要重新录入并重置本地 Key。",
                );
            },
        );
        if changed {
            self.persist();
        }
        if manage_keys {
            self.page = Page::LocalKeys;
        }
        if create_key {
            self.open_new_local_key();
        }
    }

    // -- Data ---------------------------------------------------------------

    fn settings_data(&mut self, ui: &mut egui::Ui) {
        let config_dir = self.store.paths.config_dir.display().to_string();
        let db_path = self.store.paths.db_file.display().to_string();
        let mut changed = false;
        let mut cleanup = false;
        let mut export = false;
        let mut import = false;
        let mut copy_config = false;
        let mut copy_db = false;

        kit::section_card(
            ui,
            icons::DATABASE,
            "本地存储",
            "配置目录、数据库与用量保留策略",
            |ui| {
                kit::badge(ui, "仅本机", theme::MUTED);
            },
            |ui| {
                ui.horizontal(|ui| {
                    setting_label(ui, icons::CALENDAR, "用量保留");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::DragValue::new(&mut self.config.ui.retention_days)
                                    .range(1..=3650)
                                    .suffix(" 天"),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    });
                });
                setting_desc(ui, "超过该天数的请求记录会定期清理");
                kit::divider(ui);
                path_row(ui, icons::FOLDER, "配置目录", &config_dir, &mut copy_config);
                path_row(ui, icons::DATABASE, "数据库", &db_path, &mut copy_db);
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    if kit::secondary_button(ui, icons::BROOM, "立即清理").clicked() {
                        cleanup = true;
                    }
                    if kit::secondary_button(ui, icons::DOWNLOAD_SIMPLE, "导出配置").clicked() {
                        export = true;
                    }
                    if kit::secondary_button(ui, icons::UPLOAD_SIMPLE, "导入配置").clicked() {
                        import = true;
                    }
                });
                if let Some(path) = &self.last_export_path {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(path)
                            .monospace()
                            .size(9.5)
                            .color(theme::muted(ui)),
                    );
                }
                if import {
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.import_path)
                                .desired_width(360.0)
                                .hint_text("导出 JSON 的绝对路径"),
                        );
                        if kit::secondary_button(ui, icons::CHECK, "确认导入").clicked() {
                            if self.import_path.trim().is_empty() {
                                self.toast(NoticeLevel::Warning, "请先填写导入文件路径。");
                            } else {
                                self.confirm = Some(ConfirmAction::ImportConfig(
                                    self.import_path.trim().to_string(),
                                ));
                            }
                        }
                    });
                    kit::info_row(
                        ui,
                        icons::INFO,
                        "导入会替换当前供应商、账号、路由与本地 Key；文件不含密钥。",
                    );
                }
            },
        );

        if copy_config {
            ui.ctx().copy_text(config_dir);
            self.toast(NoticeLevel::Success, "配置目录已复制");
        }
        if copy_db {
            ui.ctx().copy_text(db_path);
            self.toast(NoticeLevel::Success, "数据库路径已复制");
        }
        if changed {
            self.persist();
        }
        if cleanup {
            let _ = self.cmd_tx.send(UiCommand::CleanupNow);
        }
        if export {
            match self.store.export_without_secrets(&self.config) {
                Ok(path) => {
                    self.last_export_path = Some(path.display().to_string());
                    self.toast(NoticeLevel::Success, format!("已导出到 {}", path.display()));
                }
                Err(err) => self.toast(NoticeLevel::Error, format!("导出失败：{err}")),
            }
        }
    }

    // -- About --------------------------------------------------------------

    fn settings_about(&mut self, ui: &mut egui::Ui) {
        let uptime = crate::config::fmt_duration_secs(self.snapshot.uptime_secs);
        let mut restart_onboarding = false;
        let mut quit = false;
        let mut show_alerts = false;
        kit::section_card(
            ui,
            icons::INFO,
            "关于 YoyoRelay",
            &format!(
                "v{} · Rust + egui · 运行 {}",
                env!("CARGO_PKG_VERSION"),
                uptime
            ),
            |_ui| {},
            |ui| {
                ui.horizontal(|ui| {
                    kit::brand_logo(ui, &self.logo, 64.0);
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("YoyoRelay").size(20.0).strong());
                        ui.label(
                            RichText::new("本地优先的多上游 LLM 代理")
                                .size(11.0)
                                .color(theme::muted(ui)),
                        );
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            kit::badge(
                                ui,
                                &format!("{} 个供应商", self.config.providers.len()),
                                theme::MUTED,
                            );
                            kit::badge(
                                ui,
                                &format!("{} 个账号", self.config.accounts.len()),
                                theme::MUTED,
                            );
                            kit::badge(
                                ui,
                                &format!("{} 条路由", self.config.routes.len()),
                                theme::MUTED,
                            );
                            kit::badge(
                                ui,
                                &format!("{} 个模型", self.config.client_models().len()),
                                theme::MUTED,
                            );
                        });
                    });
                });
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    ui.hyperlink_to(
                        format!("{}  GitHub 仓库", icons::GITHUB_LOGO),
                        "https://github.com/ImYoyoData/yoyo-relay",
                    );
                    if kit::ghost_button(ui, icons::BOOK_OPEN, "快速开始").clicked() {
                        restart_onboarding = true;
                    }
                    if kit::ghost_button(ui, icons::WARNING_CIRCLE, "查看告警").clicked() {
                        show_alerts = true;
                    }
                    if kit::danger_button(ui, icons::POWER, "退出应用").clicked() {
                        quit = true;
                    }
                });
                if !self.snapshot.warnings.is_empty() {
                    ui.add_space(8.0);
                    kit::divider(ui);
                    for warning in self.snapshot.warnings.iter().take(8) {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(icons::WARNING)
                                    .size(12.0)
                                    .color(theme::ORANGE),
                            );
                            ui.label(RichText::new(warning).size(10.5).color(theme::ORANGE));
                        });
                    }
                }
            },
        );
        ui.add_space(12.0);
        kit::section_card(
            ui,
            icons::INFO,
            "发布与反馈",
            "版本号与更新日志来自 Cargo.toml 与 CHANGELOG.md",
            |_ui| {},
            |ui| {
                kit::info_row(
                    ui,
                    icons::GIT_BRANCH,
                    "main 分支的版本标签会自动构建 Windows / macOS / Linux 发布包（含 UPX 压缩版）。",
                );
                kit::info_row(
                    ui,
                    icons::BUG,
                    "问题反馈请附上版本号、操作系统与设置页的告警信息。",
                );
            },
        );
        if restart_onboarding {
            self.config.ui.onboarding_done = false;
            self.show_onboarding = true;
            self.persist();
        }
        if show_alerts {
            self.page = Page::Overview;
        }
        if quit {
            self.confirm = Some(ConfirmAction::Quit);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setting_label(ui: &mut Ui, icon: &str, title: &str) {
    let t = theme::tokens(ui);
    kit::icon_chip(ui, icon, t.primary, 30.0);
    ui.add_space(2.0);
    ui.label(RichText::new(title).size(12.0).strong().color(t.text));
}

fn setting_desc(ui: &mut Ui, desc: &str) {
    let t = theme::tokens(ui);
    ui.add(egui::Label::new(RichText::new(desc).size(9.5).color(t.muted)).wrap());
}

fn setting_row(ui: &mut Ui, icon: &str, title: &str, desc: &str, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        setting_label(ui, icon, title);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.set_height(30.0);
            control(ui);
        });
    });
    setting_desc(ui, desc);
    ui.add_space(4.0);
    kit::divider(ui);
}

fn setting_row_last(
    ui: &mut Ui,
    icon: &str,
    title: &str,
    desc: &str,
    control: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        setting_label(ui, icon, title);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.set_height(30.0);
            control(ui);
        });
    });
    setting_desc(ui, desc);
}

fn path_row(ui: &mut Ui, icon: &str, label: &str, value: &str, copied: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon).size(12.0).color(theme::muted(ui)));
        ui.label(RichText::new(label).size(10.0).color(theme::muted(ui)));
        ui.label(
            RichText::new(value)
                .monospace()
                .size(9.0)
                .color(theme::text(ui)),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if kit::icon_button(ui, icons::COPY, "复制路径").clicked() {
                *copied = true;
            }
        });
    });
}

fn proxy_fields_a(ui: &mut Ui, server: &mut crate::config::ServerConfig) -> bool {
    let mut changed = false;
    egui::Grid::new("proxy_a")
        .num_columns(2)
        .spacing([14.0, 9.0])
        .show(ui, |ui| {
            ui.label("监听地址");
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut server.bind)
                        .desired_width(170.0)
                        .hint_text("127.0.0.1"),
                )
                .changed();
            ui.end_row();
            ui.label("端口");
            changed |= ui
                .add(egui::DragValue::new(&mut server.port).range(1..=65535))
                .changed();
            ui.end_row();
            ui.label("全局并发上限");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut server.max_global_concurrency)
                        .range(1..=1024)
                        .suffix(" 个"),
                )
                .changed();
            ui.end_row();
            ui.label("排队容量");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut server.queue_capacity)
                        .range(0..=10000)
                        .suffix(" 个"),
                )
                .changed();
            ui.end_row();
        });
    changed
}

fn proxy_fields_b(ui: &mut Ui, server: &mut crate::config::ServerConfig) -> bool {
    let mut changed = false;
    egui::Grid::new("proxy_b")
        .num_columns(2)
        .spacing([14.0, 9.0])
        .show(ui, |ui| {
            ui.label("排队超时");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut server.queue_timeout_secs)
                        .range(1..=600)
                        .suffix(" s"),
                )
                .changed();
            ui.end_row();
            ui.label("上游响应头超时");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut server.upstream_timeout_secs)
                        .range(5..=1200)
                        .suffix(" s"),
                )
                .changed();
            ui.end_row();
            ui.label("流式空闲超时");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut server.stream_idle_timeout_secs)
                        .range(5..=1200)
                        .suffix(" s"),
                )
                .changed();
            ui.end_row();
        });
    changed
}

fn scheduler_fields_a(ui: &mut Ui, cfg: &mut crate::config::SchedulerConfig) -> bool {
    let mut changed = false;
    egui::Grid::new("sched_a")
        .num_columns(2)
        .spacing([14.0, 9.0])
        .show(ui, |ui| {
            ui.label("最大重试次数");
            changed |= ui
                .add(egui::DragValue::new(&mut cfg.max_retries).range(0..=20))
                .changed();
            ui.end_row();
            ui.label("映射级额外机会");
            changed |= ui
                .add(egui::DragValue::new(&mut cfg.fallback_retries).range(0..=10))
                .changed();
            ui.end_row();
            ui.label("基础冷却");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut cfg.cooldown_secs)
                        .range(1..=3600)
                        .suffix(" s"),
                )
                .changed();
            ui.end_row();
            ui.label("并发超限冷却");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut cfg.concurrency_cooldown_secs)
                        .range(1..=600)
                        .suffix(" s"),
                )
                .changed();
            ui.end_row();
            ui.label("鉴权失败冷却");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut cfg.auth_cooldown_secs)
                        .range(1..=86400)
                        .suffix(" s"),
                )
                .changed();
            ui.end_row();
        });
    changed
}

fn scheduler_fields_b(ui: &mut Ui, cfg: &mut crate::config::SchedulerConfig) -> bool {
    let mut changed = false;
    egui::Grid::new("sched_b")
        .num_columns(2)
        .spacing([14.0, 9.0])
        .show(ui, |ui| {
            ui.label("冷却封顶");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut cfg.max_cooldown_secs)
                        .range(1..=86400)
                        .suffix(" s"),
                )
                .changed();
            ui.end_row();
            ui.label("熔断阈值");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut cfg.breaker_threshold)
                        .range(1..=1000)
                        .suffix(" 次"),
                )
                .changed();
            ui.end_row();
            ui.label("熔断冷却");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut cfg.breaker_cooldown_secs)
                        .range(1..=600)
                        .suffix(" s"),
                )
                .changed();
            ui.end_row();
            ui.label("半开探测间隔");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut cfg.half_open_probe_secs)
                        .range(1..=300)
                        .suffix(" s"),
                )
                .changed();
            ui.end_row();
            ui.label("额度缓存");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut cfg.quota_cache_secs)
                        .range(1..=300)
                        .suffix(" s"),
                )
                .changed();
            ui.end_row();
        });
    changed
}
