use std::collections::BTreeSet;

use egui::{Align, Layout, RichText, Ui, Vec2};
use egui_phosphor::regular as icons;

use crate::config::ThemeMode;
use crate::runtime::UiCommand;
use crate::snapshot::{NoticeLevel, ProxyStatus};

use super::{kit, theme, widgets, App, ConfirmAction, Page, SettingsTab};

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
                        SettingsTab::Taskbar => self.settings_taskbar(ui),
                        SettingsTab::Proxy => self.settings_proxy(ui),
                        SettingsTab::Scheduler => self.settings_scheduler(ui),
                        SettingsTab::Pricing => self.settings_pricing(ui),
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
        let mut refresh_fx = false;
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
        let taskbar_label = if cfg!(windows) {
            "任务栏实时徽标"
        } else {
            "任务栏显示请求信息"
        };
        let taskbar_desc = if cfg!(windows) {
            "在 Windows 任务栏通知区域左侧常驻显示实时状态；显示内容、尺寸与位置可在「任务栏徽标」页调整"
        } else {
            "窗口标题与托盘提示显示活跃请求、今日 Token 与代理状态"
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
                setting_row_last(ui, icons::BROWSER, taskbar_label, taskbar_desc, |ui| {
                    if kit::toggle_switch(ui, &mut self.config.ui.show_taskbar_stats).changed() {
                        changed = true;
                    }
                });
                ui.add_space(5.0);
                kit::info_row(
                    ui,
                    icons::INFO,
                    if cfg!(windows) {
                        "任务栏徽标按系统主题自动切换深浅色，鼠标点击穿透；托盘图标使用 YoyoRelay 标志，左键恢复窗口，右键菜单可显示或退出。"
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
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if kit::secondary_button(ui, icons::ARROWS_CLOCKWISE, "立即获取汇率")
                            .clicked()
                        {
                            refresh_fx = true;
                        }
                        if self.snapshot.fx_refreshing {
                            ui.add(egui::Spinner::new().size(14.0));
                        }
                    });
                });
                setting_desc(
                    ui,
                    "按币种分别设置低余额阈值；汇率以 1 USD 为基准，用于手动余额折算与余额页的 CNY 合计。",
                );
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(theme::subtle_fill(ui))
                    .corner_radius(9)
                    .inner_margin(egui::Margin::symmetric(11, 8))
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if kit::toggle_switch(ui, &mut self.config.ui.fx_auto).changed() {
                                changed = true;
                            }
                            ui.label(
                                RichText::new("自动获取实时汇率")
                                    .size(11.0)
                                    .strong()
                                    .color(theme::text(ui)),
                            );
                            ui.label(RichText::new("每").size(10.0).color(theme::muted(ui)));
                            if ui
                                .add(
                                    egui::DragValue::new(&mut self.config.ui.fx_refresh_hours)
                                        .range(1..=168)
                                        .suffix(" 小时"),
                                )
                                .changed()
                            {
                                changed = true;
                            }
                            ui.label(RichText::new("刷新一次").size(10.0).color(theme::muted(ui)));
                        });
                        ui.add_space(2.0);
                        let source = if self.config.ui.fx_source.is_empty() {
                            "尚未获取".to_string()
                        } else {
                            self.config.ui.fx_source.clone()
                        };
                        let when = if self.config.ui.fx_updated_at > 0 {
                            widgets::format_time(self.config.ui.fx_updated_at)
                        } else {
                            "—".to_string()
                        };
                        ui.label(
                            RichText::new(format!(
                                "数据源：{source} · 最近更新：{when} · 1 USD = {:.2} CNY",
                                self.config.ui.fx_rate("CNY")
                            ))
                            .size(9.5)
                            .color(theme::muted(ui)),
                        );
                    });
                ui.add_space(6.0);
                let currencies = self.balance_currencies();
                let mut thresholds = self.config.ui.low_balance_thresholds.clone();
                let mut rates = self.config.ui.fx_rates.clone();
                let mut threshold_changed = false;
                let mut rate_changed = false;
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
                                threshold_changed = true;
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
                                    rate_changed = true;
                                }
                            }
                            ui.end_row();
                        }
                    });
                if threshold_changed {
                    self.config.ui.low_balance_thresholds = thresholds;
                    changed = true;
                }
                if rate_changed {
                    self.config.ui.fx_rates = rates;
                    self.config.ui.fx_updated_at = crate::config::now_ts();
                    self.config.ui.fx_source = "手动填写".into();
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
        if refresh_fx {
            let _ = self.cmd_tx.send(UiCommand::RefreshFxRates { manual: true });
        }
        if changed {
            self.persist();
        }
    }

    // -- Taskbar badge ------------------------------------------------------

    fn settings_taskbar(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let mut reset = false;
        let windows = cfg!(windows);
        kit::section_card(
            ui,
            icons::MONITOR,
            "任务栏实时徽标",
            if windows {
                "常驻任务栏通知区域左侧的实时状态胶囊"
            } else {
                "当前平台不支持任务栏原生徽标；窗口标题与托盘提示仍会显示状态"
            },
            |ui| {
                kit::badge(
                    ui,
                    if windows {
                        "实时预览"
                    } else {
                        "仅 Windows"
                    },
                    if windows { theme::GREEN } else { theme::MUTED },
                );
            },
            |ui| {
                setting_row(
                    ui,
                    icons::BROWSER,
                    "启用任务栏徽标",
                    "关闭后徽标立即从任务栏消失，代理继续在后台运行",
                    |ui| {
                        if kit::toggle_switch(ui, &mut self.config.ui.show_taskbar_stats).changed()
                        {
                            changed = true;
                        }
                    },
                );
                let taskbar = &mut self.config.ui.taskbar;
                kit::section_label(ui, "显示内容");
                ui.add_space(3.0);
                egui::Grid::new("taskbar_content")
                    .num_columns(3)
                    .spacing([18.0, 8.0])
                    .show(ui, |ui| {
                        for (index, (label, value)) in [
                            ("状态圆点", &mut taskbar.show_dot),
                            ("代理状态文字", &mut taskbar.show_status),
                            ("活跃请求", &mut taskbar.show_active),
                            ("今日 Token", &mut taskbar.show_tokens),
                            ("今日请求", &mut taskbar.show_requests),
                            ("排队数量", &mut taskbar.show_queued),
                            ("成功率", &mut taskbar.show_success),
                            ("预估花费", &mut taskbar.show_cost),
                            ("实时 tok/s", &mut taskbar.show_speed),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            if kit::toggle_switch(ui, value).changed() {
                                changed = true;
                            }
                            ui.label(RichText::new(label).size(11.0).color(theme::text(ui)));
                            if index % 3 == 2 {
                                ui.end_row();
                            }
                        }
                    });
                ui.add_space(3.0);
                ui.horizontal_wrapped(|ui| {
                    if kit::toggle_switch(ui, &mut self.config.ui.taskbar.transparent_bg).changed()
                    {
                        changed = true;
                    }
                    ui.label(
                        RichText::new("透明底色")
                            .size(11.0)
                            .strong()
                            .color(theme::text(ui)),
                    );
                    ui.label(
                        RichText::new("只绘制文字与状态点，不显示胶囊背景，适配深色或彩色任务栏")
                            .size(10.0)
                            .color(theme::muted(ui)),
                    );
                });
                ui.add_space(3.0);
                ui.horizontal_wrapped(|ui| {
                    if kit::toggle_switch(ui, &mut self.config.ui.taskbar.custom_text_color)
                        .changed()
                    {
                        changed = true;
                    }
                    ui.label(
                        RichText::new("自定义文字颜色")
                            .size(11.0)
                            .strong()
                            .color(theme::text(ui)),
                    );
                    let mut rgb = parse_hex_color(&self.config.ui.taskbar.text_color)
                        .map(|(r, g, b)| [r, g, b])
                        .unwrap_or([17, 24, 39]);
                    let color_response = ui
                        .add_enabled_ui(self.config.ui.taskbar.custom_text_color, |ui| {
                            egui::color_picker::color_edit_button_srgb(ui, &mut rgb)
                        })
                        .inner;
                    if color_response.changed() {
                        self.config.ui.taskbar.text_color =
                            format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
                        changed = true;
                    }
                    ui.label(
                        RichText::new(if self.config.ui.taskbar.custom_text_color {
                            "关闭后按系统主题与任务栏底色自动选择对比色"
                        } else {
                            "当前使用自动对比色"
                        })
                        .size(10.0)
                        .color(theme::muted(ui)),
                    );
                });
                ui.add_space(4.0);
                kit::divider(ui);
                kit::section_label(ui, "尺寸与位置");
                ui.add_space(3.0);
                ui.spacing_mut().slider_width = 190.0;
                egui::Grid::new("taskbar_geometry")
                    .num_columns(2)
                    .spacing([18.0, 9.0])
                    .show(ui, |ui| {
                        ui.label("整体缩放");
                        if ui
                            .add(
                                egui::Slider::new(&mut self.config.ui.taskbar.scale, 0.6..=2.2)
                                    .fixed_decimals(2)
                                    .suffix("×"),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();
                        ui.label("水平偏移");
                        if ui
                            .add(
                                egui::Slider::new(&mut self.config.ui.taskbar.offset_x, -300..=300)
                                    .suffix(" px"),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();
                        ui.label("垂直偏移");
                        if ui
                            .add(
                                egui::Slider::new(&mut self.config.ui.taskbar.offset_y, -40..=40)
                                    .suffix(" px"),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();
                        ui.label("与通知区域间距");
                        if ui
                            .add(
                                egui::Slider::new(&mut self.config.ui.taskbar.gap, 0..=60)
                                    .suffix(" px"),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();
                        ui.label("不透明度");
                        if ui
                            .add(
                                egui::Slider::new(&mut self.config.ui.taskbar.opacity, 0.3..=1.0)
                                    .fixed_decimals(2),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                        ui.end_row();
                    });
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if kit::secondary_button(ui, icons::ARROW_COUNTER_CLOCKWISE, "恢复默认")
                        .clicked()
                    {
                        reset = true;
                    }
                    ui.label(
                        RichText::new(
                            "滑块与开关会立即应用到任务栏，无需重启。位置偏移以默认锚点为基准。",
                        )
                        .size(9.5)
                        .color(theme::muted(ui)),
                    );
                });
            },
        );
        if reset {
            self.config.ui.taskbar = crate::config::TaskbarBadgeConfig::default();
            changed = true;
        }
        if changed {
            self.persist();
        }
    }

    // -- Pricing ------------------------------------------------------------

    fn settings_pricing(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let mut remove: Option<usize> = None;
        let mut export_plans = false;
        let mut import_plans = false;
        let plan_count = self.config.price_plans.len();

        kit::section_card(
            ui,
            icons::TAG,
            "价格方案库",
            "可复用的价格表：一次配置，多个上游一键套用，也便于导出分享",
            |ui| {
                kit::badge(ui, &format!("{plan_count} 套"), theme::ACCENT);
            },
            |ui| {
                if self.config.price_plans.is_empty() {
                    kit::tooltip_text(
                        ui,
                        icons::INFO,
                        "还没有价格方案。在任一上游的「计费价格」里配置好后点「存为方案」即可。",
                    );
                }
                for (index, plan) in self.config.price_plans.iter_mut().enumerate() {
                    egui::Frame::new()
                        .fill(theme::subtle_fill(ui))
                        .corner_radius(10)
                        .inner_margin(egui::Margin::symmetric(12, 10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    if ui
                                        .add(
                                            egui::TextEdit::singleline(&mut plan.name)
                                                .id(egui::Id::new((
                                                    "price_plan_library_name",
                                                    index,
                                                )))
                                                .desired_width(220.0)
                                                .hint_text("方案名称"),
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                    let periods = if plan.periods.is_empty() {
                                        "全天默认价".to_string()
                                    } else {
                                        plan.periods
                                            .iter()
                                            .map(|period| {
                                                format!(
                                                    "{} {}",
                                                    period.name,
                                                    period.schedule_label()
                                                )
                                            })
                                            .collect::<Vec<_>>()
                                            .join(" · ")
                                    };
                                    if !plan.note.trim().is_empty() {
                                        ui.label(
                                            RichText::new(&plan.note)
                                                .size(9.5)
                                                .color(theme::muted(ui)),
                                        );
                                    }
                                    ui.label(
                                        RichText::new(format!(
                                            "{} · 输入 {} · 命中 {} · 输出 {} · {}",
                                            plan.currency.code(),
                                            fmt_price(plan.base.input_per_mtok),
                                            fmt_price(plan.base.cached_input_per_mtok),
                                            fmt_price(plan.base.output_per_mtok),
                                            periods
                                        ))
                                        .size(10.0)
                                        .color(theme::muted(ui)),
                                    );
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if kit::danger_icon_button(
                                        ui,
                                        icons::TRASH_SIMPLE,
                                        "删除价格方案",
                                    )
                                    .clicked()
                                    {
                                        remove = Some(index);
                                    }
                                });
                            });
                        });
                    ui.add_space(5.0);
                }
            },
        );
        ui.add_space(12.0);

        kit::section_card(
            ui,
            icons::ARROWS_LEFT_RIGHT,
            "导入与导出",
            "价格方案单独存储为 JSON，可复制到其他机器或团队共享",
            |_ui| {},
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    if kit::secondary_button(ui, icons::DOWNLOAD_SIMPLE, "导出价格方案").clicked()
                    {
                        export_plans = true;
                    }
                    if kit::secondary_button(ui, icons::UPLOAD_SIMPLE, "导入价格方案").clicked()
                    {
                        import_plans = true;
                    }
                });
                if import_plans {
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.price_plan_import_path)
                                .desired_width(380.0)
                                .hint_text("价格方案 JSON 的绝对路径"),
                        );
                        if kit::secondary_button(ui, icons::CHECK, "确认导入").clicked() {
                            let path = self.price_plan_import_path.trim().to_string();
                            if path.is_empty() {
                                self.toast(NoticeLevel::Warning, "请先填写导入文件路径。");
                            } else {
                                match self.store.import_price_plans(std::path::Path::new(&path)) {
                                    Ok(plans) => {
                                        let added = self.merge_price_plans(plans);
                                        if self.persist() {
                                            self.toast(
                                                NoticeLevel::Success,
                                                format!("已导入 {added} 套价格方案"),
                                            );
                                        }
                                    }
                                    Err(err) => {
                                        self.toast(NoticeLevel::Error, format!("导入失败：{err}"))
                                    }
                                }
                            }
                        }
                    });
                    ui.add_space(4.0);
                    kit::info_row(
                        ui,
                        icons::INFO,
                        "导入会合并同名方案：文件中的同名方案覆盖当前值，其余追加。",
                    );
                }
                ui.add_space(4.0);
                kit::info_row(
                    ui,
                    icons::INFO,
                    "价格方案只包含价格表，不含任何账号、Key 或上游地址。",
                );
            },
        );

        if let Some(index) = remove {
            self.config.price_plans.remove(index);
            changed = true;
        }
        if export_plans {
            match self.store.export_price_plans(&self.config.price_plans) {
                Ok(path) => self.toast(
                    NoticeLevel::Success,
                    format!("已导出价格方案到 {}", path.display()),
                ),
                Err(err) => self.toast(NoticeLevel::Error, format!("导出失败：{err}")),
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
                let lan = matches!(
                    self.config.server.bind.trim(),
                    "0.0.0.0" | "::" | "0:0:0:0:0:0:0:0"
                );
                let mut allow_lan = lan;
                let mut lan_changed = false;
                ui.horizontal_wrapped(|ui| {
                    if kit::toggle_row(ui, &mut allow_lan, "允许局域网访问").changed() {
                        lan_changed = true;
                    }
                    ui.label(
                        RichText::new(if allow_lan {
                            "监听 0.0.0.0，局域网内其他设备可用下方地址访问"
                        } else {
                            "仅监听 127.0.0.1，只有本机可访问"
                        })
                        .size(10.0)
                        .color(theme::muted(ui)),
                    );
                });
                if lan_changed {
                    self.config.server.bind = if allow_lan {
                        "0.0.0.0".to_string()
                    } else {
                        "127.0.0.1".to_string()
                    };
                    // The bind TextEdit may still hold focus with its old
                    // buffer; drop it so a pending edit cannot write 127.0.0.1
                    // back over the switch in the same frame.
                    ui.memory_mut(|memory| memory.surrender_focus(proxy_bind_id()));
                    changed = true;
                }
                if allow_lan && !self.config.server.auth_enabled {
                    ui.add_space(3.0);
                    egui::Frame::new()
                        .fill(theme::soft_for(ui, theme::RED))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::symmetric(10, 7))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{} 局域网监听已开启，但「安全」页的本地鉴权是关闭的。请先启用鉴权，否则同网段设备可直接调用。",
                                    icons::WARNING
                                ))
                                .size(10.0)
                                .color(theme::RED),
                            );
                        });
                }
                ui.add_space(8.0);
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
                if self.snapshot.endpoints.len() > 1 {
                    ui.add_space(5.0);
                    for view in self
                        .snapshot
                        .endpoints
                        .iter()
                        .filter(|endpoint| endpoint.lan)
                    {
                        ui.horizontal(|ui| {
                            kit::badge(ui, &view.label, theme::BLUE_INFO);
                            ui.label(RichText::new(&view.url).monospace().size(10.0));
                            if kit::icon_button(ui, icons::COPY, "复制该地址").clicked() {
                                ui.ctx().copy_text(view.url.clone());
                            }
                        });
                    }
                }
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
                setting_row(
                    ui,
                    icons::GLOBE,
                    "系统代理失败时直连重试",
                    "上游请求经系统代理连接失败或超时后，自动改用直连客户端重试同一账号一次",
                    |ui| {
                        if kit::toggle_switch(ui, &mut self.config.server.direct_fallback).changed()
                        {
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
                changed |= scheduler_slow_fields(ui, &mut self.config.scheduler);
                ui.add_space(6.0);
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

    fn contributors_card(&mut self, ui: &mut egui::Ui) {
        let mut refresh = false;
        let loading = self.snapshot.contributors_loading;
        let error = self.snapshot.contributors_error.clone();
        let contributors = self.snapshot.contributors.clone();
        kit::section_card(
            ui,
            icons::USERS_THREE,
            "贡献者",
            "来自 GitHub 仓库的贡献者列表",
            |ui| {
                if kit::ghost_button(ui, icons::ARROWS_CLOCKWISE, "刷新").clicked() {
                    refresh = true;
                }
            },
            |ui| {
                if loading && contributors.is_empty() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new("正在获取贡献者…")
                                .size(10.5)
                                .color(theme::muted(ui)),
                        );
                    });
                    return;
                }
                if let Some(error) = &error {
                    ui.label(RichText::new(error).size(10.0).color(theme::ORANGE));
                }
                if contributors.is_empty() {
                    ui.label(
                        RichText::new("暂未获取到贡献者列表，可稍后重试。")
                            .size(10.5)
                            .color(theme::muted(ui)),
                    );
                    ui.hyperlink_to(
                        "在 GitHub 查看贡献者",
                        "https://github.com/ImYoyoData/yoyo-relay/graphs/contributors",
                    );
                    return;
                }
                for contributor in &contributors {
                    if self.contributor_textures.get(&contributor.login).is_none() {
                        if let Some(bytes) = &contributor.avatar {
                            if let Ok(image) = image::load_from_memory(bytes) {
                                let rgba = image.to_rgba8();
                                let (width, height) = rgba.dimensions();
                                let color = egui::ColorImage::from_rgba_unmultiplied(
                                    [width as usize, height as usize],
                                    rgba.as_raw(),
                                );
                                let texture = ui.ctx().load_texture(
                                    format!("contributor-{}", contributor.login),
                                    color,
                                    egui::TextureOptions::LINEAR,
                                );
                                self.contributor_textures
                                    .insert(contributor.login.clone(), texture);
                            }
                        }
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    for contributor in &contributors {
                        let profile_url = contributor.url.clone();
                        let response = ui.allocate_ui(egui::Vec2::new(86.0, 88.0), |ui| {
                            ui.vertical_centered(|ui| {
                                let size = 48.0;
                                if let Some(texture) =
                                    self.contributor_textures.get(&contributor.login)
                                {
                                    ui.add(
                                        egui::Image::new(texture)
                                            .fit_to_exact_size(egui::Vec2::splat(size))
                                            .corner_radius(size / 2.0),
                                    );
                                } else {
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::Vec2::splat(size),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().circle_filled(
                                        rect.center(),
                                        size / 2.0,
                                        theme::soft_for(ui, theme::PURPLE),
                                    );
                                    let initial = contributor
                                        .login
                                        .chars()
                                        .next()
                                        .unwrap_or('?')
                                        .to_ascii_uppercase();
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        initial,
                                        egui::FontId::proportional(19.0),
                                        theme::PURPLE,
                                    );
                                }
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&contributor.login)
                                            .size(9.2)
                                            .color(theme::muted(ui)),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(format!(
                                    "{} · {} 次提交",
                                    contributor.login, contributor.contributions
                                ));
                            });
                        });
                        response
                            .response
                            .interact(egui::Sense::click())
                            .on_hover_text(format!("打开 {profile_url}"))
                            .clicked()
                            .then(|| crate::ui::open_url(&profile_url));
                    }
                });
            },
        );
        if refresh {
            self.contributors_requested = true;
            let _ = self.cmd_tx.send(UiCommand::RefreshContributors);
        }
    }

    fn settings_about(&mut self, ui: &mut egui::Ui) {
        if self.snapshot.contributors.is_empty()
            && !self.snapshot.contributors_loading
            && self.snapshot.contributors_error.is_none()
            && !self.contributors_requested
        {
            self.contributors_requested = true;
            let _ = self.cmd_tx.send(UiCommand::RefreshContributors);
        }
        let uptime = crate::config::fmt_duration_secs(self.snapshot.uptime_secs);
        let mut changed = false;
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
                ui.add_space(8.0);
                egui::Frame::new()
                    .fill(theme::subtle_fill(ui))
                    .corner_radius(9)
                    .inner_margin(egui::Margin::symmetric(11, 9))
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if kit::toggle_switch(ui, &mut self.config.ui.auto_update_check)
                                .changed()
                            {
                                changed = true;
                            }
                            ui.label(
                                RichText::new("自动检查更新")
                                    .size(11.0)
                                    .strong()
                                    .color(theme::text(ui)),
                            );
                            ui.label(RichText::new("每").size(10.0).color(theme::muted(ui)));
                            if ui
                                .add(
                                    egui::DragValue::new(&mut self.config.ui.update_check_hours)
                                        .range(1..=168)
                                        .suffix(" 小时"),
                                )
                                .changed()
                            {
                                changed = true;
                            }
                            ui.label(RichText::new("检查一次").size(10.0).color(theme::muted(ui)));
                        });
                        ui.add_space(3.0);
                        let update = &self.snapshot.update;
                        let status = if update.checking {
                            "正在检查…".to_string()
                        } else if let Some(error) = &update.error {
                            format!("检查失败：{error}")
                        } else if update.available {
                            format!("发现新版本 v{}", update.latest)
                        } else if update.checked_at > 0 {
                            format!(
                                "已是最新版本 · {}",
                                widgets::time_ago(update.checked_at, crate::config::now_ts())
                            )
                        } else {
                            "尚未检查".to_string()
                        };
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(status).size(10.0).color(if update.available {
                                theme::GREEN
                            } else {
                                theme::muted(ui)
                            }));
                            if kit::ghost_button(ui, icons::ARROWS_CLOCKWISE, "立即检查").clicked()
                            {
                                let _ = self.cmd_tx.send(UiCommand::CheckUpdate { manual: true });
                            }
                            if update.available
                                && kit::ghost_button(ui, icons::DOWNLOAD_SIMPLE, "查看更新")
                                    .clicked()
                            {
                                crate::ui::open_url(&update.url);
                            }
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
        self.contributors_card(ui);
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
        if changed {
            self.persist();
        }
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

