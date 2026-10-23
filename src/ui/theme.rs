use std::sync::Arc;

use armas::ext::ArmasContextExt;
use armas::Theme as ArmasTheme;
use egui::{
    Color32, Context, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke, TextStyle,
    Visuals,
};

use crate::config::ThemeMode;

pub const ACCENT: Color32 = Color32::from_rgb(37, 99, 235);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(232, 240, 254);
pub const GREEN: Color32 = Color32::from_rgb(22, 163, 74);
pub const ORANGE: Color32 = Color32::from_rgb(217, 119, 6);
pub const RED: Color32 = Color32::from_rgb(220, 38, 38);
pub const BLUE_INFO: Color32 = Color32::from_rgb(59, 130, 246);
pub const PURPLE: Color32 = Color32::from_rgb(124, 58, 237);
pub const MUTED: Color32 = Color32::from_rgb(107, 114, 128);

pub struct Tokens {
    pub bg: Color32,
    pub sidebar: Color32,
    pub card: Color32,
    pub card_border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub primary: Color32,
    pub primary_soft: Color32,
    pub green: Color32,
    pub green_soft: Color32,
    pub amber: Color32,
    pub amber_soft: Color32,
    pub red: Color32,
    pub red_soft: Color32,
    pub blue: Color32,
    pub blue_soft: Color32,
    pub purple: Color32,
    pub purple_soft: Color32,
    pub hover: Color32,
    pub input: Color32,
    pub dark_mode: bool,
    pub card_shadow: Shadow,
}

impl Tokens {
    pub fn light() -> Self {
        Self {
            bg: Color32::from_rgb(244, 246, 251),
            sidebar: Color32::from_rgb(251, 252, 254),
            card: Color32::WHITE,
            card_border: Color32::from_rgb(231, 235, 243),
            text: Color32::from_rgb(17, 24, 39),
            muted: Color32::from_rgb(107, 114, 128),
            primary: ACCENT,
            primary_soft: ACCENT_SOFT,
            green: GREEN,
            green_soft: Color32::from_rgb(220, 252, 231),
            amber: ORANGE,
            amber_soft: Color32::from_rgb(254, 243, 199),
            red: RED,
            red_soft: Color32::from_rgb(254, 226, 226),
            blue: BLUE_INFO,
            blue_soft: Color32::from_rgb(219, 234, 254),
            purple: PURPLE,
            purple_soft: Color32::from_rgb(237, 233, 254),
            hover: Color32::from_rgb(238, 242, 248),
            input: Color32::WHITE,
            dark_mode: false,
            card_shadow: Shadow {
                offset: [0, 2],
                blur: 12,
                spread: 0,
                color: Color32::from_rgba_unmultiplied(15, 23, 42, 18),
            },
        }
    }

    pub fn dark() -> Self {
        Self {
            bg: Color32::from_rgb(10, 13, 19),
            sidebar: Color32::from_rgb(13, 17, 24),
            card: Color32::from_rgb(20, 25, 34),
            card_border: Color32::from_rgb(35, 42, 55),
            text: Color32::from_rgb(232, 236, 243),
            muted: Color32::from_rgb(148, 163, 184),
            primary: Color32::from_rgb(79, 142, 247),
            primary_soft: Color32::from_rgb(28, 42, 68),
            green: Color32::from_rgb(52, 199, 123),
            green_soft: Color32::from_rgb(20, 46, 34),
            amber: Color32::from_rgb(245, 166, 65),
            amber_soft: Color32::from_rgb(50, 39, 20),
            red: Color32::from_rgb(239, 87, 87),
            red_soft: Color32::from_rgb(56, 24, 27),
            blue: Color32::from_rgb(96, 165, 250),
            blue_soft: Color32::from_rgb(22, 38, 60),
            purple: Color32::from_rgb(167, 139, 250),
            purple_soft: Color32::from_rgb(38, 30, 61),
            hover: Color32::from_rgb(28, 34, 44),
            input: Color32::from_rgb(16, 21, 29),
            dark_mode: true,
            card_shadow: Shadow {
                offset: [0, 2],
                blur: 14,
                spread: 0,
                color: Color32::from_rgba_unmultiplied(0, 0, 0, 70),
            },
        }
    }
}

