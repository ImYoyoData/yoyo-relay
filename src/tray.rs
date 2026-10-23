//! System tray icon and menu.
//!
//! The tray always uses the application logo. Creation is deferred to the first
//! UI frame because macOS requires a running event loop and Windows needs the
//! winit message pump to deliver tray callbacks.

use std::sync::Arc;

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::instance::Signals;

pub struct Tray {
    icon: TrayIcon,
    stats_item: MenuItem,
    detail_item: MenuItem,
    last_summary: String,
    last_detail: String,
}

impl Tray {
    /// Update the notification-area tooltip and the always-visible menu header
    /// with live request/token information.
    pub fn update_stats(&mut self, summary: &str, detail: &str) {
        if self.last_summary == summary && self.last_detail == detail {
            return;
        }
        self.last_summary = summary.to_string();
        self.last_detail = detail.to_string();
        self.stats_item.set_text(summary);
        self.detail_item.set_text(detail);
        let tooltip = format!("{}\n{summary}\n{detail}", crate::identity::APP_NAME);
        // Windows truncates notifications longer than 127 UTF-16 units.
        let tooltip: String = tooltip.chars().take(120).collect();
        let _ = self.icon.set_tooltip(Some(tooltip));
        // Used by Linux AppIndicator and macOS status items where a title is
        // rendered next to the icon.
        self.icon.set_title(Some(summary.to_string()));
    }
}

pub fn install(signals: Arc<Signals>) -> Option<Tray> {
    const SIZE: u32 = 64;
    const RGBA: &[u8] = include_bytes!("../assets/icon_64.rgba");
    let icon = Icon::from_rgba(RGBA.to_vec(), SIZE, SIZE).ok()?;

    let menu = Menu::new();
    let stats = MenuItem::new("正在加载运行状态…", false, None);
    let detail = MenuItem::new("打开主窗口查看请求与用量", false, None);
    let show = MenuItem::new("显示主窗口", true, None);
    let quit = MenuItem::new(format!("退出 {}", crate::identity::APP_NAME), true, None);
    let separator = PredefinedMenuItem::separator();
    let separator2 = PredefinedMenuItem::separator();
    menu.append_items(&[&stats, &detail, &separator, &show, &separator2, &quit])
        .ok()?;

    let show_id = show.id().clone();
    let quit_id = quit.id().clone();

    TrayIconEvent::set_event_handler(Some({
        let signals = signals.clone();
        move |event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
            {
                signals.request_show();
            }
        }
    }));

    MenuEvent::set_event_handler(Some({
        let signals = signals.clone();
        move |event: MenuEvent| {
            if event.id == show_id {
                signals.request_show();
            } else if event.id == quit_id {
                signals.request_quit();
            }
        }
    }));

    let icon = TrayIconBuilder::new()
        .with_tooltip(format!("{} · 本地 LLM 代理", crate::identity::APP_NAME))
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .build()
        .ok()?;

    Some(Tray {
        icon,
        stats_item: stats,
        detail_item: detail,
        last_summary: String::new(),
        last_detail: String::new(),
    })
}
