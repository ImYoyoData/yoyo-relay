use egui::{
    Align, Align2, Color32, FontId, Layout, Pos2, Rect, Response, RichText, Sense, Shape, Stroke,
    Ui, Vec2,
};
use egui_phosphor::regular as icons;

use super::theme;
use super::widgets;

// ---------------------------------------------------------------------------
// Surfaces
// ---------------------------------------------------------------------------

pub fn surface(ui: &mut Ui, body: impl FnOnce(&mut Ui)) {
    let t = theme::tokens(ui);
    egui::Frame::new()
        .fill(t.card)
        .stroke(Stroke::new(1.0_f32, t.card_border))
        .corner_radius(14)
        .shadow(t.card_shadow)
        .inner_margin(egui::Margin::symmetric(18, 16))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            body(ui);
        });
}

pub fn section_card(
    ui: &mut Ui,
    icon: &str,
    title: &str,
    subtitle: &str,
    actions: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui),
) {
    surface(ui, |ui| {
        card_header(ui, icon, title, subtitle, actions);
        body(ui);
    });
}

pub fn card_header(
    ui: &mut Ui,
    icon: &str,
    title: &str,
    subtitle: &str,
    actions: impl FnOnce(&mut Ui),
) {
    let t = theme::tokens(ui);
    ui.horizontal(|ui| {
        icon_chip(ui, icon, t.primary, 32.0);
        ui.add_space(2.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(14.0).strong().color(t.text));
            if !subtitle.is_empty() {
                ui.label(RichText::new(subtitle).size(10.5).color(t.muted));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.set_height(30.0);
            actions(ui);
        });
    });
    ui.add_space(12.0);
    let rect = ui.available_rect_before_wrap();
    ui.painter().line_segment(
        [
            Pos2::new(rect.left(), rect.top()),
            Pos2::new(rect.right(), rect.top()),
        ],
        Stroke::new(1.0_f32, t.card_border),
    );
    ui.add_space(12.0);
}

pub fn soft_panel(ui: &mut Ui, body: impl FnOnce(&mut Ui)) {
    let t = theme::tokens(ui);
    egui::Frame::new()
        .fill(if t.dark_mode {
            t.hover
        } else {
            Color32::from_rgb(248, 250, 253)
        })
        .stroke(Stroke::new(1.0_f32, t.card_border))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, body);
}

pub fn divider(ui: &mut Ui) {
    let t = theme::tokens(ui);
    let rect = ui.available_rect_before_wrap();
    ui.painter().line_segment(
        [
            Pos2::new(rect.left(), rect.top()),
            Pos2::new(rect.right(), rect.top()),
        ],
        Stroke::new(1.0_f32, t.card_border),
    );
    ui.add_space(6.0);
}

// ---------------------------------------------------------------------------
// Page header
// ---------------------------------------------------------------------------

pub fn page_header(
    ui: &mut Ui,
    eyebrow: &str,
    title: &str,
    subtitle: &str,
    actions: impl FnOnce(&mut Ui),
) {
    let t = theme::tokens(ui);
    let narrow = ui.available_width() < 760.0;
    if narrow {
        eyebrow_line(ui, eyebrow, t.primary, t.muted);
        ui.add_space(4.0);
        ui.label(RichText::new(title).size(24.0).strong().color(t.text));
        ui.add_space(2.0);
        ui.label(RichText::new(subtitle).size(11.5).color(t.muted));
        ui.add_space(10.0);
        ui.horizontal_wrapped(actions);
    } else {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                eyebrow_line(ui, eyebrow, t.primary, t.muted);
                ui.add_space(4.0);
                ui.label(RichText::new(title).size(24.0).strong().color(t.text));
                ui.add_space(2.0);
                ui.label(RichText::new(subtitle).size(11.5).color(t.muted));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.set_height(34.0);
                actions(ui);
            });
        });
    }
    ui.add_space(16.0);
}

fn eyebrow_line(ui: &mut Ui, eyebrow: &str, primary: Color32, muted: Color32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("•").size(11.0).color(primary));
        ui.label(
            RichText::new(eyebrow.to_uppercase())
                .size(9.5)
                .strong()
                .color(primary),
        );
        let _ = muted;
    });
}

