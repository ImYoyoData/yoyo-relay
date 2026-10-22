use egui::RichText;
use egui_phosphor::regular as icons;

use crate::snapshot::{NoticeLevel, RouteGroupView, RouteRowView};

use super::{theme, widgets, App, ConfirmAction};

impl App {
    pub(crate) fn page_routes(&mut self, ui: &mut egui::Ui) {
        super::kit::page_header(
            ui,
            super::Page::Routes.eyebrow(),
            "模型路由",
            "客户端模型 → 上游模型的候选链与降级顺序",
            |ui| {
                if super::kit::primary_labeled(ui, icons::PLUS, "新增路由").clicked() {
                    self.open_new_route();
                }
            },
        );

        if self.snapshot.route_groups.is_empty() {
            if super::kit::empty_state(
                ui,
                icons::GIT_BRANCH,
                "还没有模型路由",
                "路由把客户端请求的模型名映射到上游供应商与真实模型，并定义降级顺序。",
                icons::PLUS,
                "添加路由",
            ) {
                self.open_new_route();
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
            if matching_rows.is_empty() {
                continue;
            }
            let subtitle = match &group.fallback_model {
                Some(fallback) => {
                    format!("{} 条候选 · 映射级兜底 → {}", matching_rows.len(), fallback)
                }
                None => format!("{} 条候选路由", matching_rows.len()),
            };
            super::kit::section_card(
                ui,
                icons::GIT_BRANCH,
                &group.client_model,
                &subtitle,
                |ui| {
                    super::kit::badge(ui, "降级链", theme::ACCENT);
                },
                |ui| {
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
            }
        }

        if ui.small_button("管理供应商").clicked() {
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
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if super::kit::danger_icon_button(ui, icons::TRASH_SIMPLE, "删除路由")
                            .clicked()
                        {
                            pending.push(RouteAction::Delete(row.route_id.clone()));
                        }
                        if super::kit::icon_button(ui, icons::PENCIL_SIMPLE, "编辑路由").clicked()
                        {
                            pending.push(RouteAction::Edit(row.route_id.clone()));
                        }
                        if super::kit::icon_button(
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
                        if super::kit::icon_button(ui, icons::ARROW_UP, "提高优先级").clicked()
                        {
                            pending.push(RouteAction::Priority(row.route_id.clone(), 1));
                        }
                        if super::kit::icon_button(ui, icons::ARROW_DOWN, "降低优先级").clicked()
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
}
