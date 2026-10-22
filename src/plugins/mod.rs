//! JavaScript plugin host.
//!
//! Every plugin owns a dedicated OS thread with its own Boa `Context`. The
//! context is `!Send`, so the host only ever holds an `mpsc::Sender` and the
//! worker thread performs parsing, execution, event dispatch and teardown.
//! Dropping the worker's `Context` is the unload operation; Boa's GC reclaims
//! everything that belongs to it.

pub mod docs;
mod worker;

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::config::PluginSettings;
use crate::snapshot::NoticeLevel;

const EVENT_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_LOGS: usize = 200;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub main: String,
    pub author: String,
    /// Informational list of events the plugin implements.
    pub events: Vec<String>,
}

impl Default for PluginManifest {
    fn default() -> Self {
        Self {
            name: String::new(),
            version: "1.0.0".to_string(),
            description: String::new(),
            main: "main.js".to_string(),
            author: String::new(),
            events: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginStatus {
    Disabled,
    Stopped,
    Running,
    Error,
}

impl PluginStatus {
    pub fn label(self) -> &'static str {
        match self {
            PluginStatus::Disabled => "已禁用",
            PluginStatus::Stopped => "已卸载",
            PluginStatus::Running => "运行中",
            PluginStatus::Error => "异常",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PluginLog {
    pub at: i64,
    pub level: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub dir: PathBuf,
    pub entry: PathBuf,
    pub data_dir: PathBuf,
    pub status: PluginStatus,
    pub enabled: bool,
    /// Event names the module actually exported.
    pub events: Vec<String>,
    pub last_error: Option<String>,
    pub logs: VecDeque<PluginLog>,
    pub calls: u64,
    pub loaded_at: i64,
}

impl PluginInfo {
    fn new(
        id: String,
        dir: PathBuf,
        entry: PathBuf,
        data_dir: PathBuf,
        manifest: &PluginManifest,
    ) -> Self {
        Self {
            name: if manifest.name.trim().is_empty() {
                id.clone()
            } else {
                manifest.name.clone()
            },
            version: if manifest.version.trim().is_empty() {
                "1.0.0".to_string()
            } else {
                manifest.version.clone()
            },
            description: manifest.description.clone(),
            author: manifest.author.clone(),
            id,
            dir,
            entry,
            data_dir,
            status: PluginStatus::Stopped,
            enabled: true,
            events: manifest.events.clone(),
            last_error: None,
            logs: VecDeque::new(),
            calls: 0,
            loaded_at: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginUiKind {
    Confirm,
    Prompt,
}

#[derive(Clone, Debug)]
pub struct PluginUiRequest {
    pub id: u64,
    pub plugin_id: String,
    pub plugin_name: String,
    pub kind: PluginUiKind,
    pub title: String,
    pub message: String,
    pub default_value: String,
}

#[derive(Clone, Debug, Default)]
pub struct PluginUiAnswer {
    pub accepted: bool,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct BeforeRequest {
    /// Replacement payload when a plugin returned one.
    pub payload: Option<Value>,
    /// Some plugin intercepted the request.
    pub block: Option<BlockResponse>,
}

#[derive(Clone, Debug)]
pub struct BlockResponse {
    pub status: u16,
    pub body: Value,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkerMeta {
    pub name: String,
    pub version: String,
    pub description: String,
    pub events: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct WorkerEnv {
    pub id: String,
    pub dir: PathBuf,
    pub data_dir: PathBuf,
    pub entry: PathBuf,
    pub endpoint: String,
    pub models: Vec<String>,
    pub settings: PluginSettings,
    pub http: reqwest::Client,
    pub handle: tokio::runtime::Handle,
    pub shared: Arc<Shared>,
}

pub(crate) enum WorkerCommand {
    Event {
        name: String,
        payload: Value,
        reply: Option<oneshot::Sender<Result<Option<Value>, String>>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

struct WorkerHandle {
    tx: mpsc::UnboundedSender<WorkerCommand>,
    join: Option<std::thread::JoinHandle<()>>,
}

pub(crate) struct Shared {
    infos: Mutex<BTreeMap<String, PluginInfo>>,
    ui: Mutex<UiQueue>,
    notices: Mutex<VecDeque<(NoticeLevel, String)>>,
}

struct UiQueue {
    next_id: AtomicU64,
    pending: BTreeMap<u64, (PluginUiRequest, oneshot::Sender<PluginUiAnswer>)>,
}

impl Shared {
    fn new() -> Self {
        Self {
            infos: Mutex::new(BTreeMap::new()),
            ui: Mutex::new(UiQueue {
                next_id: AtomicU64::new(1),
                pending: BTreeMap::new(),
            }),
            notices: Mutex::new(VecDeque::new()),
        }
    }

    pub(crate) fn log(&self, plugin_id: &str, level: &str, message: impl Into<String>) {
        let mut infos = self.infos.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(info) = infos.get_mut(plugin_id) {
            info.logs.push_back(PluginLog {
                at: crate::config::now_ts(),
                level: level.to_string(),
                message: message.into(),
            });
            while info.logs.len() > MAX_LOGS {
                info.logs.pop_front();
            }
            if level == "error" {
                info.last_error = info.logs.back().map(|log| log.message.clone());
            }
        }
    }

    pub(crate) fn count_call(&self, plugin_id: &str) {
        let mut infos = self.infos.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(info) = infos.get_mut(plugin_id) {
            info.calls += 1;
        }
    }

    pub(crate) fn notice(&self, level: NoticeLevel, message: impl Into<String>) {
        let mut notices = self.notices.lock().unwrap_or_else(|e| e.into_inner());
        notices.push_back((level, message.into()));
        while notices.len() > 30 {
            notices.pop_front();
        }
    }

    fn request_ui(
        &self,
        plugin_id: &str,
        plugin_name: &str,
        kind: PluginUiKind,
        title: String,
        message: String,
        default_value: String,
    ) -> oneshot::Receiver<PluginUiAnswer> {
        let (tx, rx) = oneshot::channel();
        let mut ui = self.ui.lock().unwrap_or_else(|e| e.into_inner());
        let id = ui.next_id.fetch_add(1, Ordering::SeqCst);
        ui.pending.insert(
            id,
            (
                PluginUiRequest {
                    id,
                    plugin_id: plugin_id.to_string(),
                    plugin_name: plugin_name.to_string(),
                    kind,
                    title,
                    message,
                    default_value,
                },
                tx,
            ),
        );
        rx
    }
}

// ---------------------------------------------------------------------------
// Host
// ---------------------------------------------------------------------------

pub struct PluginHost {
    plugins_dir: PathBuf,
    data_dir: PathBuf,
    shared: Arc<Shared>,
    workers: Mutex<BTreeMap<String, WorkerHandle>>,
    settings: RwLock<PluginSettings>,
    http: reqwest::Client,
    handle: tokio::runtime::Handle,
    endpoint: RwLock<(String, Vec<String>)>,
}

impl PluginHost {
    pub fn new(config_dir: &Path, handle: tokio::runtime::Handle) -> Arc<Self> {
        let plugins_dir = config_dir.join("plugins");
        let data_dir = config_dir.join("plugin-data");
        let _ = std::fs::create_dir_all(&plugins_dir);
        let _ = std::fs::create_dir_all(&data_dir);
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(90))
            .user_agent(concat!("YoyoRelay-Plugin/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let host = Arc::new(Self {
            plugins_dir,
            data_dir,
            shared: Arc::new(Shared::new()),
            workers: Mutex::new(BTreeMap::new()),
            settings: RwLock::new(PluginSettings::default()),
            http,
            handle,
            endpoint: RwLock::new((String::new(), Vec::new())),
        });
        host.scan();
        host
    }

    pub fn set_runtime_info(&self, endpoint: String, models: Vec<String>) {
        if let Ok(mut info) = self.endpoint.write() {
            *info = (endpoint, models);
        }
    }

    pub fn apply_settings(&self, settings: PluginSettings) {
        if let Ok(mut guard) = self.settings.write() {
            *guard = settings.clone();
        }
        let mut infos = self.shared.infos.lock().unwrap_or_else(|e| e.into_inner());
        for info in infos.values_mut() {
            info.enabled = !settings.disabled.contains(&info.id);
            if !info.enabled {
                info.status = PluginStatus::Disabled;
            } else if info.status == PluginStatus::Disabled {
                info.status = PluginStatus::Stopped;
            }
        }
    }

    pub fn settings(&self) -> PluginSettings {
        self.settings
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub fn views(&self) -> Vec<PluginInfo> {
        let infos = self.shared.infos.lock().unwrap_or_else(|e| e.into_inner());
        let mut views: Vec<_> = infos.values().cloned().collect();
        views.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        views
    }

    pub fn ui_request(&self) -> Option<PluginUiRequest> {
        let ui = self.shared.ui.lock().unwrap_or_else(|e| e.into_inner());
        ui.pending
            .values()
            .next()
            .map(|(request, _)| request.clone())
    }

    pub fn answer_ui(&self, id: u64, answer: PluginUiAnswer) -> bool {
        let mut ui = self.shared.ui.lock().unwrap_or_else(|e| e.into_inner());
        ui.pending
            .remove(&id)
            .map(|(_, tx)| tx.send(answer).is_ok())
            .unwrap_or(false)
    }

    pub fn drain_notices(&self) -> Vec<(NoticeLevel, String)> {
        let mut notices = self
            .shared
            .notices
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        notices.drain(..).collect()
    }

    /// Unload every worker on shutdown. Each worker drops its Boa context.
    pub async fn unload_all(&self) {
        let ids: Vec<String> = self
            .workers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        for id in ids {
            self.unload(&id).await;
        }
    }

    /// Reload every running plugin, e.g. after permission changes.
    pub async fn reload_all(&self) {
        let settings = self.settings();
        let ids: Vec<String> = self
            .views()
            .into_iter()
            .filter(|info| info.enabled && info.status == PluginStatus::Running)
            .map(|info| info.id)
            .collect();
        for id in ids {
            self.unload(&id).await;
            let _ = self.load(&id, &settings).await;
        }
    }

    /// Unload plugins the user disabled and load enabled ones that are not
    /// running yet.
    pub async fn sync_enabled(&self) {
        let settings = self.settings();
        let views = self.views();
        for info in views {
            if !info.enabled {
                self.unload(&info.id).await;
                let mut infos = self.shared.infos.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(slot) = infos.get_mut(&info.id) {
                    slot.status = PluginStatus::Disabled;
                }
            } else if info.status != PluginStatus::Running {
                if let Err(err) = self.load(&info.id, &settings).await {
                    tracing::warn!("plugin {} failed to load: {err}", info.id);
                }
            }
        }
    }

    pub fn log(&self, id: &str, level: &str, message: impl Into<String>) {
        self.shared.log(id, level, message);
    }

    /// Re-read the plugins directory. Existing runtime state is preserved.
    pub fn scan(&self) {
        let mut infos = self.shared.infos.lock().unwrap_or_else(|e| e.into_inner());
        let previous: BTreeMap<String, PluginInfo> = infos.clone();
        let mut discovered: BTreeMap<String, PluginInfo> = BTreeMap::new();

        let Ok(entries) = std::fs::read_dir(&self.plugins_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                let manifest_path = path.join("plugin.json");
                let mut manifest = if manifest_path.exists() {
                    std::fs::read_to_string(&manifest_path)
                        .ok()
                        .and_then(|text| serde_json::from_str::<PluginManifest>(&text).ok())
                        .unwrap_or_default()
                } else {
                    PluginManifest::default()
                };
                if manifest.main.trim().is_empty() {
                    manifest.main = "main.js".to_string();
                }
                let entry_path = path.join(manifest.main.trim());
                if !entry_path.is_file() {
                    continue;
                }
                let id = file_name.clone();
                let data_dir = self.data_dir.join(&id);
                let mut info = PluginInfo::new(id.clone(), path, entry_path, data_dir, &manifest);
                if let Some(old) = previous.get(&id) {
                    info.status = if info.enabled {
                        old.status
                    } else {
                        PluginStatus::Disabled
                    };
                    info.logs = old.logs.clone();
                    info.last_error = old.last_error.clone();
                    info.calls = old.calls;
                    info.loaded_at = old.loaded_at;
                    info.events = if old.events.is_empty() {
                        info.events
                    } else {
                        old.events.clone()
                    };
                }
                discovered.insert(id, info);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("js") {
                let id = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or(&file_name)
                    .to_string();
                let manifest = PluginManifest {
                    main: file_name.clone(),
                    ..PluginManifest::default()
                };
                let data_dir = self.data_dir.join(&id);
                let mut info =
                    PluginInfo::new(id.clone(), path.clone(), path.clone(), data_dir, &manifest);
                if let Some(old) = previous.get(&id) {
                    info.status = old.status;
                    info.logs = old.logs.clone();
                    info.last_error = old.last_error.clone();
                    info.calls = old.calls;
                    info.loaded_at = old.loaded_at;
                    info.events = old.events.clone();
                }
                discovered.insert(id, info);
            }
        }

        let settings = self
            .settings
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        for info in discovered.values_mut() {
            info.enabled = !settings.disabled.contains(&info.id);
            if !info.enabled {
                info.status = PluginStatus::Disabled;
            } else if matches!(info.status, PluginStatus::Disabled) {
                info.status = PluginStatus::Stopped;
            }
            let _ = std::fs::create_dir_all(&info.data_dir);
        }
        *infos = discovered;
    }

    pub async fn load_all_enabled(&self) {
        self.scan();
        let settings = self.settings();
        let ids: Vec<String> = self
            .views()
            .into_iter()
            .filter(|info| info.enabled && info.status != PluginStatus::Running)
            .map(|info| info.id)
            .collect();
        for id in ids {
            let _ = self.load(&id, &settings).await;
        }
    }

    pub async fn load(&self, id: &str, settings: &PluginSettings) -> Result<(), String> {
        if self
            .workers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(id)
        {
            return Ok(());
        }
        let info = {
            let infos = self.shared.infos.lock().unwrap_or_else(|e| e.into_inner());
            infos.get(id).cloned()
        }
        .ok_or_else(|| format!("插件 {id} 不存在"))?;
        if !info.enabled {
            return Err("插件已被禁用".into());
        }

        let (endpoint, models) = self
            .endpoint
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        let env = WorkerEnv {
            id: info.id.clone(),
            dir: info.dir.clone(),
            data_dir: info.data_dir.clone(),
            entry: info.entry.clone(),
            endpoint,
            models,
            settings: settings.clone(),
            http: self.http.clone(),
            handle: self.handle.clone(),
            shared: self.shared.clone(),
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let (init_tx, init_rx) = oneshot::channel();
        let shared = self.shared.clone();
        let join = std::thread::Builder::new()
            .name(format!("yoyo-plugin-{}", info.id))
            .spawn(move || {
                worker::run(env, rx, init_tx);
            })
            .map_err(|err| format!("无法创建插件线程：{err}"))?;

        let meta = match tokio::time::timeout(Duration::from_secs(15), init_rx).await {
            Ok(Ok(Ok(meta))) => meta,
            Ok(Ok(Err(err))) => {
                let _ = join.join();
                tracing::warn!("plugin {} failed to load: {err}", info.id);
                shared.log(&info.id, "error", err.clone());
                let mut infos = shared.infos.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(slot) = infos.get_mut(&info.id) {
                    slot.status = PluginStatus::Error;
                    slot.last_error = Some(err.clone());
                }
                return Err(err);
            }
            _ => {
                let _ = join.join();
                let err = "插件初始化超时".to_string();
                shared.log(&info.id, "error", err.clone());
                let mut infos = shared.infos.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(slot) = infos.get_mut(&info.id) {
                    slot.status = PluginStatus::Error;
                    slot.last_error = Some(err.clone());
                }
                return Err(err);
            }
        };

        self.workers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                id.to_string(),
                WorkerHandle {
                    tx,
                    join: Some(join),
                },
            );
        {
            let mut infos = self.shared.infos.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(slot) = infos.get_mut(id) {
                slot.name = if meta.name.trim().is_empty() {
                    info.name.clone()
                } else {
                    meta.name
                };
                if !meta.version.trim().is_empty() {
                    slot.version = meta.version;
                }
                if !meta.description.trim().is_empty() {
                    slot.description = meta.description;
                }
                slot.events = meta.events;
                slot.status = PluginStatus::Running;
                slot.last_error = None;
                slot.loaded_at = crate::config::now_ts();
            }
        }
        tracing::info!("plugin {} loaded", id);
        self.shared.log(id, "info", "插件已加载");
        Ok(())
    }

    pub async fn unload(&self, id: &str) {
        let handle = self
            .workers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        let Some(worker) = handle else {
            self.mark_stopped(id);
            return;
        };
        let (tx, rx) = oneshot::channel();
        let _ = worker.tx.send(WorkerCommand::Shutdown { reply: tx });
        let _ = tokio::time::timeout(Duration::from_secs(3), rx).await;
        drop(worker.tx);
        if let Some(join) = worker.join {
            // The thread owns the Context; it exits after shutting down. If it
            // is stuck in a native call we detach rather than blocking the UI.
            let _ = tokio::time::timeout(
                Duration::from_secs(1),
                tokio::task::spawn_blocking(move || {
                    let _ = join.join();
                }),
            )
            .await;
        }
        self.mark_stopped(id);
        self.shared.log(id, "info", "插件已卸载");
    }

    fn mark_stopped(&self, id: &str) {
        let mut infos = self.shared.infos.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(slot) = infos.get_mut(id) {
            if slot.enabled {
                slot.status = PluginStatus::Stopped;
            }
        }
    }

    pub async fn reload(&self, id: &str) -> Result<(), String> {
        self.unload(id).await;
        self.load(id, &self.settings()).await
    }

    fn senders(&self) -> Vec<(String, mpsc::UnboundedSender<WorkerCommand>)> {
        let workers = self.workers.lock().unwrap_or_else(|e| e.into_inner());
        workers
            .iter()
            .map(|(id, worker)| (id.clone(), worker.tx.clone()))
            .collect()
    }

    async fn send_event(
        &self,
        id: &str,
        tx: &mpsc::UnboundedSender<WorkerCommand>,
        event: &str,
        payload: Value,
        timeout: Duration,
    ) -> Option<Value> {
        let (reply_tx, reply_rx) = oneshot::channel();
        if tx
            .send(WorkerCommand::Event {
                name: event.to_string(),
                payload,
                reply: Some(reply_tx),
            })
            .is_err()
        {
            return None;
        }
        match tokio::time::timeout(timeout, reply_rx).await {
            Ok(Ok(Ok(value))) => value,
            Ok(Ok(Err(err))) => {
                self.shared.log(id, "error", format!("{event}: {err}"));
                None
            }
            Ok(Err(_)) => None,
            Err(_) => {
                self.shared.log(
                    id,
                    "warn",
                    format!("{event}: 超过 {} ms，已跳过", timeout.as_millis()),
                );
                None
            }
        }
    }

    /// Run `beforeRequest` on every running plugin. The payload is threaded
    /// through plugins so later ones see earlier modifications.
    pub async fn before_request(&self, mut request: Value) -> BeforeRequest {
        let mut outcome = BeforeRequest::default();
        for (id, tx) in self.senders() {
            let payload = request.get("payload").cloned().unwrap_or(Value::Null);
            let event = serde_json::json!({
                "event": "beforeRequest",
                "request": {
                    "id": request.get("id"),
                    "model": request.get("model"),
                    "stream": request.get("stream"),
                    "protocol": request.get("protocol"),
                    "path": request.get("path"),
                    "localKey": request.get("localKey"),
                    "payload": payload,
                }
            });
            let Some(result) = self
                .send_event(&id, &tx, "beforeRequest", event, EVENT_TIMEOUT)
                .await
            else {
                continue;
            };
            if result.get("block").and_then(Value::as_bool) == Some(true) {
                let status = result
                    .get("status")
                    .and_then(Value::as_u64)
                    .unwrap_or(403)
                    .clamp(100, 599) as u16;
                let body = result
                    .get("response")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"error": "blocked by plugin"}));
                outcome.block = Some(BlockResponse { status, body });
                return outcome;
            }
            if let Some(payload) = result.get("payload") {
                request["payload"] = payload.clone();
                outcome.payload = Some(payload.clone());
            } else if result.get("model").is_some() || result.get("messages").is_some() {
                // Convenience: returning the request body itself is treated as
                // a replacement payload.
                request["payload"] = result.clone();
                outcome.payload = Some(result);
            }
        }
        outcome
    }

    pub async fn after_response(&self, request: Value, response: Value) -> Option<Value> {
        let mut current = response;
        let mut changed = false;
        for (id, tx) in self.senders() {
            let event = serde_json::json!({
                "event": "afterResponse",
                "request": request,
                "response": current,
            });
            let Some(result) = self
                .send_event(&id, &tx, "afterResponse", event, EVENT_TIMEOUT)
                .await
            else {
                continue;
            };
            if let Some(response) = result.get("response") {
                current = response.clone();
                changed = true;
            } else if result.get("choices").is_some() || result.get("output").is_some() {
                current = result;
                changed = true;
            }
        }
        changed.then_some(current)
    }

    /// Fire-and-forget notification.
    pub fn notify(&self, event: &str, payload: Value) {
        for (id, tx) in self.senders() {
            let _ = &id;
            let _ = tx.send(WorkerCommand::Event {
                name: event.to_string(),
                payload: payload.clone(),
                reply: None,
            });
        }
    }

    /// Invoke a custom exported function (used by the plugin test button).
    pub async fn invoke(
        &self,
        id: &str,
        name: &str,
        payload: Value,
    ) -> Result<Option<Value>, String> {
        let tx = self
            .senders()
            .into_iter()
            .find(|(plugin_id, _)| plugin_id == id)
            .map(|(_, tx)| tx)
            .ok_or_else(|| "插件未运行".to_string())?;
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(WorkerCommand::Event {
            name: name.to_string(),
            payload,
            reply: Some(reply_tx),
        })
        .map_err(|_| "插件线程已退出".to_string())?;
        match tokio::time::timeout(Duration::from_secs(10), reply_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("插件线程没有返回结果".into()),
            Err(_) => Err("插件调用超时".into()),
        }
    }

    /// Write the bundled sample plugins into the plugins directory.
    pub fn install_samples(&self) -> Result<Vec<String>, String> {
        docs::install_samples(&self.plugins_dir)
    }
}

// Keep the UI-side instant import alive in non-test builds.
#[allow(dead_code)]
fn _event_timeout() -> Instant {
    Instant::now() + EVENT_TIMEOUT
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PLUGIN: &str = r#"
export const name = "Test Plugin";
export const version = "1.2.3";
export const description = "plugin host integration test";

export function beforeRequest(event) {
  const text = JSON.stringify(event.request.payload?.messages ?? "");
  if (text.includes("block")) {
    return { block: true, status: 418, response: { error: { message: "blocked" } } };
  }
  if (text.includes("rename")) {
    return { payload: { ...event.request.payload, model: "renamed" } };
  }
  return null;
}

export function afterResponse(event) {
  return { response: { ...event.response, tagged: true } };
}

export async function selfTest() {
  host.fs.writeText("a.txt", "hello");
  await host.sleep(5);
  const read = host.fs.readText("a.txt");
  return {
    read,
    b64: host.encoding.base64Encode("hi"),
    roundtrip: host.encoding.base64Decode(host.encoding.base64Encode("插件")),
    sha: host.crypto.sha256("x"),
    random: host.crypto.randomBytes(4).length
  };
}
"#;

    #[test]
    fn installs_sample_plugins_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        let first = docs::install_samples(&plugins).unwrap();
        assert_eq!(first.len(), 3);
        assert!(plugins.join("request-guard/main.js").is_file());
        assert!(plugins.join("hello-plugin/plugin.json").is_file());
        // Keep an existing file untouched.
        std::fs::write(plugins.join("toolkit/main.js"), "// custom").unwrap();
        docs::install_samples(&plugins).unwrap();
        assert_eq!(
            std::fs::read_to_string(plugins.join("toolkit/main.js")).unwrap(),
            "// custom"
        );
    }

    #[test]
    fn loads_plugins_dispatches_events_and_unloads() {
        let dir = tempfile::tempdir().unwrap();
        let plugins_dir = dir.path().join("plugins");
        let plugin_dir = plugins_dir.join("test");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("main.js"), TEST_PLUGIN).unwrap();

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let host = PluginHost::new(dir.path(), tokio::runtime::Handle::current());
            host.apply_settings(PluginSettings::default());
            host.load_all_enabled().await;
            let views = host.views();
            assert_eq!(views.len(), 1, "plugin should be discovered");
            assert_eq!(
                views[0].status,
                PluginStatus::Running,
                "{:?}",
                views[0].last_error
            );
            assert_eq!(views[0].version, "1.2.3");
            assert!(views[0].events.iter().any(|event| event == "beforeRequest"));

            // Interception.
            let blocked = host
                .before_request(serde_json::json!({
                    "id": "r1",
                    "model": "m",
                    "stream": false,
                    "payload": {"model": "m", "messages": [{"role": "user", "content": "block me"}]}
                }))
                .await;
            let block = blocked.block.expect("plugin should block");
            assert_eq!(block.status, 418);

            // Payload mutation.
            let modified = host
                .before_request(serde_json::json!({
                    "id": "r2",
                    "model": "m",
                    "stream": false,
                    "payload": {"model": "m", "messages": [{"role": "user", "content": "rename"}]}
                }))
                .await;
            assert!(modified.block.is_none());
            assert_eq!(
                modified
                    .payload
                    .unwrap()
                    .get("model")
                    .and_then(Value::as_str),
                Some("renamed")
            );

            // Response mutation.
            let response = host
                .after_response(
                    serde_json::json!({"id": "r3", "model": "m"}),
                    serde_json::json!({"choices": []}),
                )
                .await
                .expect("plugin should modify the response");
            assert_eq!(response.get("tagged").and_then(Value::as_bool), Some(true));

            // Async API coverage.
            let result = host
                .invoke("test", "selfTest", serde_json::json!({"event": "selfTest"}))
                .await
                .expect("selfTest should run")
                .expect("selfTest should return a value");
            assert_eq!(result.get("read").and_then(Value::as_str), Some("hello"));
            assert_eq!(result.get("b64").and_then(Value::as_str), Some("aGk="));
            assert_eq!(
                result.get("roundtrip").and_then(Value::as_str),
                Some("插件")
            );
            assert_eq!(
                result.get("sha").and_then(Value::as_str),
                Some("2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881")
            );
            assert_eq!(result.get("random").and_then(Value::as_u64), Some(8));

            host.unload("test").await;
            let views = host.views();
            assert_eq!(views[0].status, PluginStatus::Stopped);
        });
    }
}