// ---------------------------------------------------------------------------
// Icons and chips
// ---------------------------------------------------------------------------

pub fn icon_chip(ui: &mut Ui, icon: &str, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let bg = theme::soft_for(ui, color);
    ui.painter().rect_filled(rect, size * 0.26, bg);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(size * 0.46),
        color,
    );
}

pub fn status_pill(ui: &mut Ui, text: &str, color: Color32) {
    let bg = theme::soft_for(ui, color);
    let font = FontId::proportional(10.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let size = Vec2::new(galley.size().x + 24.0, 19.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 999.0, bg);
    ui.painter()
        .circle_filled(Pos2::new(rect.left() + 8.0, rect.center().y), 3.0, color);
    ui.painter().galley(
        Pos2::new(rect.left() + 15.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
}

pub fn status_badge(ui: &mut Ui, text: &str, color: Color32) {
    status_pill(ui, text, color);
}

pub fn badge(ui: &mut Ui, text: &str, color: Color32) {
    let bg = theme::soft_for(ui, color);
    let font = FontId::proportional(9.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let size = Vec2::new(galley.size().x + 14.0, 18.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 5.0, bg);
    ui.painter().galley(
        Pos2::new(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        color,
    );
}

pub fn plain_badge(ui: &mut Ui, text: &str) {
    let t = theme::tokens(ui);
    let font = FontId::proportional(9.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, t.muted);
    let size = Vec2::new(galley.size().x + 14.0, 18.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 5.0, t.hover);
    ui.painter().galley(
        Pos2::new(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        t.muted,
    );
}

pub fn label(ui: &mut Ui, text: &str) {
    let t = theme::tokens(ui);
    ui.label(RichText::new(text).size(10.5).strong().color(t.muted));
}

#[allow(dead_code)]
pub fn muted_text(ui: &mut Ui, text: &str) {
    let t = theme::tokens(ui);
    ui.label(RichText::new(text).size(10.5).color(t.muted));
}

pub fn info_row(ui: &mut Ui, icon: &str, text: &str) {
    let t = theme::tokens(ui);
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon).size(12.0).color(t.muted));
        ui.label(RichText::new(text).size(10.5).color(t.muted));
    });
}

pub fn kv(ui: &mut Ui, key: &str, value: &str) {
    let t = theme::tokens(ui);
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).size(10.0).color(t.muted));
        ui.label(RichText::new(value).size(11.0).strong().color(t.text));
    });
}

// ---------------------------------------------------------------------------
// Buttons
// ---------------------------------------------------------------------------

fn button_text(icon: &str, text: &str) -> String {
    if icon.is_empty() {
        text.to_string()
    } else {
        format!("{icon}  {text}")
    }
}

pub fn primary_button(ui: &mut Ui, icon: &str, text: &str) -> Response {
    let t = theme::tokens(ui);
    ui.add(
        egui::Button::new(
            RichText::new(button_text(icon, text))
                .size(12.5)
                .color(Color32::WHITE),
        )
        .fill(t.primary)
        .stroke(Stroke::NONE)
        .corner_radius(8)
        .min_size(Vec2::new(0.0, 32.0)),
    )
}

pub fn secondary_button(ui: &mut Ui, icon: &str, text: &str) -> Response {
    let t = theme::tokens(ui);
    ui.add(
        egui::Button::new(
            RichText::new(button_text(icon, text))
                .size(12.5)
                .color(t.text),
        )
        .fill(t.card)
        .stroke(Stroke::new(1.0_f32, t.card_border))
        .corner_radius(8)
        .min_size(Vec2::new(0.0, 32.0)),
    )
}

