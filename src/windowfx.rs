//! Native window frame polish.
//!
//! The main window is undecorated (`with_decorations(false)`), so Windows
//! draws neither the drop shadow nor the rounded corners users expect. Asking
//! DWM for a 1px extended frame keeps its shadow, and the corner preference
//! clips the client surface to the Win11 rounded shape.

#[cfg(windows)]
pub fn apply_native_frame(hwnd: isize) {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Dwm::{
        DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE,
        DWMWCP_ROUND,
    };
    use windows_sys::Win32::UI::Controls::MARGINS;

    let hwnd = hwnd as HWND;
    if hwnd.is_null() {
        return;
    }
    unsafe {
        // A minimal non-zero margin opts the window into the DWM frame,
        // which is what produces the drop shadow around a borderless window.
        let margins = MARGINS {
            cxLeftWidth: 1,
            cxRightWidth: 1,
            cyTopHeight: 1,
            cyBottomHeight: 1,
        };
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);

        let preference: i32 = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &preference as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }
}

#[cfg(not(windows))]
pub fn apply_native_frame(_hwnd: isize) {}