pub fn tokens(ui: &egui::Ui) -> Tokens {
    if ui.visuals().dark_mode {
        Tokens::dark()
    } else {
        Tokens::light()
    }
}

pub fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    let candidates: &[(&str, u32)] = &[
        ("C:/Windows/Fonts/msyh.ttc", 0),
        ("C:/Windows/Fonts/msyh.ttf", 0),
        ("C:/Windows/Fonts/simhei.ttf", 0),
        ("/System/Library/Fonts/PingFang.ttc", 0),
        ("/System/Library/Fonts/STHeiti Light.ttc", 0),
        ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
        ("/usr/share/fonts/truetype/wqy/wqy-microhei.ttc", 0),
        ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 0),
    ];
    for (path, index) in candidates {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let data = FontData {
            font: std::borrow::Cow::Owned(bytes),
            index: *index,
            tweak: Default::default(),
        };
        fonts.font_data.insert("cjk".to_owned(), Arc::new(data));
        if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
            family.insert(0, "cjk".to_owned());
        }
        if let Some(family) = fonts.families.get_mut(&FontFamily::Monospace) {
            family.push("cjk".to_owned());
        }
        ctx.set_fonts(fonts);
        return;
    }
    tracing::warn!("未找到系统中文字体，界面可能出现缺字");
    ctx.set_fonts(fonts);
}

pub fn is_dark(ctx: &Context, mode: ThemeMode) -> bool {
    match mode {
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
        ThemeMode::System => ctx
            .system_theme()
            .map(|t| t == egui::Theme::Dark)
            .unwrap_or_else(|| ctx.style().visuals.dark_mode),
    }
}

fn armas_theme(tokens: &Tokens) -> ArmasTheme {
    let mut theme = if tokens.dark_mode {
        ArmasTheme::dark()
    } else {
        ArmasTheme::light()
    };
    let c = &mut theme.colors;
    let rgb = |c: Color32| [c.r(), c.g(), c.b()];
    c.background = rgb(tokens.bg);
    c.foreground = rgb(tokens.text);
    c.card = rgb(tokens.card);
    c.card_foreground = rgb(tokens.text);
    c.popover = rgb(tokens.card);
    c.popover_foreground = rgb(tokens.text);
    c.primary = rgb(tokens.primary);
    c.primary_foreground = [255, 255, 255];
    c.secondary = rgb(tokens.primary_soft);
    c.secondary_foreground = rgb(tokens.primary);
    c.muted = rgb(if tokens.dark_mode {
        Color32::from_rgb(28, 34, 44)
    } else {
        Color32::from_rgb(241, 245, 249)
    });
    c.muted_foreground = rgb(tokens.muted);
    c.accent = rgb(tokens.hover);
    c.accent_foreground = rgb(tokens.text);
    c.destructive = rgb(tokens.red);
    c.destructive_foreground = [255, 255, 255];
    c.border = rgb(tokens.card_border);
    c.input = rgb(tokens.card_border);
    c.ring = rgb(tokens.primary);
    c.chart_1 = rgb(tokens.primary);
    c.chart_2 = rgb(tokens.green);
    c.chart_3 = rgb(tokens.amber);
    c.chart_4 = rgb(tokens.purple);
    c.chart_5 = rgb(tokens.blue);
    c.hover = rgb(tokens.hover);
    c.focus = rgb(tokens.primary);
    c.sidebar = rgb(tokens.sidebar);
    c.sidebar_foreground = rgb(tokens.text);
    c.sidebar_primary = rgb(tokens.primary);
    c.sidebar_primary_foreground = [255, 255, 255];
    c.sidebar_accent = rgb(tokens.primary_soft);
    c.sidebar_accent_foreground = rgb(tokens.primary);
    c.sidebar_border = rgb(tokens.card_border);
    c.sidebar_ring = rgb(tokens.primary);
    theme
}