pub fn ghost_button(ui: &mut Ui, icon: &str, text: &str) -> Response {
    let t = theme::tokens(ui);
    ui.add(
        egui::Button::new(
            RichText::new(button_text(icon, text))
                .size(11.5)
                .color(t.muted),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
        .corner_radius(8),
    )
}

pub fn primary_labeled(ui: &mut Ui, icon: &str, text: &str) -> Response {
    primary_button(ui, icon, text)
}

pub fn secondary_labeled(ui: &mut Ui, icon: &str, text: &str) -> Response {
    secondary_button(ui, icon, text)
}

pub fn ghost_labeled(ui: &mut Ui, icon: &str, text: &str) -> Response {
    ghost_button(ui, icon, text)
}

pub fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str) -> Response {
    let t = theme::tokens(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let bg = if response.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 7.0, bg);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(15.0),
        if response.hovered() { t.text } else { t.muted },
    );
    response.on_hover_text(tooltip)
}

pub fn danger_icon_button(ui: &mut Ui, icon: &str, tooltip: &str) -> Response {
    let t = theme::tokens(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let bg = if response.hovered() {
        t.red_soft
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 7.0, bg);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(15.0),
        if response.hovered() { t.red } else { t.muted },
    );
    response.on_hover_text(tooltip)
}

pub fn window_button(ui: &mut Ui, icon: &str, tooltip: &str, danger: bool) -> Response {
    let t = theme::tokens(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(36.0, 26.0), Sense::click());
    let bg = if response.hovered() {
        if danger {
            t.red
        } else {
            t.hover
        }
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 6.0, bg);
    let color = if response.hovered() && danger {
        Color32::WHITE
    } else {
        t.muted
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(12.5),
        color,
    );
    response.on_hover_text(tooltip)
}

// ---------------------------------------------------------------------------
// KPI cards
// ---------------------------------------------------------------------------

pub fn kpi_grid(ui: &mut Ui, cards: &[(&str, &str, String, String, Color32)]) {
    let avail = ui.available_width();
    let min_card = 168.0;
    let gap = 12.0;
    let per_row: usize = if avail >= min_card * 4.0 + gap * 3.0 {
        4
    } else if avail >= min_card * 2.0 + gap {
        2
    } else {
        1
    };
    let width = ((avail - gap * (per_row.saturating_sub(1)) as f32) / per_row as f32).max(120.0);
    let total_rows = cards.len().div_ceil(per_row);
    for (row_index, chunk) in cards.chunks(per_row).enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (icon, label, value, sub, color) in chunk {
                ui.allocate_ui_with_layout(
                    Vec2::new(width, 106.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(width);
                        kpi_card(ui, icon, label, value, sub, *color);
                    },
                );
            }
        });
        if row_index + 1 < total_rows {
            ui.add_space(10.0);
        }
    }
}

pub fn kpi_card(ui: &mut Ui, icon: &str, label: &str, value: &str, sub: &str, color: Color32) {
    let t = theme::tokens(ui);
    egui::Frame::new()
        .fill(t.card)
        .stroke(Stroke::new(1.0_f32, t.card_border))
        .corner_radius(14)
        .shadow(t.card_shadow)
        .inner_margin(egui::Margin::symmetric(16, 13))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(label).size(10.5).color(t.muted));
                    ui.add_space(1.0);
                    ui.label(RichText::new(value).size(25.0).strong().color(t.text));
                });
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    icon_chip(ui, icon, color, 33.0);
                });
            });
            ui.add_space(2.0);
            ui.label(RichText::new(sub).size(9.5).color(t.muted));
        });
}

pub fn kpi_cards_from_snapshot(ui: &mut Ui, snapshot: &crate::snapshot::UiSnapshot) {
    let today = &snapshot.today.totals;
    let success = if today.requests == 0 {
        "—".to_string()
    } else {
        format!("{:.1}%", today.success_rate() * 100.0)
    };
    let t = theme::tokens(ui);
    let cards: Vec<(&str, &str, String, String, Color32)> = vec![
        (
            icons::PAPER_PLANE_TILT,
            "今日请求",
            widgets::format_count(today.requests),
            format!("成功 {}", widgets::format_count(today.ok)),
            t.primary,
        ),
        (
            icons::COINS,
            "今日 Token",
            widgets::format_count(today.total_tokens),
            format!(
                "输入 {} · 输出 {}",
                widgets::format_count(today.prompt_tokens),
                widgets::format_count(today.completion_tokens)
            ),
            t.blue,
        ),
        (
            icons::CHECK_CIRCLE,
            "成功率",
            success,
            format!("平均延迟 {} ms", today.avg_latency_ms),
            if today.requests > 0 && today.success_rate() >= 0.95 {
                t.green
            } else {
                t.amber
            },
        ),
        (
            icons::CURRENCY_DOLLAR,
            "今日预估花费",
            widgets::format_cost(today.cost_micros),
            "按路由价格估算".to_string(),
            t.amber,
        ),
    ];
    kpi_grid(ui, &cards);
}

