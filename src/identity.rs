//! Build identity.
//!
//! Debug builds are a separate application from the released one: they use
//! their own config directory, default proxy port, autostart entry, window
//! title and single-instance lock. That way running a development build can
//! never take over, signal, or quit the installed release.

/// Display name used in the window title, tray tooltip and messages.
pub const APP_NAME: &str = if cfg!(debug_assertions) {
    "YoyoRelay Dev"
} else {
    "YoyoRelay"
};

/// Directory under the user's config root.
pub const CONFIG_DIR_NAME: &str = if cfg!(debug_assertions) {
    "YoyoRelay-dev"
} else {
    "YoyoRelay"
};

/// Windows `Run` registry value name (also used as the tray/AppUserModel id).
pub const AUTOSTART_VALUE: &str = if cfg!(debug_assertions) {
    "YoyoRelayDev"
} else {
    "YoyoRelay"
};

/// macOS LaunchAgent label; the debug build registers a separate job.
#[cfg(target_os = "macos")]
pub const LAUNCH_AGENT: &str = if cfg!(debug_assertions) {
    "com.yoyorelay.app.dev"
} else {
    "com.yoyorelay.app"
};

/// Linux XDG autostart file name.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn autostart_desktop_file() -> &'static str {
    if cfg!(debug_assertions) {
        "yoyo-relay-dev.desktop"
    } else {
        "yoyo-relay.desktop"
    }
}

/// Window class for the taskbar badge; debug and release may run side by side,
/// so they must not share a native window class (or overwrite each other).
pub const BADGE_CLASS: &str = if cfg!(debug_assertions) {
    "YoyoRelayDevTaskbarBadge"
} else {
    "YoyoRelayTaskbarBadge"
};

/// Default local proxy port. The debug build listens elsewhere so both
/// instances can run side by side.
pub const DEFAULT_PORT: u16 = if cfg!(debug_assertions) {
    17_789
} else {
    7_789
};
