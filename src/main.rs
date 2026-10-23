#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod balance;
mod config;
mod contributors;
mod db;
mod fx;
mod gateway;
mod identity;
mod instance;
mod netinfo;
mod plugins;
mod probe;
mod protocol;
mod runtime;
mod scheduler;
mod snapshot;
mod taskbar;
mod tray;
mod ui;
mod update;
mod windowfx;

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
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
        eprintln!("{} 启动失败：{err:#}", identity::APP_NAME);
        run_fatal(&format!("{err:#}"));
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Autostart management does not need the single-instance lock: it must
    // also work while YoyoRelay is already running. Persisting the flag keeps
    // the next launch from re-enabling it from a stale config.
    if let Some(index) = args.iter().position(|a| a == "--autostart") {
        let enabled = args
            .get(index + 1)
            .map(|value| !matches!(value.as_str(), "off" | "false" | "0" | "disable"))
            .unwrap_or(true);
        autostart::set_enabled(enabled)?;
        if let Ok(store) = ConfigStore::open() {
            if let Ok(outcome) = store.load() {
                let mut config = outcome.config;
                if config.ui.autostart != enabled {
                    config.ui.autostart = enabled;
                    let _ = store.save(&mut config);
                }
            }
        }
        println!(
            "autostart {}: {}",
            if enabled { "enabled" } else { "disabled" },
            std::env::current_exe()?.display()
        );
        return Ok(());
    }

    let store = Arc::new(ConfigStore::open().context("初始化配置目录失败")?);
    let signals = Arc::new(instance::Signals::default());
    let _instance_guard = match instance::acquire(&store.paths.config_dir, signals.clone())? {
        instance::Instance::Primary(guard) => guard,
        instance::Instance::AlreadyRunning => {
            println!("{} 已在运行，已请求显示已有窗口。", identity::APP_NAME);
            return Ok(());
        }
    };
    let outcome = store.load()?;
    let headless = args.iter().any(|a| a == "--headless");
    let selftest = args.iter().any(|a| a == "--selftest");
    let speedtest = args.iter().any(|a| a == "--speedtest");
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

    if speedtest {
        run_speedtest(&outcome.config, &cmd_tx, snap_rx);
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
            .with_min_inner_size([900.0, 620.0])
            .with_decorations(false)
            .with_transparent(false)
            .with_resizable(true)
            .with_clamp_size_to_monitor_size(true)
            .with_title(identity::APP_NAME)
            .with_icon(app_icon()),
        renderer: eframe::Renderer::Glow,
        vsync: true,
        ..Default::default()
    };

    let result = eframe::run_native(
        identity::APP_NAME,
        options,
        Box::new(move |cc| {
            Ok(Box::new(ui::App::new(
                cc,
                store,
                outcome.config,
                cmd_tx,
                snap_rx,
                initial_page,
                signals,
            )))
        }),
    );
    if let Err(err) = result {
        return Err(anyhow!("窗口运行失败：{err}"));
    }
    Ok(())
}

fn run_headless(snap_rx: std::sync::mpsc::Receiver<Arc<snapshot::UiSnapshot>>) {
    println!(
        "{} headless 模式已启动；使用 curl 访问本地端点，按 Ctrl+C 退出。",
        identity::APP_NAME
    );
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

/// Run the streaming speed test for every provider and print the results.
/// Diagnostics only: nothing is written to the usage database.
fn run_speedtest(
    config: &config::AppConfig,
    cmd_tx: &tokio::sync::mpsc::UnboundedSender<runtime::UiCommand>,
    snap_rx: std::sync::mpsc::Receiver<Arc<snapshot::UiSnapshot>>,
) {
    use runtime::UiCommand;

    let mut snapshot = match snap_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(snapshot) => snapshot,
        Err(_) => {
            eprintln!("测速：等待运行时超时");
            return;
        }
    };
    let mut expected = Vec::new();
    for provider in &config.providers {
        let key = format!("speed:{}", provider.id);
        let _ = cmd_tx.send(UiCommand::SpeedTestProvider(provider.id.clone()));
        expected.push(key);
    }
    if expected.is_empty() {
        println!("测速：配置中没有供应商");
        return;
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while std::time::Instant::now() < deadline {
        if let Ok(s) = snap_rx.recv_timeout(std::time::Duration::from_millis(250)) {
            snapshot = s;
        }
        if expected.iter().all(|key| snapshot.tests.contains_key(key)) {
            break;
        }
    }

    let mut failures = 0usize;
    for key in &expected {
        match snapshot.tests.get(key) {
            Some(result) => {
                println!(
                    "[{}] {} → {}（{} ms）",
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
            None => {
                println!("[speed] {key} → 无结果（超时或运行时已退出）");
                failures += 1;
            }
        }
    }
    println!(
        "\n测速完成：{} 个供应商，{} 个未成功",
        expected.len(),
        failures
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
                ui.heading(format!("{} 无法启动", identity::APP_NAME));
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
            .with_title(identity::APP_NAME)
            .with_icon(app_icon()),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    let message = message.to_string();
    let _ = eframe::run_native(
        identity::APP_NAME,
        options,
        Box::new(move |cc| {
            ui::theme::install_fonts(&cc.egui_ctx);
            Ok(Box::new(FatalApp { message }))
        }),
    );
}
