//! Single-instance coordination.
//!
//! The first process takes an exclusive lock on `instance.lock` and listens on a
//! loopback control socket whose port is published in `instance.json`. Later
//! launches read that file, ask the running process to reveal its window and then
//! exit quietly — relaunching the app never produces an error dialog.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use tokio::sync::mpsc;

use crate::runtime::UiCommand;

/// Cross-thread window requests raised by the tray menu or a second launch.
#[derive(Default)]
pub struct Signals {
    show: AtomicBool,
    quit: AtomicBool,
    hidden: AtomicBool,
    ctx: Mutex<Option<egui::Context>>,
    /// Native window handle, used because a hidden window on Windows stops
    /// receiving redraw requests (so viewport commands alone cannot restore it).
    window: AtomicIsize,
    quit_tx: Mutex<Option<mpsc::UnboundedSender<UiCommand>>>,
    info_path: Mutex<Option<PathBuf>>,
}

impl Signals {
    /// Remember the UI context so other threads can wake the event loop.
    pub fn attach(&self, ctx: &egui::Context) {
        if let Ok(mut slot) = self.ctx.lock() {
            *slot = Some(ctx.clone());
        }
    }

    pub fn attach_window(&self, raw: isize) {
        self.window.store(raw, Ordering::SeqCst);
    }

    pub fn attach_quit(&self, tx: mpsc::UnboundedSender<UiCommand>) {
        if let Ok(mut slot) = self.quit_tx.lock() {
            *slot = Some(tx);
        }
    }

    fn attach_info(&self, path: PathBuf) {
        if let Ok(mut slot) = self.info_path.lock() {
            *slot = Some(path);
        }
    }

    pub fn request_show(&self) {
        self.show.store(true, Ordering::SeqCst);
        self.show_native();
        self.wake();
    }

    pub fn request_quit(&self) {
        self.quit.store(true, Ordering::SeqCst);
        self.wake();
        if !self.hidden.load(Ordering::SeqCst) {
            // Visible window: the next frame runs the normal shutdown path.
            return;
        }
        // A hidden window never runs another frame, so a tray "quit" cannot be
        // completed through the UI loop. Stop the runtime, then exit.
        let tx = self.quit_tx.lock().ok().and_then(|slot| slot.clone());
        if let Some(tx) = tx {
            let _ = tx.send(UiCommand::Shutdown);
            std::thread::sleep(Duration::from_millis(350));
            if let Some(path) = self.info_path.lock().ok().and_then(|slot| slot.clone()) {
                let _ = std::fs::remove_file(path);
            }
            std::process::exit(0);
        }
    }

    pub fn mark_hidden(&self, hidden: bool) {
        self.hidden.store(hidden, Ordering::SeqCst);
    }

    pub fn take_show(&self) -> bool {
        self.show.swap(false, Ordering::SeqCst)
    }

    pub fn take_quit(&self) -> bool {
        self.quit.swap(false, Ordering::SeqCst)
    }

    fn wake(&self) {
        if let Ok(slot) = self.ctx.lock() {
            if let Some(ctx) = slot.as_ref() {
                ctx.request_repaint();
            }
        }
    }

    /// Reveal and focus the window without waiting for a new UI frame.
    /// Returns `true` when a native handle was available.
    #[cfg(windows)]
    fn show_native(&self) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
        };
        let hwnd = self.window.load(Ordering::SeqCst);
        if hwnd == 0 {
            return false;
        }
        let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;
        unsafe {
            ShowWindow(hwnd, SW_SHOW);
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
        }
        true
    }

    #[cfg(not(windows))]
    fn show_native(&self) -> bool {
        false
    }
}

pub enum Instance {
    Primary(Guard),
    AlreadyRunning,
}

/// Keeps the instance lock and control listener alive for the process lifetime.
pub struct Guard {
    _lock: std::fs::File,
    stop: Arc<AtomicBool>,
    info_path: PathBuf,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::fs::remove_file(&self.info_path);
    }
}

pub fn acquire(config_dir: &Path, signals: Arc<Signals>) -> Result<Instance> {
    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("无法创建配置目录：{}", config_dir.display()))?;
    let lock_path = config_dir.join("instance.lock");
    let info_path = config_dir.join("instance.json");

    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("无法创建单实例锁：{}", lock_path.display()))?;

    if lock.try_lock_exclusive().is_err() {
        notify_running(&info_path)?;
        return Ok(Instance::AlreadyRunning);
    }

    let listener = TcpListener::bind(("127.0.0.1", 0)).context("无法创建单实例控制通道")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let info = serde_json::json!({
        "port": port,
        "pid": std::process::id(),
    });
    std::fs::write(&info_path, info.to_string())
        .with_context(|| format!("无法写入单实例信息：{}", info_path.display()))?;
    signals.attach_info(info_path.clone());

    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let thread_signals = signals;
    std::thread::Builder::new()
        .name("yoyo-instance".into())
        .spawn(move || {
            let mut buf = [0u8; 32];
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                        let read = stream.read(&mut buf).unwrap_or(0);
                        match String::from_utf8_lossy(&buf[..read]).trim() {
                            "show" => thread_signals.request_show(),
                            "quit" => thread_signals.request_quit(),
                            _ => {}
                        }
                        let _ = stream.write_all(b"ok");
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(120));
                    }
                    Err(_) => break,
                }
            }
        })
        .context("无法启动单实例监听线程")?;

    Ok(Instance::Primary(Guard {
        _lock: lock,
        stop,
        info_path,
    }))
}

/// Ask the already-running process to show its window.
fn notify_running(info_path: &Path) -> Result<()> {
    let raw = std::fs::read_to_string(info_path).unwrap_or_default();
    let info: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
    let port = info
        .get("port")
        .and_then(|value| value.as_u64())
        .filter(|port| *port > 0 && *port <= u16::MAX as u64)
        .ok_or_else(|| anyhow!("YoyoRelay 已在运行，但无法定位它的窗口。请从系统托盘打开。"))?
        as u16;
    let pid = info
        .get("pid")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as u32;

    #[cfg(windows)]
    if pid != 0 {
        // The running process may be denied foreground activation otherwise.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
        }
    }

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut stream) => {
                stream.set_write_timeout(Some(Duration::from_millis(500)))?;
                stream.write_all(b"show")?;
                let _ = stream.flush();
                return Ok(());
            }
            Err(err) if Instant::now() < deadline => {
                let _ = err;
                std::thread::sleep(Duration::from_millis(120));
            }
            Err(_) => {
                return Err(anyhow!(
                    "YoyoRelay 已在运行，但暂时无法连接它的窗口。请稍后重试，或从系统托盘打开。"
                ))
            }
        }
    }
}
