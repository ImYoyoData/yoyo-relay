use egui::RichText;
use egui_phosphor::regular as icons;

use crate::snapshot::{NoticeLevel, RouteGroupView, RouteRowView};

use super::{brands, theme, widgets, App, ConfirmAction};

impl App {
    pub(crate) fn page_routes(&mut self, ui: &mut egui::Ui) {
        super::kit::page_header(
            ui,
            super::Page::Routes.eyebrow(),
            "模型路由",
            "先创建路由模型，再为它添加上游供应商与真实模型",
            |ui| {
                if super::kit::primary_labeled(ui, icons::PLUS, "新增路由模型").clicked() {
                    self.open_new_route_model();
                }
            },
        );

        if self.snapshot.route_groups.is_empty() {
            if super::kit::empty_state(
                ui,
                icons::GIT_BRANCH,
                "还没有路由模型",
                "先定义一个客户端模型（例如 gpt-5），再在它的卡片里添加上游供应商模型和降级顺序。",
                icons::PLUS,
                "创建路由模型",
            ) {
                self.open_new_route_model();
            }
            return;
        }

        let groups: Vec<RouteGroupView> = self
            .snapshot
            .route_groups
            .iter()
            .filter(|g| {
                self.matches_search(&g.client_model)
                    || g.rows.iter().any(|r| {
                        self.matches_search(&format!(
                            "{} {} {}",
                            r.provider_name, r.upstream_model, r.protocol
                        ))
                    })
            })
            .cloned()
            .collect();
        let mut pending: Vec<RouteAction> = Vec::new();
        for group in &groups {
            let matching_rows: Vec<_> = group
                .rows
                .iter()
                .filter(|r| {
                    self.matches_search(&group.client_model)
                        || self.matches_search(&format!(
                            "{} {} {}",
                            r.provider_name, r.upstream_model, r.protocol
                        ))
                })
                .cloned()
                .collect();
            if !self.matches_search(&group.client_model) && matching_rows.is_empty() {
                continue;
            }
            let profile = self.config.model_profile(&group.client_model);
            let enabled_rows = group.rows.iter().filter(|r| r.enabled).count();
            let mut meta = vec![format!(
                "{} 个上游 · {} 个参与调度",
                group.rows.len(),
                enabled_rows
            )];
            if let Some(profile) = profile {
                if let Some(context) = profile.context_window {
                    meta.push(format!("上下文 {}", widgets::format_count(context)));
                }
                if profile.input_modalities.iter().any(|m| m == "image") {
                    meta.push("图片输入".into());
                }
                if !profile.reasoning_levels.is_empty() {
                    meta.push(format!("思考 {}", profile.reasoning_levels.join("/")));
                }
            }
            if let Some(fallback) = &group.fallback_model {
                meta.push(format!("兜底 → {fallback}"));
            }
            let subtitle = meta.join(" · ");
            let client_model = group.client_model.clone();
            super::kit::section_card(
                ui,
                icons::GIT_BRANCH,
                &group.client_model,
                &subtitle,
                |ui| {
                    super::kit::badge(ui, "路由模型", theme::ACCENT);
                },
                |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if super::kit::secondary_labeled(ui, icons::PLUS, "添加上游模型").clicked()
                        {
                            pending.push(RouteAction::AddUpstream(client_model.clone()));
                        }
                        if super::kit::icon_button(ui, icons::PENCIL_SIMPLE, "编辑路由模型")
                            .clicked()
                        {
                            pending.push(RouteAction::EditModel(client_model.clone()));
                        }
                        if super::kit::danger_icon_button(
                            ui,
                            icons::TRASH_SIMPLE,
                            "删除路由模型及其上游",
                        )
                        .clicked()
                        {
                            pending.push(RouteAction::DeleteModel(client_model.clone()));
                        }
                    });
                    ui.add_space(6.0);
                    if matching_rows.is_empty() {
                        super::kit::tooltip_text(
                            ui,
                            icons::INFO,
                            "这个路由模型还没有上游模型，客户端暂时不会在 /v1/models 中看到它。",
                        );
                    }
                    let mut previous_provider: Option<String> = None;
                    for (index, row) in matching_rows.iter().enumerate() {
                        let same_provider =
                            previous_provider.as_deref() == Some(row.provider_id.as_str());
                        if same_provider {
                            ui.add_space(2.0);
                        }
                        Self::route_row(
                            ui,
                            index,
                            row,
                            same_provider,
                            &self.snapshot,
                            &mut pending,
                        );
                        previous_provider = Some(row.provider_id.clone());
                        ui.add_space(6.0);
                    }
                    for note in &group.notes {
                        super::kit::tooltip_text(ui, icons::INFO, note);
                    }
                },
            );
            ui.add_space(10.0);
        }

        for action in pending {
            match action {
                RouteAction::Edit(id) => self.open_edit_route(&id),
                RouteAction::Delete(id) => self.confirm = Some(ConfirmAction::DeleteRoute(id)),
                RouteAction::Test(id) => self.start_test("route", &id),
                RouteAction::Priority(id, delta) => self.adjust_route_priority(&id, delta),
                RouteAction::Toggle(id) => self.toggle_route_enabled(&id),
                RouteAction::AddUpstream(model) => self.open_new_route(&model),
                RouteAction::EditModel(model) => self.open_edit_route_model(&model),
                RouteAction::DeleteModel(model) => {
                    self.confirm = Some(ConfirmAction::DeleteRouteModel(model))
                }
            }
        }

        if super::kit::ghost_button(ui, icons::HARD_DRIVES, "管理供应商").clicked() {
            self.page = super::Page::Providers;
        }
    }

    fn route_row(
        ui: &mut egui::Ui,
        index: usize,
        row: &RouteRowView,
        same_provider: bool,
        snapshot: &crate::snapshot::UiSnapshot,
        pending: &mut Vec<RouteAction>,
    ) {
        let test_key = format!("route:{}", row.route_id);
        let running = snapshot.running_tests.contains(&test_key);
        let dynamic = row.route_id.starts_with("plugin:");
        egui::Frame::new()
            .fill(theme::subtle_fill(ui))
            .corner_radius(9)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if same_provider {
                        ui.add_space(18.0);
                    }
                    super::kit::badge(
                        ui,
                        &format!("{}", index + 1),
                        if index == 0 {
                            theme::ACCENT
                        } else {
                            theme::MUTED
                        },
                    );
                    brands::tile(ui, &brands::for_name(&row.provider_name), 20.0);
                    ui.label(RichText::new(&row.provider_name).size(13.5).strong());
                    ui.label(
                        RichText::new(icons::ARROW_RIGHT)
                            .size(11.0)
                            .color(theme::muted(ui)),
                    );
                    ui.label(RichText::new(&row.upstream_model).monospace().size(11.0));
                    if !row.enabled {
                        super::kit::badge(ui, "已停用", theme::MUTED);
                    }
                    if dynamic {
                        super::kit::badge(ui, "插件供应商", theme::PURPLE);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !dynamic
                            && super::kit::danger_icon_button(ui, icons::TRASH_SIMPLE, "删除路由")
                                .clicked()
                        {
                            pending.push(RouteAction::Delete(row.route_id.clone()));
                        }
                        if !dynamic
                            && super::kit::icon_button(ui, icons::PENCIL_SIMPLE, "编辑路由")
                                .clicked()
                        {
                            pending.push(RouteAction::Edit(row.route_id.clone()));
                        }
                        if !dynamic
                            && super::kit::icon_button(
                                ui,
                                if row.enabled {
                                    icons::PAUSE
                                } else {
                                    icons::PLAY
                                },
                                if row.enabled {
                                    "停用该上游（保留在列表，不参与调度）"
                                } else {
                                    "启用该上游"
                                },
                            )
                            .clicked()
                        {
                            pending.push(RouteAction::Toggle(row.route_id.clone()));
                        }
                        if !dynamic
                            && super::kit::icon_button(
                                ui,
                                if running {
                                    icons::SPINNER
                                } else {
                                    icons::FLASK
                                },
                                "测试路由",
                            )
                            .clicked()
                            && !running
                        {
                            pending.push(RouteAction::Test(row.route_id.clone()));
                        }
                        if !dynamic
                            && super::kit::icon_button(ui, icons::ARROW_UP, "提高优先级").clicked()
                        {
                            pending.push(RouteAction::Priority(row.route_id.clone(), 1));
                        }
                        if !dynamic
                            && super::kit::icon_button(ui, icons::ARROW_DOWN, "降低优先级")
                                .clicked()
                        {
                            pending.push(RouteAction::Priority(row.route_id.clone(), -1));
                        }
                        ui.label(
                            RichText::new(format!("P{}", row.priority))
                                .size(9.5)
                                .color(theme::muted(ui)),
                        );
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(if same_provider { 46.0 } else { 28.0 });
                    super::kit::badge(ui, &row.protocol, protocol_color(&row.protocol));
                    super::kit::badge(ui, &row.mode, theme::MUTED);
                    if row.accounts_available == 0 {
                        super::kit::badge(ui, "无可用账号", theme::RED);
                    } else {
                        ui.label(
                            RichText::new(format!(
                                "{}/{} 账号可用",
                                row.accounts_available, row.accounts_total
                            ))
                            .size(10.0)
                            .color(theme::muted(ui)),
                        );
                    }
                    if !row.price_label.is_empty() {
                        let mut label = row.price_label.clone();
                        if row.price_periods > 0 {
                            label.push_str(&format!(" · {} 时段", row.price_periods));
                        }
                        super::kit::badge(ui, &label, theme::PURPLE);
                    }
                    if let Some(warning) = &row.warning {
                        ui.label(
                            RichText::new(icons::WARNING)
                                .size(12.0)
                                .color(theme::ORANGE),
                        )
                        .on_hover_text(warning);
                    }
                    if !running {
                        widgets::test_badge(ui, snapshot, &test_key);
                    }
                });
            });
    }

    fn toggle_route_enabled(&mut self, id: &str) {
        let backup = self.config.clone();
        if let Some(route) = self.config.routes.iter_mut().find(|r| r.id == id) {
            route.enabled = !route.enabled;
            let enabled = route.enabled;
            if self.persist() {
                self.toast(
                    NoticeLevel::Success,
                    if enabled {
                        "该上游已启用，将参与调度"
                    } else {
                        "该上游已停用，仍保留在路由列表中"
                    },
                );
            } else {
                self.config = backup;
            }
        }
    }

    fn adjust_route_priority(&mut self, id: &str, delta: i32) {
        let backup = self.config.clone();
        if let Some(route) = self.config.routes.iter_mut().find(|r| r.id == id) {
            route.priority = (route.priority + delta).clamp(-100, 100);
            if self.persist() {
                self.toast(NoticeLevel::Success, "优先级已更新");
            } else {
                self.config = backup;
            }
        }
    }
}

fn protocol_color(protocol: &str) -> egui::Color32 {
    match protocol {
        "anthropic-messages" => egui::Color32::from_rgb(214, 137, 89),
        "openai-responses" => egui::Color32::from_rgb(122, 180, 240),
        _ => theme::GREEN,
    }
}

enum RouteAction {
    Edit(String),
    Delete(String),
    Test(String),
    Priority(String, i32),
    Toggle(String),
    AddUpstream(String),
    EditModel(String),
    DeleteModel(String),
}