pub fn empty_state(
    ui: &mut Ui,
    icon: &str,
    title: &str,
    subtitle: &str,
    action_icon: &str,
    action: &str,
) -> bool {
    let t = theme::tokens(ui);
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        ui.add_space(28.0);
        icon_chip(ui, icon, t.primary, 52.0);
        ui.add_space(8.0);
        ui.label(RichText::new(title).size(15.0).strong().color(t.text));
        ui.add_space(2.0);
        ui.label(RichText::new(subtitle).size(11.5).color(t.muted));
        ui.add_space(12.0);
        clicked = primary_button(ui, action_icon, action).clicked();
        ui.add_space(24.0);
    });
    clicked
}

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

pub fn nav_item(
    ui: &mut Ui,
    icon: &str,
    text: &str,
    active: bool,
    count: Option<String>,
) -> Response {
    let t = theme::tokens(ui);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
    let bg = if active {
        t.primary_soft
    } else if response.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 8.0, bg);
    if active {
        ui.painter().rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.left(), rect.top() + 8.0),
                Vec2::new(3.0, rect.height() - 16.0),
            ),
            2.0,
            t.primary,
        );
    }
    let color = if active { t.primary } else { t.text };
    ui.painter().text(
        Pos2::new(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        icon,
        FontId::proportional(15.0),
        color,
    );
    ui.painter().text(
        Pos2::new(rect.left() + 38.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(12.5),
        color,
    );
    if let Some(count) = count {
        ui.painter().text(
            Pos2::new(rect.right() - 12.0, rect.center().y),
            Align2::RIGHT_CENTER,
            count,
            FontId::proportional(10.0),
            t.muted,
        );
    }
    response
}

pub fn nav_rail_item(ui: &mut Ui, icon: &str, tooltip: &str, active: bool) -> Response {
    let t = theme::tokens(ui);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::click());
    let bg = if active {
        t.primary_soft
    } else if response.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 8.0, bg);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(16.0),
        if active { t.primary } else { t.muted },
    );
    response.on_hover_text(tooltip)
}

pub fn nav_label(ui: &mut Ui, text: &str) {
    let t = theme::tokens(ui);
    ui.add_space(8.0);
    ui.label(RichText::new(text).size(9.5).strong().color(t.muted));
    ui.add_space(2.0);
}

// ---------------------------------------------------------------------------
// Modal
// ---------------------------------------------------------------------------

