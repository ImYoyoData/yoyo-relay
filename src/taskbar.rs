//! Windows taskbar live-stats badge.
//!
//! Windows has no supported deskband API for third-party apps, and the GL
//! window surface cannot be transparent. The badge is therefore a small native
//! layered window drawn with GDI (per-pixel alpha), placed just left of the
//! notification area and marked click-through.

/// Visual placement options for the taskbar badge. Passed from the UI so the
/// settings page can resize/nudge the badge in real time.
#[derive(Clone, Copy, Debug)]
pub struct BadgeStyle {
    pub scale: f32,
    pub offset_x: i32,
    pub offset_y: i32,
    pub gap: i32,
    pub opacity: f32,
    pub show_dot: bool,
}

impl Default for BadgeStyle {
    fn default() -> Self {
        Self {
            scale: 1.0,
            offset_x: 0,
            offset_y: 0,
            gap: 10,
            opacity: 0.95,
            show_dot: true,
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::Once;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW,
        GetDC, ReleaseDC, SelectObject, SetBkMode, SetTextColor, AC_SRC_ALPHA, AC_SRC_OVER,
        BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CLEARTYPE_QUALITY, DEFAULT_CHARSET,
        DIB_RGB_COLORS, DT_CALCRECT, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FW_NORMAL,
        HGDIOBJ, TRANSPARENT,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowExW, FindWindowW, GetWindowRect,
        RegisterClassW, SetWindowPos, ShowWindow, UpdateLayeredWindow, HWND_TOPMOST,
        SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, ULW_ALPHA, WNDCLASSW, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };

    use super::BadgeStyle;

    static REGISTER_CLASS: Once = Once::new();

    const FONT_NAME: &str = "Microsoft YaHei UI";

    /// A click-through, always-on-top badge showing live request statistics.
    pub struct TaskbarBadge {
        hwnd: HWND,
        last_text: String,
        last_dark: bool,
        last_dot: (u8, u8, u8),
        last_size: (i32, i32),
        last_pos: (i32, i32),
        last_style: (i32, i32, i32, u8, bool),
        last_paint: Instant,
        visible: bool,
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn colorref(rgb: (u8, u8, u8)) -> COLORREF {
        (rgb.0 as u32) | ((rgb.1 as u32) << 8) | ((rgb.2 as u32) << 16)
    }

    fn register_class() {
        REGISTER_CLASS.call_once(|| unsafe {
            let class_name = wide("YoyoRelayTaskbarBadge");
            let instance = GetModuleHandleW(std::ptr::null());
            let wnd = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(DefWindowProcW),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };
            RegisterClassW(&wnd);
        });
    }

    fn anchor(width: i32, height: i32, style: &BadgeStyle) -> Option<(i32, i32)> {
        unsafe {
            let tray = FindWindowW(windows_sys::w!("Shell_TrayWnd"), std::ptr::null());
            if tray.is_null() {
                return None;
            }
            let mut tray_rect = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if GetWindowRect(tray, &mut tray_rect) == 0 {
                return None;
            }
            let tray_w = tray_rect.right - tray_rect.left;
            let tray_h = tray_rect.bottom - tray_rect.top;
            if tray_w <= tray_h {
                return None;
            }
            let notify = FindWindowExW(
                tray,
                std::ptr::null_mut(),
                windows_sys::w!("TrayNotifyWnd"),
                std::ptr::null(),
            );
            let mut notify_left = tray_rect.right;
            if !notify.is_null() {
                let mut candidate = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                if GetWindowRect(notify, &mut candidate) != 0
                    && candidate.right > candidate.left
                    && candidate.bottom > candidate.top
                {
                    notify_left = candidate.left;
                }
            }
            let x = (notify_left - width - style.gap + style.offset_x).max(tray_rect.left);
            let y = tray_rect.top + ((tray_h - height) / 2).max(0) + style.offset_y;
            Some((x, y))
        }
    }

    fn create_font(scale: f32) -> windows_sys::Win32::Graphics::Gdi::HFONT {
        unsafe {
            let font_name = wide(FONT_NAME);
            let font_height = -((11.0 * scale).round() as i32).max(12);
            CreateFontW(
                font_height,
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                CLEARTYPE_QUALITY as u32,
                0,
                font_name.as_ptr(),
            )
        }
    }

    /// Measure the rendered width of the badge text so the capsule hugs its
    /// content instead of using a fixed width.
    fn measure(text: &str, scale: f32, show_dot: bool) -> (i32, i32) {
        unsafe {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            if dc.is_null() {
                return ((120.0 * scale) as i32, (30.0 * scale) as i32);
            }
            let font = create_font(scale);
            let old_font = if font.is_null() {
                std::ptr::null_mut()
            } else {
                SelectObject(dc, font as HGDIOBJ)
            };
            let text_wide = wide(text);
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            DrawTextW(
                dc,
                text_wide.as_ptr(),
                -1,
                &mut rect,
                DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
            );
            if !old_font.is_null() {
                SelectObject(dc, old_font);
            }
            if !font.is_null() {
                DeleteObject(font as HGDIOBJ);
            }
            DeleteDC(dc);
            let pad = (9.0 * scale).round().max(5.0) as i32;
            let dot_width = if show_dot {
                (3.4 * scale).max(2.5) * 2.0 + 7.0 * scale
            } else {
                0.0
            } as i32;
            let width = (rect.right.max(1) + pad * 2 + dot_width).max((40.0 * scale) as i32);
            let height = ((30.0 * scale).round() as i32).max(22);
            (width, height)
        }
    }

    impl TaskbarBadge {
        pub fn new() -> Option<Self> {
            register_class();
            unsafe {
                let class_name = wide("YoyoRelayTaskbarBadge");
                let title = wide("YoyoRelay 状态");
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED
                        | WS_EX_TOOLWINDOW
                        | WS_EX_TOPMOST
                        | WS_EX_NOACTIVATE
                        | WS_EX_TRANSPARENT,
                    class_name.as_ptr(),
                    title.as_ptr(),
                    WS_POPUP,
                    0,
                    0,
                    10,
                    10,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    GetModuleHandleW(std::ptr::null()),
                    std::ptr::null_mut(),
                );
                if hwnd.is_null() {
                    return None;
                }
                Some(Self {
                    hwnd,
                    last_text: String::new(),
                    last_dark: false,
                    last_dot: (0, 0, 0),
                    last_size: (0, 0),
                    last_pos: (i32::MIN, i32::MIN),
                    last_style: (0, 0, 0, 0, false),
                    last_paint: Instant::now() - Duration::from_secs(10),
                    visible: false,
                })
            }
        }

        pub fn update(
            &mut self,
            text: &str,
            dot: (u8, u8, u8),
            dark: bool,
            dpi: f32,
            style: &BadgeStyle,
        ) {
            let dpi = if dpi.is_finite() {
                dpi.clamp(0.75, 3.0)
            } else {
                1.0
            };
            let scale = (dpi * style.scale.clamp(0.6, 2.2)).clamp(0.6, 4.0);
            let opacity = if style.opacity.is_finite() {
                style.opacity.clamp(0.25, 1.0)
            } else {
                1.0
            };
            let (width, height) = measure(text, scale, style.show_dot);
            let style_key = (
                style.offset_x,
                style.offset_y,
                style.gap,
                (opacity * 255.0).round() as u8,
                style.show_dot,
            );
            let same_content = self.last_text == text
                && self.last_dark == dark
                && self.last_dot == dot
                && self.last_size == (width, height)
                && self.last_style == style_key;
            // Avoid Win32 window queries on every UI frame; content changes or
            // the periodic re-anchor below still repaint promptly.
            if same_content
                && self.visible
                && self.last_paint.elapsed() < Duration::from_millis(800)
            {
                return;
            }
            let Some(pos) = anchor(width, height, style) else {
                if self.visible {
                    unsafe {
                        ShowWindow(self.hwnd, SW_HIDE);
                    }
                    self.visible = false;
                }
                return;
            };
            let changed = !same_content || self.last_pos != pos;
            // Re-anchor periodically so the badge follows taskbar moves.
            if !changed && self.last_paint.elapsed() < Duration::from_secs(2) {
                return;
            }
            if self.paint(
                width,
                height,
                pos,
                text,
                dot,
                dark,
                scale,
                opacity,
                style.show_dot,
            ) {
                self.last_text = text.to_string();
                self.last_dark = dark;
                self.last_dot = dot;
                self.last_size = (width, height);
                self.last_pos = pos;
                self.last_style = style_key;
                self.last_paint = Instant::now();
                self.visible = true;
            }
        }

        #[allow(clippy::too_many_arguments)]
        fn paint(
            &self,
            width: i32,
            height: i32,
            pos: (i32, i32),
            text: &str,
            dot: (u8, u8, u8),
            dark: bool,
            scale: f32,
            opacity: f32,
            show_dot: bool,
        ) -> bool {
            unsafe {
                let mut bmi = BITMAPINFO::default();
                bmi.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: 0,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                };
                let mut bits: *mut c_void = std::ptr::null_mut();
                let dib = CreateDIBSection(
                    std::ptr::null_mut(),
                    &bmi,
                    DIB_RGB_COLORS,
                    &mut bits,
                    std::ptr::null_mut(),
                    0,
                );
                if dib.is_null() || bits.is_null() {
                    return false;
                }
                let pixels =
                    std::slice::from_raw_parts_mut(bits as *mut u32, (width * height) as usize);
                pixels.fill(0);

                let (bg, fg, rim) = if dark {
                    ((38, 40, 46), (236, 239, 245), (66, 70, 78))
                } else {
                    ((252, 252, 253), (33, 41, 54), (222, 227, 234))
                };
                let bg_alpha = 242u32;
                let radius = height as f32 / 2.0;
                let dot_radius = (3.4 * scale).max(2.5);
                let dot_cx = 10.0 * scale + dot_radius;
                let dot_cy = height as f32 / 2.0;
                for y in 0..height {
                    let fy = y as f32 + 0.5;
                    for x in 0..width {
                        let fx = x as f32 + 0.5;
                        let distance = capsule_distance(fx, fy, width, height, radius);
                        let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
                        if coverage <= 0.0 {
                            continue;
                        }
                        let color = if distance > radius - 1.2 { rim } else { bg };
                        let alpha = (bg_alpha as f32 * coverage) as u32;
                        pixels[(y * width + x) as usize] = argb(alpha, color);
                    }
                }

                if show_dot {
                    for y in 0..height {
                        let fy = y as f32 + 0.5;
                        for x in 0..width {
                            let fx = x as f32 + 0.5;
                            let dist = ((fx - dot_cx).powi(2) + (fy - dot_cy).powi(2)).sqrt();
                            let coverage = (dot_radius - dist + 0.5).clamp(0.0, 1.0);
                            if coverage <= 0.0 {
                                continue;
                            }
                            let idx = (y * width + x) as usize;
                            let (r, g, b) = (
                                (dot.0 as f32 * coverage + bg.0 as f32 * (1.0 - coverage)) as u8,
                                (dot.1 as f32 * coverage + bg.1 as f32 * (1.0 - coverage)) as u8,
                                (dot.2 as f32 * coverage + bg.2 as f32 * (1.0 - coverage)) as u8,
                            );
                            pixels[idx] = argb(bg_alpha, (r, g, b));
                        }
                    }
                }

                let before = pixels.to_vec();
                let dc = CreateCompatibleDC(std::ptr::null_mut());
                if dc.is_null() {
                    DeleteObject(dib as HGDIOBJ);
                    return false;
                }
                let old_bitmap = SelectObject(dc, dib as HGDIOBJ);
                SetBkMode(dc, TRANSPARENT as i32);
                SetTextColor(dc, colorref(fg));
                let font = create_font(scale);
                let old_font = if font.is_null() {
                    std::ptr::null_mut()
                } else {
                    SelectObject(dc, font as HGDIOBJ)
                };
                let text_wide = wide(text);
                let mut rect = RECT {
                    left: if show_dot {
                        (dot_cx + dot_radius + 7.0 * scale) as i32
                    } else {
                        (9.0 * scale) as i32
                    },
                    top: 0,
                    right: width - (8.0 * scale) as i32,
                    bottom: height,
                };
                DrawTextW(
                    dc,
                    text_wide.as_ptr(),
                    -1,
                    &mut rect,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                );
                // GDI does not maintain the alpha channel. Pixels whose RGB was
                // touched by the glyphs become opaque.
                for (idx, pixel) in pixels.iter_mut().enumerate() {
                    if (*pixel & 0x00FF_FFFF) != (before[idx] & 0x00FF_FFFF) {
                        *pixel = (*pixel & 0x00FF_FFFF) | 0xFF00_0000;
                    }
                }

                let screen = GetDC(std::ptr::null_mut());
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: (opacity.clamp(0.25, 1.0) * 255.0).round() as u8,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                let dst = POINT { x: pos.0, y: pos.1 };
                let size = SIZE {
                    cx: width,
                    cy: height,
                };
                let src = POINT { x: 0, y: 0 };
                UpdateLayeredWindow(
                    self.hwnd, screen, &dst, &size, dc, &src, 0, &blend, ULW_ALPHA,
                );
                ReleaseDC(std::ptr::null_mut(), screen);
                SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    pos.0,
                    pos.1,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
                if !old_font.is_null() {
                    SelectObject(dc, old_font);
                }
                if !font.is_null() {
                    DeleteObject(font as HGDIOBJ);
                }
                SelectObject(dc, old_bitmap);
                DeleteDC(dc);
                DeleteObject(dib as HGDIOBJ);
                true
            }
        }
    }

    impl Drop for TaskbarBadge {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.hwnd);
            }
        }
    }

    fn capsule_distance(x: f32, y: f32, width: i32, height: i32, radius: f32) -> f32 {
        let cy = height as f32 / 2.0;
        if x < radius {
            ((x - radius).powi(2) + (y - cy).powi(2)).sqrt()
        } else if x > width as f32 - radius {
            ((x - (width as f32 - radius)).powi(2) + (y - cy).powi(2)).sqrt()
        } else {
            0.0
        }
    }

    fn argb(alpha: u32, rgb: (u8, u8, u8)) -> u32 {
        (alpha << 24) | ((rgb.0 as u32) << 16) | ((rgb.1 as u32) << 8) | (rgb.2 as u32)
    }
}

#[cfg(windows)]
pub use imp::TaskbarBadge;

#[cfg(not(windows))]
pub struct TaskbarBadge;

#[cfg(not(windows))]
impl TaskbarBadge {
    pub fn new() -> Option<Self> {
        None
    }

    pub fn update(
        &mut self,
        _text: &str,
        _dot: (u8, u8, u8),
        _dark: bool,
        _dpi: f32,
        _style: &BadgeStyle,
    ) {
    }
}
