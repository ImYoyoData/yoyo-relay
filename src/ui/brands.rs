//! Brand marks for upstream providers.
//!
//! YoyoRelay ships no remote artwork, so each vendor gets a deterministic
//! painted mark: a brand-colored rounded tile with a monogram. Vendors with a
//! Phosphor glyph (OpenAI) use the real glyph. The mark is used consistently on
//! the provider page, the provider editor and the route editor.

use egui::{Align2, Color32, FontId, Pos2, Rect, Response, RichText, Sense, Stroke, Ui, Vec2};
use egui_phosphor::regular as icons;

use crate::config::{presets, Provider};

use super::theme;

#[derive(Clone, Copy, Debug)]
pub struct Brand {
    #[allow(dead_code)]
    pub key: &'static str,
    pub label: &'static str,
    pub glyph: &'static str,
    pub bg: Color32,
    pub fg: Color32,
    /// Glyph font scale relative to the tile size.
    pub glyph_scale: f32,
}

const WHITE: Color32 = Color32::WHITE;

pub fn for_preset(key: Option<&str>) -> Brand {
    let brand = match key.unwrap_or("custom") {
        "openai" => Brand {
            key: "openai",
            label: "OpenAI",
            glyph: icons::OPEN_AI_LOGO,
            bg: Color32::from_rgb(16, 163, 127),
            fg: WHITE,
            glyph_scale: 0.56,
        },
        "anthropic" => Brand {
            key: "anthropic",
            label: "Anthropic",
            glyph: "A",
            bg: Color32::from_rgb(217, 119, 87),
            fg: WHITE,
            glyph_scale: 0.50,
        },
        "command_code" => Brand {
            key: "command_code",
            label: "Command Code",
            glyph: "CC",
            bg: Color32::from_rgb(30, 41, 59),
            fg: WHITE,
            glyph_scale: 0.36,
        },
        "opencode_zen" => Brand {
            key: "opencode_zen",
            label: "OpenCode Zen",
            glyph: "OZ",
            bg: Color32::from_rgb(15, 23, 42),
            fg: WHITE,
            glyph_scale: 0.36,
        },
        "opencode_go" => Brand {
            key: "opencode_go",
            label: "OpenCode Go",
            glyph: "OG",
            bg: Color32::from_rgb(4, 120, 87),
            fg: WHITE,
            glyph_scale: 0.36,
        },
        "zhipu" => Brand {
            key: "zhipu",
            label: "智谱 AI",
            glyph: "智",
            bg: Color32::from_rgb(37, 99, 235),
            fg: WHITE,
            glyph_scale: 0.48,
        },
        "xiaomi" => Brand {
            key: "xiaomi",
            label: "小米 MiMo",
            glyph: "mi",
            bg: Color32::from_rgb(255, 105, 0),
            fg: WHITE,
            glyph_scale: 0.44,
        },
        "minimax" => Brand {
            key: "minimax",
            label: "MiniMax",
            glyph: "M",
            bg: Color32::from_rgb(225, 29, 72),
            fg: WHITE,
            glyph_scale: 0.50,
        },
        "doubao" => Brand {
            key: "doubao",
            label: "豆包",
            glyph: "豆",
            bg: Color32::from_rgb(59, 130, 246),
            fg: WHITE,
            glyph_scale: 0.48,
        },
        "qwen" => Brand {
            key: "qwen",
            label: "通义千问",
            glyph: "Q",
            bg: Color32::from_rgb(109, 40, 217),
            fg: WHITE,
            glyph_scale: 0.50,
        },
        "hunyuan" => Brand {
            key: "hunyuan",
            label: "腾讯混元",
            glyph: "混",
            bg: Color32::from_rgb(14, 165, 233),
            fg: WHITE,
            glyph_scale: 0.48,
        },
        "stepfun" => Brand {
            key: "stepfun",
            label: "阶跃星辰",
            glyph: "阶",
            bg: Color32::from_rgb(124, 58, 237),
            fg: WHITE,
            glyph_scale: 0.48,
        },
        _ => Brand {
            key: "custom",
            label: "自定义端点",
            glyph: icons::PLUGS_CONNECTED,
            bg: Color32::from_rgb(100, 116, 139),
            fg: WHITE,
            glyph_scale: 0.50,
        },
    };
    brand
}

/// Resolve a brand for a saved provider. Presets win; otherwise the provider
/// name is matched so imported configuration still gets a sensible mark.
pub fn for_provider(provider: &Provider) -> Brand {
    if let Some(preset) = provider.preset.as_deref() {
        if presets::preset(preset).is_some() {
            return for_preset(Some(preset));
        }
    }
    for_name(&provider.name)
}