fn proxy_bind_id() -> egui::Id {
    egui::Id::new("proxy_bind")
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
                        .id(proxy_bind_id())
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

fn scheduler_slow_fields(ui: &mut Ui, cfg: &mut crate::config::SchedulerConfig) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        if kit::toggle_switch(ui, &mut cfg.slow_switch).changed() {
            changed = true;
        }
        ui.label(RichText::new("慢请求自动切换").size(12.0).strong());
        ui.label(
            RichText::new(
                "首包超时后先换本供应商的其他账号，再换其他供应商；第一个请求失败也走同一条链路",
            )
            .size(10.0)
            .color(theme::muted(ui)),
        );
    });
    if cfg.slow_switch {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label("首包超时");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut cfg.slow_first_byte_secs)
                        .range(3..=300)
                        .suffix(" 秒"),
                )
                .changed();
            ui.label("慢响应冷却");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut cfg.slow_cooldown_secs)
                        .range(5..=3600)
                        .suffix(" 秒"),
                )
                .changed();
            ui.label(
                RichText::new("仅统计首包/首个流式分片，不打断已开始输出的响应")
                    .size(9.5)
                    .color(theme::muted(ui)),
            );
        });
    }
    changed
}

fn fmt_price(value: f64) -> String {
    if value == 0.0 {
        "—".to_string()
    } else if value >= 100.0 {
        format!("{value:.1}")
    } else if value >= 1.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.4}")
    }
}

/// Parse `#RRGGBB` (or `RRGGBB`) into an RGB triple.
pub fn parse_hex_color(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r, g, b))
}
