#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod balance;
mod config;
mod db;
mod gateway;
mod probe;
mod protocol;
mod runtime;
mod scheduler;
mod snapshot;
mod ui;

use std::fs::OpenOptions;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use tracing_subscriber::EnvFilter;

use crate::config::ConfigStore;

fn main() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .try_init();

    if let Err(err) = run() {
        eprintln!("YoyoRelay 启动失败：{err:#}");
        run_fatal(&format!("{err:#}"));
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Autostart management does not need the config store or the single-instance lock:
    // it must also work while YoyoRelay is already running.
    if let Some(index) = args.iter().position(|a| a == "--autostart") {
        let enabled = args
            .get(index + 1)
            .map(|value| !matches!(value.as_str(), "off" | "false" | "0" | "disable"))
            .unwrap_or(true);
        autostart::set_enabled(enabled)?;
        println!(
            "autostart {}: {}",
            if enabled { "enabled" } else { "disabled" },
            std::env::current_exe()?.display()
        );
        return Ok(());
    }

    let store = Arc::new(ConfigStore::open().context("初始化配置目录失败")?);
    let _instance_lock = acquire_instance_lock(&store.paths.config_dir.join("instance.lock"))?;
    let outcome = store.load()?;
    let headless = args.iter().any(|a| a == "--headless");
    let selftest = args.iter().any(|a| a == "--selftest");
    let initial_page = args
        .iter()
        .position(|a| a == "--page")
        .and_then(|i| args.get(i + 1))
        .and_then(|key| ui::Page::from_key(key))
        .or_else(|| {
            std::env::var("YOYO_RELAY_PAGE")
                .ok()
                .and_then(|key| ui::Page::from_key(&key))
        })
        .unwrap_or(ui::Page::Overview);

    let (cmd_tx, snap_rx) = runtime::spawn(
        store.clone(),
        outcome.config.clone(),
        outcome.warnings.clone(),
    )
    .map_err(|err| anyhow!("启动运行时失败：{err}"))?;

    if selftest {
        run_selftest(&outcome.config, &cmd_tx, snap_rx);
        let _ = cmd_tx.send(runtime::UiCommand::Shutdown);
        std::thread::sleep(std::time::Duration::from_millis(300));
        return Ok(());
    }

    if headless {
        run_headless(snap_rx);
        let _ = cmd_tx.send(runtime::UiCommand::Shutdown);
        std::thread::sleep(std::time::Duration::from_millis(300));
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 660.0])
            .with_min_inner_size([820.0, 560.0])
            .with_decorations(false)
            .with_transparent(false)
            .with_resizable(true)
            .with_clamp_size_to_monitor_size(true)
            .with_title("YoyoRelay")
            .with_icon(app_icon()),
        renderer: eframe::Renderer::Glow,
        vsync: true,
        ..Default::default()
    };

    let result = eframe::run_native(
        "YoyoRelay",
        options,
        Box::new(move |cc| {
            Ok(Box::new(ui::App::new(
                cc,
                store,
                outcome.config,
                cmd_tx,
                snap_rx,
                initial_page,
            )))
        }),
    );
    if let Err(err) = result {
        return Err(anyhow!("窗口运行失败：{err}"));
    }
    Ok(())
}

fn acquire_instance_lock(path: &std::path::Path) -> Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("无法创建单实例锁：{}", path.display()))?;
    file.try_lock_exclusive().map_err(|_| {
        anyhow!("已有一个 YoyoRelay 实例在运行。请从任务栏恢复已有窗口，或先退出它。")
    })?;
    Ok(file)
}

fn run_headless(snap_rx: std::sync::mpsc::Receiver<Arc<snapshot::UiSnapshot>>) {
    println!("YoyoRelay headless 模式已启动；使用 curl 访问本地端点，按 Ctrl+C 退出。");
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let _ = ctrlc::set_handler(move || {
        let _ = stop_tx.send(());
    });
    std::thread::spawn(move || {
        let mut last = String::new();
        while let Ok(snapshot) = snap_rx.recv() {
            let line = format!(
                "endpoint={} proxy={} active={} queued={}",
                snapshot.endpoint,
                snapshot.proxy.label(),
                snapshot.active_requests,
                snapshot.queued_requests
            );
            if line != last {
                println!("{line}");
                last = line;
            }
        }
    });
    let _ = stop_rx.recv();
    println!("收到退出信号，正在停止代理…");
}