pub fn modal_shell(
    ctx: &egui::Context,
    id: egui::Id,
    title: &str,
    subtitle: &str,
    width: f32,
    body: impl FnOnce(&mut Ui),
    footer: impl FnOnce(&mut Ui),
) -> bool {
    let t = theme::tokens_ctx(ctx);
    let screen = ctx.content_rect();
    let width = width.min(screen.width() - 48.0).max(320.0);
    let max_body = (screen.height() * 0.66).max(180.0);
    let mut x_clicked = false;
    let modal = egui::containers::Modal::new(id)
        .backdrop_color(Color32::from_black_alpha(if t.dark_mode {
            150
        } else {
            110
        }))
        .frame(
            egui::Frame::new()
                .fill(t.card)
                .stroke(Stroke::new(1.0_f32, t.card_border))
                .corner_radius(16)
                .shadow(egui::epaint::Shadow {
                    offset: [0, 18],
                    blur: 44,
                    spread: 2,
                    color: Color32::from_rgba_unmultiplied(
                        15,
                        23,
                        42,
                        if t.dark_mode { 160 } else { 70 },
                    ),
                })
                .inner_margin(egui::Margin::ZERO),
        );
    let response = modal.show(ctx, |ui| {
        ui.set_width(width);
        let pad = 20.0;
        let inner_width = width - pad * 2.0;
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(pad as i8, 16))
            .show(ui, |ui| {
                ui.set_width(inner_width);
                ui.horizontal(|ui| {
                    icon_chip(ui, icons::INFO, t.primary, 32.0);
                    ui.add_space(2.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(title).size(17.0).strong().color(t.text));
                        if !subtitle.is_empty() {
                            ui.label(RichText::new(subtitle).size(11.0).color(t.muted));
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, icons::X, "关闭").clicked() {
                            x_clicked = true;
                        }
                    });
                });
            });
        let rect = ui.available_rect_before_wrap();
        ui.painter().line_segment(
            [
                Pos2::new(rect.left(), rect.top()),
                Pos2::new(rect.right(), rect.top()),
            ],
            Stroke::new(1.0_f32, t.card_border),
        );
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(pad as i8, 16))
            .show(ui, |ui| {
                ui.set_width(inner_width);
                egui::ScrollArea::vertical()
                    .max_height(max_body)
                    .id_salt(id.with("body"))
                    .show(ui, body);
            });
        let rect = ui.available_rect_before_wrap();
        ui.painter().line_segment(
            [
                Pos2::new(rect.left(), rect.top()),
                Pos2::new(rect.right(), rect.top()),
            ],
            Stroke::new(1.0_f32, t.card_border),
        );
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(pad as i8, 12))
            .show(ui, |ui| {
                ui.set_width(inner_width);
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), footer);
                });
            });
    });
    x_clicked || response.should_close()
}

