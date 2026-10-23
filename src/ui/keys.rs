use armas::components::Progress;
use egui::{RichText, Vec2};
use egui_phosphor::regular as icons;

use crate::config::LocalKey;
use crate::snapshot::NoticeLevel;

use super::{theme, widgets, App, ConfirmAction};

impl App {
    pub(crate) fn page_local_keys(&mut self, ui: &mut egui::Ui) {
        super::kit::page_header(
            ui,
            super::Page::LocalKeys.eyebrow(),
            "本地 Key",
            "客户端接入凭据、模型白名单与独立额度",
            |ui| {
                if super::kit::primary_labeled(ui, icons::PLUS, "新建 Key").clicked() {
                    self.open_new_local_key();
                }
            },
        );

        ui.horizontal(|ui| {
            if self.config.server.auth_enabled {
                super::kit::status_badge(ui, "鉴权已开启", theme::GREEN);
            } else {
                super::kit::status_badge(ui, "鉴权已关闭", theme::ORANGE);
            }
            ui.label(
                RichText::new(format!("{} 个本地 Key", self.config.local_keys.len()))
                    .size(11.0)
                    .color(theme::muted(ui)),
            );
        });
        ui.add_space(8.0);

        super::kit::soft_panel(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(icons::TERMINAL_WINDOW)
                        .size(14.0)
                        .color(theme::muted(ui)),
                );
                ui.label(
                    RichText::new("Authorization: Bearer <Key>")
                        .monospace()
                        .size(10.5)
                        .color(theme::muted(ui)),
                );
                ui.label(RichText::new("·").size(10.0).color(theme::muted(ui)));
                ui.label(
                    RichText::new(if self.snapshot.endpoint.is_empty() {
                        "—".to_string()
                    } else {
                        self.snapshot.endpoint.clone()
                    })
                    .monospace()
                    .size(10.5)
                    .color(theme::muted(ui)),
                );
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(icons::COPY)
                                .size(12.0)
                                .color(theme::muted(ui)),
                        )
                        .frame(false)
                        .min_size(Vec2::splat(18.0)),
                    )
                    .on_hover_text("复制端点")
                    .clicked()
                {
                    ui.ctx().copy_text(self.snapshot.endpoint.clone());
                    self.toast(NoticeLevel::Success, "端点已复制");
                }
            });
        });

        if !self.config.server.auth_enabled {
            ui.add_space(6.0);
            super::kit::tooltip_text(
                ui,
                icons::WARNING_CIRCLE,
                "当前代理未启用鉴权：任何客户端都可以访问本地端点。建议在设置页开启。",
            );
        }
        ui.add_space(12.0);

        if self.config.local_keys.is_empty() {
            if super::kit::empty_state(
                ui,
                icons::KEY,
                "还没有本地 Key",
                "本地 Key 用于客户端访问代理，可单独限制模型白名单与额度。",
                icons::PLUS,
                "新建 Key",
            ) {
                self.open_new_local_key();
            }
            return;
        }

        let keys: Vec<LocalKey> = self
            .config
            .local_keys
            .iter()
            .filter(|k| self.matches_search(&k.name))
            .cloned()
            .collect();
        if keys.is_empty() {
            ui.label(
                RichText::new("没有匹配的 Key。")
                    .size(12.0)
                    .color(theme::muted(ui)),
            );
            return;
        }

        let mut action: Option<KeyAction> = None;
        let columns = if ui.available_width() >= 720.0 { 2 } else { 1 };
        let gap = 10.0;
        let card_width =
            ((ui.available_width() - gap * (columns - 1) as f32) / columns as f32).max(280.0);
        egui::Grid::new("local_keys_grid")
            .num_columns(columns)
            .spacing([gap, 10.0])
            .show(ui, |ui| {
                for (index, key) in keys.iter().enumerate() {
                    ui.allocate_ui_with_layout(
                        Vec2::new(card_width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(card_width);
                            self.key_card(ui, key, &mut action);
                        },
                    );
                    if (index + 1) % columns == 0 {
                        ui.end_row();
                    }
                }
                if keys.len() % columns != 0 {
                    ui.end_row();
                }
            });

        if let Some(action) = action {
            match action {
                KeyAction::Copy(text) => self.copy(ui.ctx(), &text, "本地 Key"),
                KeyAction::CopyFailed => {
                    self.toast(NoticeLevel::Error, "无法解密该 Key，请重置后重新复制。")
                }
                KeyAction::Test(id) => self.start_test("local_key", &id),
                KeyAction::Toggle(id, enabled) => self.set_key_enabled(&id, enabled),
                KeyAction::Reset(id) => self.confirm = Some(ConfirmAction::ResetLocalKey(id)),
                KeyAction::Delete(id) => self.confirm = Some(ConfirmAction::DeleteLocalKey(id)),
            }
        }

        ui.add_space(6.0);
        super::kit::tooltip_text(
            ui,
            icons::INFO,
            "客户端可用 GET /v1/models 校验 Key 与模型白名单；测试请求不计入额度。",
        );
    }

    fn key_card(&self, ui: &mut egui::Ui, key: &LocalKey, action: &mut Option<KeyAction>) {
        let view = self.snapshot.local_keys.iter().find(|k| k.id == key.id);
        let test_key = format!("local_key:{}", key.id);
        let running = self.snapshot.running_tests.contains(&test_key);
        let used_tokens = view.map(|v| v.used_tokens).unwrap_or(0);
        let used_requests = view.map(|v| v.used_requests).unwrap_or(0);

        super::kit::surface(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                super::kit::icon_tile(ui, icons::KEY, theme::ACCENT, 36.0);
                ui.add_space(3.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&key.name).size(14.5).strong());
                        if key.enabled {
                            super::kit::status_badge(ui, "启用", theme::GREEN);
                        } else {
                            super::kit::status_badge(ui, "禁用", theme::MUTED);
                        }
                    });
                    ui.label(
                        RichText::new(format!("创建于 {}", widgets::format_time(key.created_at)))
                            .size(10.0)
                            .color(theme::muted(ui)),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if super::kit::danger_icon_button(ui, icons::TRASH_SIMPLE, "删除 Key").clicked()
                    {
                        *action = Some(KeyAction::Delete(key.id.clone()));
                    }
                    if super::kit::icon_button(ui, icons::ARROWS_CLOCKWISE, "重置 Key").clicked()
                    {
                        *action = Some(KeyAction::Reset(key.id.clone()));
                    }
                    if super::kit::icon_button(
                        ui,
                        if key.enabled {
                            icons::POWER
                        } else {
                            icons::PLAY
                        },
                        if key.enabled {
                            "禁用 Key"
                        } else {
                            "启用 Key"
                        },
                    )
                    .clicked()
                    {
                        *action = Some(KeyAction::Toggle(key.id.clone(), !key.enabled));
                    }
                    if super::kit::icon_button(
                        ui,
                        if running {
                            icons::SPINNER
                        } else {
                            icons::FLASK
                        },
                        "测试 Key",
                    )
                    .clicked()
                        && !running
                    {
                        *action = Some(KeyAction::Test(key.id.clone()));
                    }
                    if super::kit::icon_button(ui, icons::COPY, "复制 Key").clicked() {
                        match self.store.decrypt(&key.key) {
                            Ok(plain) if !plain.is_empty() => {
                                *action = Some(KeyAction::Copy(plain));
                            }
                            _ => *action = Some(KeyAction::CopyFailed),
                        }
                    }
                });
            });

            ui.add_space(8.0);
            super::kit::soft_panel(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(
                            view.map(|v| v.masked.clone())
                                .unwrap_or_else(|| "••••".into()),
                        )
                        .monospace()
                        .size(11.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(icons::COPY)
                                        .size(11.0)
                                        .color(theme::muted(ui)),
                                )
                                .frame(false)
                                .min_size(Vec2::splat(18.0)),
                            )
                            .on_hover_text("复制完整 Key")
                            .clicked()
                        {
                            match self.store.decrypt(&key.key) {
                                Ok(plain) if !plain.is_empty() => {
                                    *action = Some(KeyAction::Copy(plain));
                                }
                                _ => *action = Some(KeyAction::CopyFailed),
                            }
                        }
                    });
                });
            });

            ui.add_space(7.0);
            ui.horizontal_wrapped(|ui| {
                super::kit::field_label(ui, "白名单");
                if key.allowed_models.is_empty() {
                    super::kit::badge(ui, "全部模型", theme::BLUE_INFO);
                } else {
                    for model in key.allowed_models.iter().take(6) {
                        super::kit::badge(ui, model, theme::MUTED);
                    }
                    if key.allowed_models.len() > 6 {
                        super::kit::badge(
                            ui,
                            &format!("+{}", key.allowed_models.len() - 6),
                            theme::MUTED,
                        );
                    }
                }
            });

            ui.add_space(5.0);
            ui.horizontal(|ui| {
                super::kit::field_label(ui, "额度");
                ui.label(
                    RichText::new(format!("{} tokens", widgets::format_count(used_tokens)))
                        .monospace()
                        .size(11.0),
                );
                ui.label(
                    RichText::new(format!("{used_requests} 次"))
                        .monospace()
                        .size(10.5)
                        .color(theme::muted(ui)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let limit_text = match (key.max_tokens, key.max_requests) {
                        (Some(t), _) => format!("上限 {} tokens", widgets::format_count(t)),
                        (_, Some(r)) => format!("上限 {r} 次"),
                        _ => "不限额度".to_string(),
                    };
                    ui.label(RichText::new(limit_text).size(10.0).color(theme::muted(ui)));
                });
            });
            if let Some(limit) = key.max_tokens {
                if limit > 0 {
                    let pct = (used_tokens as f32 / limit as f32 * 100.0).min(100.0);
                    Progress::new(pct)
                        .width(ui.available_width().min(260.0))
                        .show(ui);
                }
            }
        });
    }

    fn set_key_enabled(&mut self, id: &str, enabled: bool) {
        let backup = self.config.clone();
        if let Some(key) = self.config.local_keys.iter_mut().find(|k| k.id == id) {
            key.enabled = enabled;
            if self.persist() {
                self.toast(NoticeLevel::Success, "本地 Key 状态已更新");
            } else {
                self.config = backup;
            }
        }
    }
}

enum KeyAction {
    Copy(String),
    CopyFailed,
    Test(String),
    Toggle(String, bool),
    Reset(String),
    Delete(String),
}