fn run_selftest(
    config: &config::AppConfig,
    cmd_tx: &tokio::sync::mpsc::UnboundedSender<runtime::UiCommand>,
    snap_rx: std::sync::mpsc::Receiver<Arc<snapshot::UiSnapshot>>,
) {
    use runtime::UiCommand;

    // Wait for the proxy to be listening so the local-key probe can hit it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut snapshot = loop {
        match snap_rx.recv_timeout(std::time::Duration::from_millis(300)) {
            Ok(s) => {
                if s.proxy.is_running() {
                    break s;
                }
                if std::time::Instant::now() >= deadline {
                    break s;
                }
            }
            Err(_) if std::time::Instant::now() >= deadline => {
                eprintln!("自检：等待代理启动超时");
                return;
            }
            Err(_) => {}
        }
    };
    println!(
        "自检开始：endpoint={} proxy={}",
        snapshot.endpoint,
        snapshot.proxy.label()
    );

    let mut expected = 0usize;
    for provider in &config.providers {
        let _ = cmd_tx.send(UiCommand::TestProvider(provider.id.clone()));
        expected += 1;
    }
    for account in &config.accounts {
        let _ = cmd_tx.send(UiCommand::TestAccount(account.id.clone()));
        expected += 1;
    }
    for route in &config.routes {
        let _ = cmd_tx.send(UiCommand::TestRoute(route.id.clone()));
        expected += 1;
    }
    for key in &config.local_keys {
        let _ = cmd_tx.send(UiCommand::TestLocalKey(key.id.clone()));
        expected += 1;
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
    while std::time::Instant::now() < deadline {
        if let Ok(s) = snap_rx.recv_timeout(std::time::Duration::from_millis(250)) {
            snapshot = s;
        }
        let done = snapshot.running_tests.is_empty() && snapshot.tests.len() >= expected;
        if done {
            break;
        }
    }

    let mut keys: Vec<_> = snapshot.tests.values().collect();
    keys.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.title.cmp(&b.title)));
    let mut failures = 0usize;
    for result in keys {
        if !result.kind.is_empty() && result.kind != "draft" {
            println!(
                "
[{}] {} → {}（{} ms）",
                result.kind,
                result.title,
                result.level.label(),
                result.latency_ms
            );
            println!("  {}", result.message);
            for line in result.detail.lines() {
                println!("  {line}");
            }
            if !matches!(result.level, probe::TestLevel::Success) {
                failures += 1;
            }
        }
    }
    println!(
        "
自检完成：{} 个测试，{} 个非成功结果",
        expected, failures
    );
}

fn app_icon() -> Arc<egui::IconData> {
    const SIZE: u32 = 256;
    const RGBA: &[u8] = include_bytes!("../assets/icon_256.rgba");
    debug_assert_eq!(RGBA.len(), (SIZE * SIZE * 4) as usize);
    Arc::new(egui::IconData {
        rgba: RGBA.to_vec(),
        width: SIZE,
        height: SIZE,
    })
}

fn run_fatal(message: &str) {
    struct FatalApp {
        message: String,
    }
    impl eframe::App for FatalApp {
        fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_space(24.0);
                ui.heading("YoyoRelay 无法启动");
                ui.add_space(10.0);
                ui.label(&self.message);
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui.button("复制错误").clicked() {
                        ctx.copy_text(self.message.clone());
                    }
                    if ui.button("退出").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 420.0])
            .with_decorations(false)
            .with_title("YoyoRelay")
            .with_icon(app_icon()),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    let message = message.to_string();
    let _ = eframe::run_native(
        "YoyoRelay",
        options,
        Box::new(move |cc| {
            ui::theme::install_fonts(&cc.egui_ctx);
            Ok(Box::new(FatalApp { message }))
        }),
    );
}
