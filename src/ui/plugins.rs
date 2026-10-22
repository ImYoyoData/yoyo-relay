use egui::RichText;
use egui_phosphor::regular as icons;

use crate::plugins::{PluginInfo, PluginStatus, PluginUiKind};
use crate::runtime::UiCommand;
use crate::snapshot::NoticeLevel;

use super::{kit, theme, widgets, App, Page};

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
        let mut reload = false;
        let mut test = false;
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
                    let mut enabled = plugin.enabled;
                    if kit::toggle_switch(ui, &mut enabled).changed() {
                        toggle = Some(enabled);
                    }
                    ui.label(
                        RichText::new(if plugin.enabled { "启用" } else { "禁用" })
                            .size(10.0)
                            .color(theme::muted(ui)),
                    );
                    if running {
                        if kit::secondary_button(ui, icons::FLASK, "自检").clicked() {
                            test = true;
                        }
                        if kit::icon_button(ui, icons::ARROWS_CLOCKWISE, "重新加载").clicked() {
                            reload = true;
                        }
                        if kit::icon_button(ui, icons::STOP, "卸载插件（保留启用状态）").clicked()
                        {
                            unload = true;
                        }
                    } else if plugin.enabled {
                        if kit::secondary_button(ui, icons::PLAY, "加载").clicked() {
                            load = true;
                        }
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
            if let Some(error) = &plugin.last_error {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(icons::WARNING).size(12.0).color(theme::RED));
                    ui.label(RichText::new(error).size(10.0).color(theme::RED));
                });
            }
            if !plugin.logs.is_empty() {
                ui.add_space(2.0);
                egui::CollapsingHeader::new(
                    RichText::new(format!("运行日志（{}）", plugin.logs.len()))
                        .size(10.5)
                        .color(theme::muted(ui)),
                )
                .id_salt(format!("plugin_logs_{}", plugin.id))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(format!("plugin_logs_scroll_{}", plugin.id))
                        .max_height(160.0)
                        .show(ui, |ui| {
                            for log in plugin.logs.iter().rev().take(60) {
                                let color = match log.level.as_str() {
                                    "error" => theme::RED,
                                    "warn" => theme::ORANGE,
                                    _ => theme::muted(ui),
                                };
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        RichText::new(widgets::format_time(log.at))
                                            .monospace()
                                            .size(9.0)
                                            .color(theme::muted(ui)),
                                    );
                                    ui.label(
                                        RichText::new(&log.level)
                                            .monospace()
                                            .size(9.0)
                                            .color(color),
                                    );
                                    ui.label(
                                        RichText::new(&log.message)
                                            .size(9.5)
                                            .color(theme::text(ui)),
                                    );
                                });
                            }
                        });
                });
            }
        });
        if let Some(enabled) = toggle {
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
    }

    pub(crate) fn show_plugin_docs(&mut self, ctx: &egui::Context) {
        if !self.plugin_docs_open {
            return;
        }
        let mut close = false;
        let closed = kit::modal_shell(
            ctx,
            egui::Id::new("plugin_api_docs"),
            "插件 API 文档",
            "ES Module · 生命周期事件 · 宿主 API · 资源限制",
            780.0,
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("plugin_docs_scroll")
                    .max_height(520.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(crate::plugins::docs::API_DOC)
                                    .monospace()
                                    .size(10.5),
                            )
                            .wrap(),
                        );
                    });
            },
            |ui| {
                if kit::primary_button(ui, icons::CHECK, "关闭").clicked() {
                    close = true;
                }
            },
        );
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
                        egui::TextEdit::singleline(&mut text)
                            .desired_width(f32::INFINITY)
                            .hint_text("输入内容"),
                    );
                }
            },
            |ui| {
                if kit::primary_button(ui, icons::CHECK, if is_prompt { "提交" } else { "确认" })
                    .clicked()
                {
                    accepted = true;
                    answer = Some((true, String::new()));
                }
                if kit::secondary_button(ui, icons::X, "取消").clicked() {
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
        }
    }
}