/// Best-effort brand from a provider display name. Used by places that only
/// carry a snapshot name (route rows, account groups).
pub fn for_name(provider_name: &str) -> Brand {
    let name = provider_name.to_lowercase();
    let key = if name.contains("openai") {
        "openai"
    } else if name.contains("anthropic") || name.contains("claude") {
        "anthropic"
    } else if name.contains("opencode go") {
        "opencode_go"
    } else if name.contains("opencode") || name.contains("zen") {
        "opencode_zen"
    } else if name.contains("command") {
        "command_code"
    } else if name.contains("智谱") || name.contains("zhipu") || name.contains("glm") {
        "zhipu"
    } else if name.contains("小米") || name.contains("xiaomi") || name.contains("mimo") {
        "xiaomi"
    } else if name.contains("minimax") {
        "minimax"
    } else if name.contains("豆包") || name.contains("doubao") || name.contains("火山") {
        "doubao"
    } else if name.contains("通义") || name.contains("qwen") || name.contains("千问") {
        "qwen"
    } else if name.contains("混元") || name.contains("hunyuan") {
        "hunyuan"
    } else if name.contains("阶跃") || name.contains("stepfun") || name.contains("step-") {
        "stepfun"
    } else {
        "custom"
    };
    for_preset(Some(key))
}

/// A provider logo tile. Allocates `size` and returns the response.
pub fn tile(ui: &mut Ui, brand: &Brand, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_tile(ui, rect, brand);
    response
}

pub fn paint_tile(ui: &Ui, rect: Rect, brand: &Brand) {
    let painter = ui.painter();
    let corner = (rect.width() * 0.28).clamp(4.0, 14.0);
    painter.rect_filled(rect, corner, brand.bg);
    // Soft depth: a clipped highlight in the upper-left corner.
    let clip = painter.with_clip_rect(rect);
    let glow = Rect::from_center_size(
        Pos2::new(
            rect.left() + rect.width() * 0.24,
            rect.top() + rect.height() * 0.18,
        ),
        Vec2::splat(rect.width() * 0.95),
    );
    clip.circle_filled(
        glow.center(),
        glow.width() * 0.5,
        Color32::from_rgba_unmultiplied(255, 255, 255, 26),
    );
    let glyph_size = (rect.width() * brand.glyph_scale).max(9.0);
    painter.text(
        rect.center() + Vec2::new(0.0, -0.5),
        Align2::CENTER_CENTER,
        brand.glyph,
        FontId::proportional(glyph_size),
        brand.fg,
    );
    painter.rect_stroke(
        rect.shrink(0.5),
        corner,
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 46)),
        egui::StrokeKind::Inside,
    );
}

/// Gallery of bundled presets. Returns the clicked preset key.
pub fn preset_gallery(ui: &mut Ui, selected: &str) -> Option<&'static str> {
    let t = theme::tokens(ui);
    let mut clicked = None;
    let mut groups: Vec<&'static str> = Vec::new();
    for preset in presets::PRESETS {
        if !groups.contains(&preset.group) {
            groups.push(preset.group);
        }
    }
    for group in groups {
        let items: Vec<&crate::config::presets::ProviderPreset> = presets::PRESETS
            .iter()
            .filter(|preset| preset.group == group)
            .collect();
        if items.is_empty() {
            continue;
        }
        ui.add_space(4.0);
        ui.label(RichText::new(group).size(10.0).strong().color(t.muted));
        ui.add_space(2.0);
        let avail = ui.available_width();
        let gap = 8.0_f32;
        let columns: usize = if avail >= 660.0 {
            3
        } else if avail >= 420.0 {
            2
        } else {
            1
        };
        let width =
            ((avail - gap * (columns.saturating_sub(1)) as f32) / columns as f32).max(150.0);
        for chunk in items.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for preset in chunk {
                    let card = preset_card(
                        ui,
                        width,
                        preset,
                        selected == preset.key,
                        t.card,
                        t.hover,
                        t.primary_soft,
                        t.primary,
                        t.card_border,
                        t.text,
                        t.muted,
                    );
                    if card.clicked() {
                        clicked = Some(preset.key);
                    }
                }
            });
            ui.add_space(gap);
        }
    }
    ui.add_space(2.0);
    clicked
}

#[allow(clippy::too_many_arguments)]
fn preset_card(
    ui: &mut Ui,
    width: f32,
    preset: &crate::config::presets::ProviderPreset,
    selected: bool,
    card: Color32,
    hover: Color32,
    selected_bg: Color32,
    primary: Color32,
    border: Color32,
    text: Color32,
    muted: Color32,
) -> Response {
    let height = 58.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let bg = if selected {
        selected_bg
    } else if response.hovered() {
        hover
    } else {
        card
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 11.0, bg);
    painter.rect_stroke(
        rect.shrink(0.5),
        11.0,
        Stroke::new(
            if selected { 1.4_f32 } else { 1.0_f32 },
            if selected { primary } else { border },
        ),
        egui::StrokeKind::Inside,
    );
    let brand = for_preset(Some(preset.key));
    let tile_rect = Rect::from_center_size(
        Pos2::new(rect.left() + 34.0, rect.center().y),
        Vec2::splat(34.0),
    );
    paint_tile(ui, tile_rect, &brand);
    painter.text(
        Pos2::new(rect.left() + 58.0, rect.center().y - 8.0),
        Align2::LEFT_CENTER,
        preset.name,
        FontId::proportional(12.0),
        text,
    );
    let desc = if preset.description.chars().count() > 22 {
        format!(
            "{}…",
            preset.description.chars().take(21).collect::<String>()
        )
    } else {
        preset.description.to_string()
    };
    painter.text(
        Pos2::new(rect.left() + 58.0, rect.center().y + 9.0),
        Align2::LEFT_CENTER,
        desc,
        FontId::proportional(9.0),
        muted,
    );
    response
}