// ---------------------------------------------------------------------------
// Interactive chart
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartMetric {
    Tokens,
    Requests,
    Cost,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ChartPoint {
    pub bucket: i64,
    pub tokens: u64,
    pub requests: u64,
    pub cost_micros: u64,
}

pub fn filled_chart_points(
    series: &[crate::db::SeriesPoint],
    start: i64,
    end: i64,
    bucket_secs: i64,
) -> Vec<ChartPoint> {
    let bucket = bucket_secs.max(60);
    let mut first = start;
    let mut out = Vec::new();
    let end = end.max(start);
    while first <= end {
        out.push(ChartPoint {
            bucket: first,
            ..ChartPoint::default()
        });
        first += bucket;
        if out.len() > 400 {
            break;
        }
    }
    for point in series {
        let index = ((point.bucket - start) / bucket).max(0) as usize;
        if let Some(slot) = out.get_mut(index) {
            slot.tokens = point.totals.total_tokens;
            slot.requests = point.totals.requests;
            slot.cost_micros = point.totals.cost_micros;
        }
    }
    out
}

fn metric_value(point: &ChartPoint, metric: ChartMetric) -> f64 {
    match metric {
        ChartMetric::Tokens => point.tokens as f64,
        ChartMetric::Requests => point.requests as f64,
        ChartMetric::Cost => point.cost_micros as f64,
    }
}

fn metric_short(value: f64, metric: ChartMetric) -> String {
    match metric {
        ChartMetric::Tokens => widgets::format_count(value as u64),
        ChartMetric::Requests => format!("{}", value as u64),
        ChartMetric::Cost => widgets::format_cost(value as u64),
    }
}

fn metric_text(value: f64, metric: ChartMetric) -> String {
    match metric {
        ChartMetric::Tokens => format!("{} tokens", widgets::format_count(value as u64)),
        ChartMetric::Requests => format!("{} 次请求", value as u64),
        ChartMetric::Cost => widgets::format_cost(value as u64),
    }
}

pub fn interactive_chart(
    ui: &mut Ui,
    source: &[ChartPoint],
    color: Color32,
    height: f32,
    metric: ChartMetric,
    bucket_secs: i64,
) -> Option<usize> {
    let points: Vec<(i64, f64)> = source
        .iter()
        .map(|point| (point.bucket, metric_value(point, metric)))
        .collect();
    let points = points.as_slice();
    let t = theme::tokens(ui);
    let width = ui.available_width().max(280.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        10.0,
        if t.dark_mode {
            t.hover
        } else {
            Color32::from_rgb(249, 251, 254)
        },
    );

    let plot = Rect::from_min_max(
        Pos2::new(rect.left() + 62.0, rect.top() + 18.0),
        Pos2::new(rect.right() - 16.0, rect.bottom() - 30.0),
    );
    let max = points.iter().map(|p| p.1).fold(0.0_f64, f64::max).max(1.0);
    let nice_max = nice_ceiling(max);
    for i in 0..=4 {
        let y = plot.top() + plot.height() * (i as f32 / 4.0);
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0_f32, t.card_border.gamma_multiply(0.8)),
        );
        let value = nice_max * (1.0 - i as f64 / 4.0);
        painter.text(
            Pos2::new(plot.left() - 10.0, y),
            Align2::RIGHT_CENTER,
            metric_short(value, metric),
            FontId::proportional(9.0),
            t.muted,
        );
    }
    if points.is_empty() {
        painter.text(
            plot.center(),
            Align2::CENTER_CENTER,
            "暂无数据",
            FontId::proportional(12.0),
            t.muted,
        );
        return None;
    }
    painter.text(
        Pos2::new(plot.right(), plot.top() - 6.0),
        Align2::RIGHT_BOTTOM,
        "悬停查看明细",
        FontId::proportional(9.0),
        t.muted,
    );
    let n = points.len();
    let x_at = |i: usize| {
        if n <= 1 {
            plot.center().x
        } else {
            plot.left() + plot.width() * (i as f32 / (n - 1) as f32)
        }
    };
    let y_at = |v: f64| plot.bottom() - (v / nice_max).clamp(0.0, 1.0) as f32 * plot.height();
    let pts: Vec<Pos2> = points
        .iter()
        .enumerate()
        .map(|(i, (_, v))| Pos2::new(x_at(i), y_at(*v)))
        .collect();

    let hover_index = response.hover_pos().and_then(|pos| {
        if !plot.expand2(Vec2::new(0.0, 8.0)).contains(pos) {
            return None;
        }
        let ratio = ((pos.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
        Some(((ratio * (n.saturating_sub(1)) as f32).round() as usize).min(n - 1))
    });
    let nonzero = points.iter().filter(|p| p.1 > 0.0).count();
    let use_bars = bucket_secs >= 86_400 || nonzero <= 2;
    if use_bars {
        let slot = if n <= 1 {
            plot.width() * 0.5
        } else {
            plot.width() / n as f32
        };
        let bar_w = (slot * 0.52).clamp(3.0, 22.0);
        for (index, value) in points.iter().enumerate() {
            let x = x_at(index);
            let y = y_at(value.1);
            let bar = Rect::from_min_max(
                Pos2::new(x - bar_w / 2.0, y),
                Pos2::new(x + bar_w / 2.0, plot.bottom()),
            );
            let hovered = hover_index == Some(index);
            painter.rect_filled(
                bar,
                egui::CornerRadius::same(3),
                color.gamma_multiply(if hovered { 0.95 } else { 0.55 }),
            );
        }
    } else {
        let mut area = pts.clone();
        area.push(Pos2::new(x_at(n - 1), plot.bottom()));
        area.push(Pos2::new(x_at(0), plot.bottom()));
        painter.add(Shape::convex_polygon(
            area,
            color.gamma_multiply(if t.dark_mode { 0.20 } else { 0.12 }),
            Stroke::NONE,
        ));
        if pts.len() == 1 {
            painter.circle_filled(pts[0], 3.0, color);
        } else {
            for pair in pts.windows(2) {
                painter.line_segment([pair[0], pair[1]], Stroke::new(2.0_f32, color));
            }
        }
    }

    let fmt_time = |ts: i64| {
        use chrono::TimeZone;
        chrono::Local
            .timestamp_opt(ts, 0)
            .single()
            .map(|d| {
                if bucket_secs < 86_400 {
                    d.format("%H:%M").to_string()
                } else {
                    d.format("%m-%d").to_string()
                }
            })
            .unwrap_or_else(|| "-".into())
    };
    painter.text(
        Pos2::new(plot.left(), rect.bottom() - 12.0),
        Align2::LEFT_CENTER,
        fmt_time(points[0].0),
        FontId::proportional(9.5),
        t.muted,
    );
    painter.text(
        Pos2::new(plot.center().x, rect.bottom() - 12.0),
        Align2::CENTER_CENTER,
        fmt_time(points[n / 2].0),
        FontId::proportional(9.5),
        t.muted,
    );
    if n > 1 {
        painter.text(
            Pos2::new(plot.right(), rect.bottom() - 12.0),
            Align2::RIGHT_CENTER,
            fmt_time(points[n - 1].0),
            FontId::proportional(9.5),
            t.muted,
        );
    }

    if let Some(index) = hover_index {
        if !use_bars {
            painter.circle_filled(pts[index], 4.5, color);
            painter.circle_stroke(
                pts[index],
                7.5,
                Stroke::new(2.0_f32, color.gamma_multiply(0.3)),
            );
        }
        let x = x_at(index);
        painter.line_segment(
            [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
            Stroke::new(1.0_f32, color.gamma_multiply(0.45)),
        );
        let text = format!(
            "{}   {}",
            fmt_time(points[index].0),
            metric_text(points[index].1, metric)
        );
        let galley = painter.layout_no_wrap(
            text,
            FontId::proportional(10.5),
            if t.dark_mode {
                t.text
            } else {
                Color32::from_rgb(30, 41, 59)
            },
        );
        let box_size = galley.size() + Vec2::new(16.0, 10.0);
        let mut box_min = Pos2::new(x + 10.0, pts[index].y - box_size.y - 10.0);
        if box_min.x + box_size.x > rect.right() {
            box_min.x = x - box_size.x - 10.0;
        }
        box_min.x = box_min.x.max(rect.left() + 4.0);
        box_min.y = box_min.y.max(rect.top() + 4.0);
        let box_rect = Rect::from_min_size(box_min, box_size);
        painter.rect_filled(
            box_rect,
            7.0,
            if t.dark_mode {
                Color32::from_rgb(28, 34, 44)
            } else {
                Color32::WHITE
            },
        );
        painter.rect_stroke(
            box_rect,
            7.0,
            Stroke::new(1.0_f32, color.gamma_multiply(0.35)),
            egui::StrokeKind::Inside,
        );
        painter.galley(box_rect.min + Vec2::new(8.0, 5.0), galley, t.text);
    }

    response.clicked().then_some(hover_index).flatten()
}

fn nice_ceiling(value: f64) -> f64 {
    if value <= 0.0 {
        return 1.0;
    }
    let exp = value.log10().floor();
    let base = 10f64.powf(exp);
    let norm = value / base;
    let nice = if norm <= 1.0 {
        1.0
    } else if norm <= 2.0 {
        2.0
    } else if norm <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * base
}

// ---------------------------------------------------------------------------
// Segmented control
// ---------------------------------------------------------------------------

pub fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    options: &[(T, &str)],
    selected: T,
) -> Option<T> {
    let t = theme::tokens(ui);
    let mut changed = None;
    egui::Frame::new()
        .fill(t.hover)
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(3, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (value, label) in options {
                    let active = *value == selected;
                    let text = RichText::new(*label).size(11.0).color(if active {
                        t.text
                    } else {
                        t.muted
                    });
                    let button = egui::Button::new(text)
                        .fill(if active { t.card } else { Color32::TRANSPARENT })
                        .stroke(if active {
                            Stroke::new(1.0_f32, t.card_border)
                        } else {
                            Stroke::NONE
                        })
                        .corner_radius(6);
                    if ui.add(button).clicked() && !active {
                        changed = Some(*value);
                    }
                }
            });
        });
    changed
}

