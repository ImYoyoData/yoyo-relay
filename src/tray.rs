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
    _icon: TrayIcon,
}

pub fn install(signals: Arc<Signals>) -> Option<Tray> {
    const SIZE: u32 = 64;
    const RGBA: &[u8] = include_bytes!("../assets/icon_64.rgba");
    let icon = Icon::from_rgba(RGBA.to_vec(), SIZE, SIZE).ok()?;

    let menu = Menu::new();
    let show = MenuItem::new("显示主窗口", true, None);
    let quit = MenuItem::new("退出 YoyoRelay", true, None);
    let separator = PredefinedMenuItem::separator();
    menu.append_items(&[&show, &separator, &quit]).ok()?;

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

    TrayIconBuilder::new()
        .with_tooltip("YoyoRelay · 本地 LLM 代理")
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .build()
        .ok()
        .map(|icon| Tray { _icon: icon })
}