pub fn apply(ctx: &Context, dark: bool) {
    let tokens = if dark {
        Tokens::dark()
    } else {
        Tokens::light()
    };
    ctx.set_armas_theme(armas_theme(&tokens));

    let mut visuals = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    visuals.panel_fill = tokens.bg;
    visuals.window_fill = tokens.card;
    visuals.faint_bg_color = tokens.hover;
    visuals.extreme_bg_color = tokens.input;
    visuals.code_bg_color = tokens.hover;
    visuals.window_stroke = Stroke::new(1.0_f32, tokens.card_border);
    visuals.window_corner_radius = 14.into();
    visuals.menu_corner_radius = 10.into();
    visuals.hyperlink_color = tokens.primary;
    visuals.warn_fg_color = tokens.amber;
    visuals.error_fg_color = tokens.red;
    visuals.window_shadow = Shadow {
        offset: [0, 10],
        blur: 32,
        spread: 4,
        color: Color32::from_rgba_unmultiplied(15, 23, 42, if dark { 110 } else { 45 }),
    };

    visuals.widgets.noninteractive.bg_fill = tokens.card;
    visuals.widgets.noninteractive.weak_bg_fill = tokens.card;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, tokens.card_border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, tokens.text);
    visuals.widgets.inactive.bg_fill = tokens.card;
    visuals.widgets.inactive.weak_bg_fill = tokens.card;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, tokens.card_border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, tokens.text);
    visuals.widgets.hovered.bg_fill = tokens.hover;
    visuals.widgets.hovered.weak_bg_fill = tokens.hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, tokens.card_border);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, tokens.text);
    visuals.widgets.active.bg_fill = tokens.primary_soft;
    visuals.widgets.active.weak_bg_fill = tokens.primary_soft;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, tokens.primary);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, tokens.text);
    visuals.widgets.open.bg_fill = tokens.hover;
    visuals.widgets.open.weak_bg_fill = tokens.hover;
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, tokens.text);
    visuals.selection.bg_fill = tokens.primary.gamma_multiply(0.22);
    visuals.selection.stroke = Stroke::new(1.0_f32, tokens.primary);
    ctx.set_visuals(visuals);

    ctx.style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(11.0, 6.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.scroll.bar_width = 9.0;
        // Floating scrollbars hide until the pointer is already on them,
        // which makes them hard to grab. Reserve a visible track instead and
        // use the foreground colour so the handle is not white-on-white.
        style.spacing.scroll.floating = false;
        style.spacing.scroll.foreground_color = true;
        style.spacing.scroll.handle_min_length = 28.0;
        style.visuals.widgets.inactive.corner_radius = 8.into();
        style.visuals.widgets.hovered.corner_radius = 8.into();
        style.visuals.widgets.active.corner_radius = 8.into();
        style.visuals.widgets.noninteractive.corner_radius = 10.into();
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(24.0, FontFamily::Proportional),
        );
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(13.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(12.0, FontFamily::Monospace),
        );
    });
}

#[allow(dead_code)]
pub fn dark_card(ui: &egui::Ui) -> Color32 {
    tokens(ui).card
}

pub fn border_color(ui: &egui::Ui) -> Color32 {
    tokens(ui).card_border
}

pub fn muted(ui: &egui::Ui) -> Color32 {
    tokens(ui).muted
}

pub fn subtle_fill(ui: &egui::Ui) -> Color32 {
    tokens(ui).hover
}

#[allow(dead_code)]
pub fn primary(ui: &egui::Ui) -> Color32 {
    tokens(ui).primary
}

pub fn text(ui: &egui::Ui) -> Color32 {
    tokens(ui).text
}

pub fn soft_for(ui: &egui::Ui, color: Color32) -> Color32 {
    let t = tokens(ui);
    if color == t.primary {
        t.primary_soft
    } else if color == t.green {
        t.green_soft
    } else if color == t.amber {
        t.amber_soft
    } else if color == t.red {
        t.red_soft
    } else if color == t.blue {
        t.blue_soft
    } else if color == t.purple {
        t.purple_soft
    } else {
        color.gamma_multiply(0.12)
    }
}

pub fn tokens_ctx(ctx: &egui::Context) -> Tokens {
    if ctx.style().visuals.dark_mode {
        Tokens::dark()
    } else {
        Tokens::light()
    }
}