// Backwards-compatible aliases used across pages.
pub fn tooltip_text(ui: &mut Ui, icon: &str, text: &str) {
    info_row(ui, icon, text);
}

pub fn icon_tile(ui: &mut Ui, icon: &str, color: Color32, size: f32) {
    icon_chip(ui, icon, color, size);
}

pub fn field_label(ui: &mut Ui, text: &str) {
    label(ui, text);
}

pub fn info_line(ui: &mut Ui, key: &str, value: &str) {
    kv(ui, key, value);
}

pub fn section_label(ui: &mut Ui, text: &str) {
    let t = theme::tokens(ui);
    ui.label(RichText::new(text).size(10.0).strong().color(t.muted));
}

pub fn labeled_button(
    ui: &mut Ui,
    icon: &str,
    text: &str,
    variant: armas::ButtonVariant,
    size: armas::ButtonSize,
) -> Response {
    let t = theme::tokens(ui);
    let color = match variant {
        armas::ButtonVariant::Default => Color32::WHITE,
        armas::ButtonVariant::Outline => t.text,
        _ => t.muted,
    };
    let mut body = egui::Button::new(
        RichText::new(button_text(icon, text))
            .size(if size == armas::ButtonSize::Small {
                12.0
            } else {
                12.5
            })
            .color(color),
    )
    .corner_radius(8)
    .min_size(Vec2::new(
        0.0,
        if size == armas::ButtonSize::Small {
            28.0
        } else {
            32.0
        },
    ));
    body = match variant {
        armas::ButtonVariant::Default => body.fill(t.primary).stroke(Stroke::NONE),
        armas::ButtonVariant::Outline => body
            .fill(t.card)
            .stroke(Stroke::new(1.0_f32, t.card_border)),
        _ => body.fill(Color32::TRANSPARENT).stroke(Stroke::NONE),
    };
    ui.add(body)
}

