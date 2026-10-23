use egui::RichText;
use egui_phosphor::regular as icons;

use crate::plugins::{PluginInfo, PluginStatus, PluginUiKind};
use crate::runtime::UiCommand;
use crate::snapshot::NoticeLevel;

use super::{kit, theme, widgets, App, ConfirmAction, Page};

fn open_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    let _ = std::fs::create_dir_all(path);
    #[cfg(windows)]
    {
        std::process::Command::new("explorer").arg(path).spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(path).spawn()?;
    }
    Ok(())
}

impl App {
    pub(crate) fn page_plugins(&mut self, ui: &mut egui::Ui) {
        let mut install_samples = false;
        let mut rescan = false;
        let mut open_docs = false;
        kit::page_header(
            ui,
            Page::Plugins.eyebrow(),
            "插件",
            "每个 JS 插件运行在独立线程与 Boa Context 中，支持生命周期事件、异步 API 与参数拦截",
            |ui| {
                if kit::secondary_button(ui, icons::BOOK_OPEN, "API 文档").clicked() {
                    open_docs = true;
                }
                if kit::secondary_button(ui, icons::ARROWS_CLOCKWISE, "重新扫描").clicked() {
                    rescan = true;
                }
                if kit::primary_button(ui, icons::DOWNLOAD_SIMPLE, "安装示例").clicked() {
                    install_samples = true;
                }
            },
        );
        if open_docs {
            self.plugin_docs_open = true;
        }
        if rescan {
            let _ = self.cmd_tx.send(UiCommand::ReloadPlugins);
        }
        if install_samples {
            let _ = self.cmd_tx.send(UiCommand::InstallSamplePlugins);
        }

        let mut settings_changed = false;
        let running = self
            .snapshot
            .plugins
            .iter()
            .filter(|plugin| plugin.status == PluginStatus::Running)
            .count();
        let dir = self
            .store
            .paths
            .config_dir
            .join("plugins")
            .display()
            .to_string();
        kit::section_card(
            ui,
            icons::SHIELD_CHECK,
            "插件权限",
            "修改网络/进程权限后会自动重载正在运行的插件",
            |ui| {
                kit::badge(ui, &format!("{running} 个运行中"), theme::GREEN);
            },
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    if kit::toggle_switch(ui, &mut self.config.plugins.allow_network).changed() {
                        settings_changed = true;
                    }
                    ui.label(
                        RichText::new("允许网络访问")
                            .size(11.0)
                            .strong()
                            .color(theme::text(ui)),
                    );
                    ui.label(
                        RichText::new("host.http.request；关闭后插件网络调用会直接报错")
                            .size(10.0)
                            .color(theme::muted(ui)),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    if kit::toggle_switch(ui, &mut self.config.plugins.allow_process).changed() {
                        settings_changed = true;
                    }
                    ui.label(
                        RichText::new("允许进程调用")
                            .size(11.0)
                            .strong()
                            .color(theme::text(ui)),
                    );
                    ui.label(
                        RichText::new("host.process.run 可执行本机程序，默认关闭")
                            .size(10.0)
                            .color(theme::muted(ui)),
                    );
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("插件目录").size(10.0).color(theme::muted(ui)));
                    ui.label(
                        RichText::new(&dir)
                            .monospace()
                            .size(10.0)
                            .color(theme::text(ui)),
                    );
                    if kit::ghost_button(ui, icons::COPY, "复制").clicked() {
                        self.copy(ui.ctx(), &dir, "插件目录");
                    }
                    if kit::ghost_button(ui, icons::FOLDER_OPEN, "打开目录").clicked() {
                        if let Err(err) = open_in_file_manager(std::path::Path::new(&dir)) {
                            self.toast(NoticeLevel::Error, format!("无法打开插件目录：{err}"));
                        }
                    }
                });
                kit::info_row(
                    ui,
                    icons::INFO,
                    "插件目录下每个文件夹是一个插件（含 plugin.json + main.js），单个 .js 文件也可直接作为插件；数据目录为 plugin-data/<id>。",
                );
            },
        );
        if settings_changed {
            self.persist();
        }
        ui.add_space(12.0);

        let plugins = self.snapshot.plugins.clone();
        if plugins.is_empty() {
            if kit::empty_state(
                ui,
                icons::PUZZLE_PIECE,
                "还没有插件",
                "点右上角「安装示例」写入三个示例插件，或把插件目录放到插件目录后重新扫描。",
                icons::DOWNLOAD_SIMPLE,
                "安装示例插件",
            ) {
                let _ = self.cmd_tx.send(UiCommand::InstallSamplePlugins);
            }
            return;
        }

        for plugin in &plugins {
            self.plugin_card(ui, plugin);
            ui.add_space(10.0);
        }
    }

    fn plugin_card(&mut self, ui: &mut egui::Ui, plugin: &PluginInfo) {
        let running = plugin.status == PluginStatus::Running;
        let (color, label) = match plugin.status {
            PluginStatus::Running => (theme::GREEN, plugin.status.label()),
            PluginStatus::Error => (theme::RED, plugin.status.label()),
            PluginStatus::Disabled => (theme::MUTED, plugin.status.label()),
            PluginStatus::Stopped => (theme::ORANGE, plugin.status.label()),
        };
        let mut toggle = None;
        let mut load = false;
        let mut unload = false;
        let mut uninstall = false;
        let mut reload = false;
        let mut test = false;
        let mut open_console = false;
        kit::surface(ui, |ui| {
            ui.horizontal(|ui| {
                kit::icon_chip(ui, icons::PUZZLE_PIECE, theme::PURPLE, 38.0);
                ui.add_space(2.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&plugin.name).size(14.5).strong());
                        kit::badge(ui, &format!("v{}", plugin.version), theme::MUTED);
                        kit::status_pill(ui, label, color);
                        if !plugin.enabled {
                            kit::plain_badge(ui, "已禁用");
                        }
                        if self
                            .plugin_overrides
                            .get(&plugin.id)
                            .map(|(value, _)| *value != plugin.enabled)
                            .unwrap_or(false)
                        {
                            kit::badge(ui, "切换中…", theme::ORANGE);
                        }
                    });
                    if plugin.description.trim().is_empty() {
                        ui.label(
                            RichText::new(&plugin.id)
                                .monospace()
                                .size(9.5)
                                .color(theme::muted(ui)),
                        );
                    } else {
                        ui.label(
                            RichText::new(&plugin.description)
                                .size(10.5)
                                .color(theme::muted(ui)),
                        );
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut enabled = self
                        .plugin_overrides
                        .get(&plugin.id)
                        .map(|(value, _)| *value)
                        .unwrap_or(plugin.enabled);
                    if kit::toggle_row(ui, &mut enabled, "启用").changed() {
                        toggle = Some(enabled);
                    }
                    if running {
                        if kit::secondary_button(ui, icons::FLASK, "自检").clicked() {
                            test = true;
                        }
                        if kit::icon_button(ui, icons::ARROWS_CLOCKWISE, "重新加载").clicked() {
                            reload = true;
                        }
                        if kit::icon_button(ui, icons::STOP, "停止插件（保留启用状态）").clicked()
                        {
                            unload = true;
                        }
                    } else if plugin.enabled {
                        if kit::secondary_button(ui, icons::PLAY, "加载").clicked() {
                            load = true;
                        }
                    }
                    if kit::icon_button(ui, icons::TRASH_SIMPLE, "卸载插件（删除插件文件）")
                        .clicked()
                    {
                        uninstall = true;
                    }
                    if kit::icon_button(ui, icons::TERMINAL_WINDOW, "运行日志控制台").clicked()
                    {
                        open_console = true;
                    }
                    if kit::ghost_button(ui, icons::COPY, "路径").clicked() {
                        self.copy(
                            ui.ctx(),
                            &plugin.entry.display().to_string(),
                            "插件入口路径",
                        );
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(plugin.entry.display().to_string())
                        .monospace()
                        .size(9.0)
                        .color(theme::muted(ui)),
                );
                for event in &plugin.events {
                    kit::plain_badge(ui, event);
                }
                if !plugin.author.trim().is_empty() {
                    ui.label(
                        RichText::new(format!("作者 {}", plugin.author))
                            .size(9.0)
                            .color(theme::muted(ui)),
                    );
                }
                ui.label(
                    RichText::new(format!("调用 {} 次", plugin.calls))
                        .size(9.0)
                        .color(theme::muted(ui)),
                );
                if plugin.log_count > 0 {
                    kit::badge(ui, &format!("日志 {}", plugin.log_count), theme::MUTED);
                }
                if plugin.loaded_at > 0 && running {
                    ui.label(
                        RichText::new(format!(
                            "加载于 {}",
                            widgets::time_ago(plugin.loaded_at, crate::config::now_ts())
                        ))
                        .size(9.0)
                        .color(theme::muted(ui)),
                    );
                }
            });
            if !plugin.providers.is_empty() {
                ui.add_space(3.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("临时供应商")
                            .size(9.5)
                            .strong()
                            .color(theme::PURPLE),
                    );
                    for provider in &plugin.providers {
                        kit::badge(
                            ui,
                            &format!("{} · {}", provider.name, provider.protocol),
                            theme::PURPLE,
                        );
                        for model in provider.models.iter().take(6) {
                            kit::plain_badge(ui, model);
                        }
                        if provider.models.len() > 6 {
                            kit::plain_badge(ui, &format!("+{}", provider.models.len() - 6));
                        }
                    }
                });
                ui.label(
                    RichText::new("虚拟账号自动创建并参与调度；插件卸载或禁用时自动移除。")
                        .size(9.0)
                        .color(theme::muted(ui)),
                );
            }
            if let Some(error) = &plugin.last_error {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(icons::WARNING).size(12.0).color(theme::RED));
                    ui.label(RichText::new(error).size(10.0).color(theme::RED));
                });
            }
        });
        if let Some(enabled) = toggle {
            self.plugin_overrides
                .insert(plugin.id.clone(), (enabled, std::time::Instant::now()));
            let _ = self.cmd_tx.send(UiCommand::SetPluginEnabled {
                id: plugin.id.clone(),
                enabled,
            });
            self.toast(
                NoticeLevel::Info,
                if enabled {
                    "正在启用插件…"
                } else {
                    "正在禁用插件…"
                },
            );
        }
        if load {
            let _ = self.cmd_tx.send(UiCommand::LoadPlugin(plugin.id.clone()));
        }
        if unload {
            let _ = self.cmd_tx.send(UiCommand::UnloadPlugin(plugin.id.clone()));
        }
        if uninstall {
            self.plugin_uninstall_keep_data = true;
            self.confirm = Some(ConfirmAction::UninstallPlugin(plugin.id.clone()));
        }
        if reload {
            let _ = self.cmd_tx.send(UiCommand::ReloadPlugin(plugin.id.clone()));
        }
        if test {
            let _ = self.cmd_tx.send(UiCommand::InvokePlugin {
                id: plugin.id.clone(),
                method: "selfTest".into(),
            });
            self.toast(NoticeLevel::Info, "正在运行插件自检…");
        }
        if open_console {
            self.plugin_console = Some(plugin.id.clone());
            self.plugin_console_auto_scroll = true;
            let _ = self
                .cmd_tx
                .send(UiCommand::SetPluginConsole(Some(plugin.id.clone())));
        }
    }

    /// Log console modal. Rows are virtualised with `show_rows`, so the full
    /// 500-line buffer stays cheap even while it updates.
    pub(crate) fn show_plugin_console(&mut self, ctx: &egui::Context) {
        if self.plugin_console.is_none() {
            return;
        }
        let view = self
            .snapshot
            .plugin_console
            .clone()
            .filter(|view| Some(&view.id) == self.plugin_console.as_ref());
        let mut close = false;
        let mut clear = false;
        let mut copy = false;
        let title = view
            .as_ref()
            .map(|view| format!("插件控制台 · {}", view.name))
            .unwrap_or_else(|| "插件控制台".to_string());
        let logs = view
            .as_ref()
            .map(|view| {
                view.logs
                    .iter()
                    .filter(|log| match self.plugin_console_level {
                        1 => log.level == "info" || log.level == "debug",
                        2 => log.level == "warn",
                        3 => log.level == "error",
                        _ => true,
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let total = view.as_ref().map(|view| view.logs.len()).unwrap_or(0);
        let closed = kit::modal_shell(
            ctx,
            egui::Id::new("plugin_log_console"),
            &title,
            "最新 500 条运行日志 · 级别过滤 · 自动滚动",
            840.0,
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("级别").size(10.0).color(theme::muted(ui)));
                    if let Some(level) = widgets::segmented(
                        ui,
                        &[(0u8, "全部"), (1, "信息"), (2, "警告"), (3, "错误")],
                        self.plugin_console_level,
                    ) {
                        self.plugin_console_level = level;
                    }
                    ui.checkbox(&mut self.plugin_console_auto_scroll, "自动滚动");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "显示 {} / 共 {} 条（最多保留 500）",
                                logs.len(),
                                total
                            ))
                            .size(9.5)
                            .color(theme::muted(ui)),
                        );
                        if kit::secondary_button(ui, icons::COPY, "复制").clicked() {
                            copy = true;
                        }
                        if kit::danger_button(ui, icons::TRASH_SIMPLE, "清空").clicked() {
                            clear = true;
                        }
                    });
                });
                ui.add_space(6.0);
                let dark = theme::tokens(ui).dark_mode;
                egui::Frame::new()
                    .fill(if dark {
                        egui::Color32::from_rgb(12, 16, 22)
                    } else {
                        egui::Color32::from_rgb(15, 23, 42)
                    })
                    .corner_radius(9)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        let rows = logs.len();
                        let stick = self.plugin_console_auto_scroll;
                        kit::scroll_rows(
                            ui,
                            "plugin_log_console_scroll",
                            500.0,
                            stick,
                            18.0,
                            rows,
                            |ui, range| {
                                for index in range {
                                    let log = &logs[index];
                                    let color = match log.level.as_str() {
                                        "error" => theme::RED,
                                        "warn" => theme::ORANGE,
                                        "debug" => theme::MUTED,
                                        _ => theme::GREEN,
                                    };
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(ui.available_width(), 18.0),
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.add_sized(
                                                egui::vec2(124.0, 16.0),
                                                egui::Label::new(
                                                    RichText::new(widgets::format_time(log.at))
                                                        .monospace()
                                                        .size(9.0)
                                                        .color(theme::MUTED),
                                                )
                                                .truncate(),
                                            );
                                            ui.add_sized(
                                                egui::vec2(38.0, 16.0),
                                                egui::Label::new(
                                                    RichText::new(&log.level)
                                                        .monospace()
                                                        .size(9.0)
                                                        .color(color),
                                                )
                                                .truncate(),
                                            );
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&log.message)
                                                        .monospace()
                                                        .size(9.5)
                                                        .color(egui::Color32::from_rgb(
                                                            226, 232, 240,
                                                        )),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(&log.message);
                                        },
                                    );
                                }
                            },
                        );
                    });
                if view.is_none() {
                    ui.add_space(4.0);
                    let exists = self
                        .snapshot
                        .plugins
                        .iter()
                        .any(|plugin| Some(&plugin.id) == self.plugin_console.as_ref());
                    ui.horizontal(|ui| {
                        if exists {
                            ui.spinner();
                            ui.label(
                                RichText::new("正在读取日志…")
                                    .size(10.5)
                                    .color(theme::muted(ui)),
                            );
                        } else {
                            ui.label(
                                RichText::new("插件已移除，日志不再可用。")
                                    .size(10.5)
                                    .color(theme::muted(ui)),
                            );
                        }
                    });
                }
            },
            |ui| {
                if kit::secondary_button(ui, icons::X, "关闭").clicked() {
                    close = true;
                }
            },
        );
        if clear {
            if let Some(id) = self.plugin_console.clone() {
                let _ = self.cmd_tx.send(UiCommand::ClearPluginLogs(id));
            }
        }
        if copy {
            let text = logs
                .iter()
                .map(|log| {
                    format!(
                        "{} [{}] {}",
                        widgets::format_time(log.at),
                        log.level,
                        log.message
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            self.copy(ctx, &text, "插件日志");
        }
        if closed || close {
            self.plugin_console = None;
            let _ = self.cmd_tx.send(UiCommand::SetPluginConsole(None));
        }
    }

    pub(crate) fn show_plugin_docs(&mut self, ctx: &egui::Context) {
        if !self.plugin_docs_open {
            return;
        }
        let mut close = false;
        let mut download_skill = false;
        let closed = kit::modal_shell(
            ctx,
            egui::Id::new("plugin_api_docs"),
            "插件 API 文档",
            "ES Module · 生命周期事件 · 宿主 API · 虚拟供应商 · WebSocket / SSE · 资源限制",
            780.0,
            |ui| {
                super::markdown::api_doc(ui);
            },
            |ui| {
                if kit::secondary_button(ui, icons::DOWNLOAD_SIMPLE, "下载开发 Skill").clicked()
                {
                    download_skill = true;
                }
                if kit::primary_button(ui, icons::CHECK, "关闭").clicked() {
                    close = true;
                }
            },
        );
        if download_skill {
            let _ = self.cmd_tx.send(UiCommand::ExportPluginSkill);
        }
        if closed || close {
            self.plugin_docs_open = false;
        }
    }

    /// Confirm/prompt requests raised by `host.ui.confirm` / `host.ui.prompt`.
    pub(crate) fn show_plugin_ui(&mut self, ctx: &egui::Context) {
        let Some(request) = self.snapshot.plugin_ui.clone() else {
            return;
        };
        if self.plugin_ui_id != request.id {
            self.plugin_ui_id = request.id;
            self.plugin_ui_text = request.default_value.clone();
        }
        let mut answer: Option<(bool, String)> = None;
        let mut text = self.plugin_ui_text.clone();
        let mut accepted = false;
        let is_prompt = request.kind == PluginUiKind::Prompt;
        let is_alert = request.kind == PluginUiKind::Alert;
        let closed = kit::modal_shell(
            ctx,
            egui::Id::new(("plugin_ui", request.id)),
            &request.title,
            &format!(
                "来自插件「{}」（{}）",
                request.plugin_name, request.plugin_id
            ),
            520.0,
            |ui| {
                egui::Frame::new()
                    .fill(theme::subtle_fill(ui))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(&request.message).size(12.0)).wrap());
                    });
                if is_prompt {
                    ui.add_space(8.0);
                    ui.add(
                        kit::text_input(&mut text)
                            .id(egui::Id::new("plugin_ui_prompt"))
                            .desired_width(f32::INFINITY)
                            .hint_text("输入内容"),
                    );
                }
            },
            |ui| {
                let confirm_text = if is_alert {
                    "知道了"
                } else if is_prompt {
                    "提交"
                } else {
                    "确认"
                };
                if kit::primary_button(ui, icons::CHECK, confirm_text).clicked() {
                    accepted = true;
                    answer = Some((true, String::new()));
                }
                if !is_alert && kit::secondary_button(ui, icons::X, "取消").clicked() {
                    answer = Some((false, String::new()));
                }
            },
        );
        if answer.is_some() {
            let _ = self.cmd_tx.send(UiCommand::AnswerPluginUi {
                id: request.id,
                accepted,
                text: if accepted { text } else { String::new() },
            });
            self.plugin_ui_text.clear();
        } else if closed {
            let _ = self.cmd_tx.send(UiCommand::AnswerPluginUi {
                id: request.id,
                accepted: false,
                text: String::new(),
            });
            self.plugin_ui_text.clear();
        } else {
            // Keep the in-progress text across frames; the modal is rebuilt
            // from this field every frame.
            self.plugin_ui_text = text;
        }
    }
}