pub fn danger_button(ui: &mut Ui, icon: &str, text: &str) -> Response {
    let t = theme::tokens(ui);
    ui.add(
        egui::Button::new(
            RichText::new(button_text(icon, text))
                .size(12.5)
                .color(Color32::WHITE),
        )
        .fill(t.red)
        .stroke(Stroke::NONE)
        .corner_radius(8)
        .min_size(Vec2::new(0.0, 32.0)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{SeriesPoint, Totals};

    #[test]
    fn fills_chart_gaps_with_zeroes() {
        let series = vec![
            SeriesPoint {
                bucket: 0,
                totals: Totals {
                    requests: 2,
                    total_tokens: 5,
                    cost_micros: 7,
                    ..Totals::default()
                },
            },
            SeriesPoint {
                bucket: 7_200,
                totals: Totals {
                    requests: 1,
                    total_tokens: 9,
                    cost_micros: 3,
                    ..Totals::default()
                },
            },
        ];
        let points = filled_chart_points(&series, 0, 7_200, 3_600);
        assert_eq!(points.len(), 3);
        assert_eq!(points[0].tokens, 5);
        assert_eq!(points[1].tokens, 0);
        assert_eq!(points[2].tokens, 9);
        assert_eq!(points[2].requests, 1);
        assert_eq!(points[2].cost_micros, 3);
    }
}

pub fn brand_logo(ui: &mut Ui, texture: &egui::TextureHandle, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
}

pub fn toggle_switch(ui: &mut Ui, value: &mut bool) -> Response {
    let t = theme::tokens(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(40.0, 22.0), Sense::click());
    if response.clicked() {
        *value = !*value;
    }
    let on = *value;
    let bg = if on {
        t.primary
    } else if t.dark_mode {
        Color32::from_rgb(55, 62, 76)
    } else {
        Color32::from_rgb(203, 210, 222)
    };
    ui.painter().rect_filled(rect, 11.0, bg);
    let cx = if on {
        rect.right() - 11.0
    } else {
        rect.left() + 11.0
    };
    ui.painter()
        .circle_filled(Pos2::new(cx, rect.center().y), 8.0, Color32::WHITE);
    response
}
