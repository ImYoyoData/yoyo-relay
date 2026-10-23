//! JavaScript plugin host.
//!
//! Every plugin owns a dedicated OS thread with its own Boa `Context`. The
//! context is `!Send`, so the host only ever holds an `mpsc::Sender` and the
//! worker thread performs parsing, execution, event dispatch and teardown.
//! Dropping the worker's `Context` is the unload operation; Boa's GC reclaims
//! everything that belongs to it.

pub mod docs;
mod worker;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::config::{
    now_ts, Account, BalanceConfig, BalanceMode, ModelRoute, PluginSettings, Protocol, Provider,
    ProviderKind, RouteMode,
};
use crate::snapshot::NoticeLevel;

const EVENT_TIMEOUT: Duration = Duration::from_secs(3);
pub const MAX_LOGS: usize = 500;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A provider a plugin offers to the relay. The relay creates a temporary
/// provider, a virtual account and one route per advertised model.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct PluginProviderSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `openai-chat` (default), `anthropic-messages` or `openai-responses`.
    pub protocol: String,
    /// Client model names served by this provider.
    pub models: Vec<String>,
    #[serde(default)]
    pub priority: i32,
    /// Whether `onProviderRequest` emits SSE frames for streaming clients.
    pub streaming: bool,
}

impl Default for PluginProviderSpec {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            protocol: "openai-chat".to_string(),
            models: Vec::new(),
            priority: 0,
            streaming: true,
        }
    }
}

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
    /// Providers advertised by the module's `providers` export.
    pub providers: Vec<PluginProviderSpec>,
    pub last_error: Option<String>,
    /// Number of buffered log lines; the lines themselves live in
    /// `Shared::logs` and are only copied when a console is open.
    pub log_count: usize,
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
            providers: Vec::new(),
            last_error: None,
            log_count: 0,
            calls: 0,
            loaded_at: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginUiKind {
    Confirm,
    Prompt,
    Alert,
}

#[derive(Clone, Debug)]
pub struct PluginConsoleView {
    pub id: String,
    pub name: String,
    pub logs: Vec<PluginLog>,
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
    pub providers: Vec<PluginProviderSpec>,
}

/// Runtime-only providers contributed by running plugins. They are merged into
/// the gateway's effective config and never written to the user's config file.
#[derive(Clone, Debug, Default)]
pub struct DynamicEntries {
    pub providers: Vec<Provider>,
    pub accounts: Vec<Account>,
    pub routes: Vec<ModelRoute>,
    pub provider_plugins: HashMap<String, String>,
    /// Provider ids whose plugin handler emits SSE frames.
    pub streaming: HashSet<String>,
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
    pub http_local: reqwest::Client,
    pub handle: tokio::runtime::Handle,
    pub shared: Arc<Shared>,
}

pub(crate) enum WorkerCommand {
    Event {
        name: String,
        payload: Value,
        reply: Option<oneshot::Sender<Result<Option<Value>, String>>>,
    },
    /// Deliver one model request to the plugin's `onProviderRequest` handler.
    ProviderRequest {
        payload: Value,
        stream: bool,
        /// Present for streaming requests; the worker forwards raw SSE bytes.
        chunks: Option<mpsc::UnboundedSender<Result<bytes::Bytes, String>>>,
        /// Fires as soon as the worker confirms the handler exists.
        start: Option<oneshot::Sender<bool>>,
        /// Present for non-streaming requests.
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
    /// Per-plugin log ring buffers (newest `MAX_LOGS` lines).
    logs: Mutex<BTreeMap<String, VecDeque<PluginLog>>>,
    /// Plugin whose console is currently open; only then are logs copied into
    /// the snapshot.
    console_target: Mutex<Option<String>>,
    ui: Mutex<UiQueue>,
    notices: Mutex<VecDeque<(NoticeLevel, String)>>,
    dynamic_version: AtomicU64,
}

struct UiQueue {
    next_id: AtomicU64,
    pending: BTreeMap<u64, (PluginUiRequest, oneshot::Sender<PluginUiAnswer>)>,
}

impl Shared {
    fn new() -> Self {
        Self {
            infos: Mutex::new(BTreeMap::new()),
            logs: Mutex::new(BTreeMap::new()),
            console_target: Mutex::new(None),
            ui: Mutex::new(UiQueue {
                next_id: AtomicU64::new(1),
                pending: BTreeMap::new(),
            }),
            notices: Mutex::new(VecDeque::new()),
            dynamic_version: AtomicU64::new(1),
        }
    }

    fn bump_dynamic(&self) {
        self.dynamic_version.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn log(&self, plugin_id: &str, level: &str, message: impl Into<String>) {
        let message = message.into();
        let (count, last) = {
            let mut logs = self.logs.lock().unwrap_or_else(|e| e.into_inner());
            let entry = logs.entry(plugin_id.to_string()).or_default();
            entry.push_back(PluginLog {
                at: crate::config::now_ts(),
                level: level.to_string(),
                message: message.clone(),
            });
            while entry.len() > MAX_LOGS {
                entry.pop_front();
            }
            (entry.len(), entry.back().map(|log| log.message.clone()))
        };
        let mut infos = self.infos.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(info) = infos.get_mut(plugin_id) {
            info.log_count = count;
            if level == "error" {
                info.last_error = last;
            }
        }
    }

    fn log_snapshot(&self, plugin_id: &str) -> Vec<PluginLog> {
        self.logs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(plugin_id)
            .map(|logs| logs.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn clear_logs(&self, plugin_id: &str) {
        self.logs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(plugin_id);
        let mut infos = self.infos.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(info) = infos.get_mut(plugin_id) {
            info.log_count = 0;
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
    loading: Mutex<std::collections::HashSet<String>>,
    settings: RwLock<PluginSettings>,
    http: reqwest::Client,
    http_local: reqwest::Client,
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
        let http_local = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(90))
            .no_proxy()
            .user_agent(concat!("YoyoRelay-Plugin/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let host = Arc::new(Self {
            plugins_dir,
            data_dir,
            shared: Arc::new(Shared::new()),
            workers: Mutex::new(BTreeMap::new()),
            loading: Mutex::new(std::collections::HashSet::new()),
            settings: RwLock::new(PluginSettings::default()),
            http,
            http_local,
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
        drop(infos);
        self.shared.bump_dynamic();
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

    /// Build temporary providers/accounts/routes for every running plugin.
    pub fn dynamic_version(&self) -> u64 {
        self.shared.dynamic_version.load(Ordering::SeqCst)
    }

    pub fn dynamic_entries(&self) -> DynamicEntries {
        let infos = self.shared.infos.lock().unwrap_or_else(|e| e.into_inner());
        let mut entries = DynamicEntries::default();
        for info in infos.values() {
            if info.status != PluginStatus::Running {
                continue;
            }
            for (index, spec) in info.providers.iter().enumerate() {
                let models: Vec<String> = spec
                    .models
                    .iter()
                    .map(|model| model.trim().to_string())
                    .filter(|model| !model.is_empty())
                    .collect();
                if models.is_empty() {
                    continue;
                }
                let protocol = match spec.protocol.as_str() {
                    "anthropic" | "anthropic-messages" => Protocol::AnthropicMessages,
                    "responses" | "openai-responses" => Protocol::OpenAiResponses,
                    _ => Protocol::OpenAiChat,
                };
                let provider_id = format!("plugin:{}:{index}", info.id);
                let account_id = format!("plugin:{}:{index}:account", info.id);
                let name = if spec.name.trim().is_empty() {
                    format!("{} · 插件供应商", info.name)
                } else {
                    spec.name.trim().to_string()
                };
                entries
                    .provider_plugins
                    .insert(provider_id.clone(), info.id.clone());
                if spec.streaming {
                    entries.streaming.insert(provider_id.clone());
                }
                entries.providers.push(Provider {
                    id: provider_id.clone(),
                    name: name.clone(),
                    kind: ProviderKind::Custom,
                    base_url: format!("plugin://{}", info.id),
                    auth: crate::config::AuthConfig {
                        mode: crate::config::AuthMode::None,
                        ..crate::config::AuthConfig::default()
                    },
                    static_protocol: protocol,
                    test_model: models.first().cloned(),
                    created_at: now_ts(),
                    ..Provider::default()
                });
                entries.accounts.push(Account {
                    id: account_id,
                    provider_id: provider_id.clone(),
                    name: format!("{name} · 虚拟账号"),
                    priority: 0,
                    enabled: true,
                    // 0 = unlimited concurrency for the virtual account.
                    max_concurrency: 0,
                    quota: None,
                    cooldown_override_secs: None,
                    api_key: String::new(),
                    balance: BalanceConfig {
                        mode: BalanceMode::Unavailable,
                        ..BalanceConfig::default()
                    },
                    created_at: now_ts(),
                });
                for (model_index, model) in models.iter().enumerate() {
                    entries.routes.push(ModelRoute {
                        id: format!("plugin:{}:{index}:route:{model_index}", info.id),
                        client_model: model.clone(),
                        provider_id: provider_id.clone(),
                        upstream_model: model.clone(),
                        protocol: Some(protocol),
                        mode: RouteMode::Adapt,
                        priority: spec.priority,
                        capabilities: vec!["chat".to_string()],
                        max_context_tokens: None,
                        fallback_client_model: None,
                        input_price_per_mtok: 0.0,
                        output_price_per_mtok: 0.0,
                        pricing: Default::default(),
                        enabled: true,
                        created_at: now_ts(),
                    });
                }
            }
        }
        entries
    }

    pub fn set_console_target(&self, id: Option<String>) {
        let mut target = self
            .shared
            .console_target
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *target = id;
    }

    /// Logs for the plugin whose console is open, if any.
    pub fn console_view(&self) -> Option<PluginConsoleView> {
        let id = self
            .shared
            .console_target
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()?;
        let name = self
            .shared
            .infos
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .map(|info| info.name.clone())?;
        Some(PluginConsoleView {
            logs: self.shared.log_snapshot(&id),
            id,
            name,
        })
    }

    pub fn clear_logs(&self, id: &str) {
        self.shared.clear_logs(id);
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
                    info.last_error = old.last_error.clone();
                    info.calls = old.calls;
                    info.loaded_at = old.loaded_at;
                    info.events = if old.events.is_empty() {
                        info.events
                    } else {
                        old.events.clone()
                    };
                    info.providers = old.providers.clone();
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
                let root = path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| self.plugins_dir.clone());
                let mut info = PluginInfo::new(id.clone(), root, path.clone(), data_dir, &manifest);
                if let Some(old) = previous.get(&id) {
                    info.status = old.status;
                    info.last_error = old.last_error.clone();
                    info.calls = old.calls;
                    info.loaded_at = old.loaded_at;
                    info.events = old.events.clone();
                    info.providers = old.providers.clone();
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
        drop(infos);
        // Lock order is logs -> infos everywhere; never hold `infos` while
        // waiting for `logs` or a plugin logging concurrently could deadlock.
        let counts: BTreeMap<String, usize> = {
            let logs = self.shared.logs.lock().unwrap_or_else(|e| e.into_inner());
            logs.iter()
                .map(|(id, entries)| (id.clone(), entries.len()))
                .collect()
        };
        let mut infos = self.shared.infos.lock().unwrap_or_else(|e| e.into_inner());
        for info in infos.values_mut() {
            info.log_count = counts.get(&info.id).copied().unwrap_or(0);
        }
        drop(infos);
        self.shared.bump_dynamic();
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
        {
            let mut loading = self.loading.lock().unwrap_or_else(|e| e.into_inner());
            if !loading.insert(id.to_string()) {
                return Err("插件正在加载中".into());
            }
        }
        let result = self.load_inner(id, settings).await;
        self.loading
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        result
    }

    async fn load_inner(&self, id: &str, settings: &PluginSettings) -> Result<(), String> {
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
            http_local: self.http_local.clone(),
            handle: self.handle.clone(),
            shared: self.shared.clone(),
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let events = tx.clone();
        let (init_tx, init_rx) = oneshot::channel();
        let shared = self.shared.clone();
        let join = std::thread::Builder::new()
            .name(format!("yoyo-plugin-{}", info.id))
            .spawn(move || {
                worker::run(env, rx, init_tx, events);
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
                slot.providers = meta.providers;
                slot.status = PluginStatus::Running;
                slot.last_error = None;
                slot.loaded_at = crate::config::now_ts();
            }
        }
        tracing::info!("plugin {} loaded", id);
        self.shared.log(id, "info", "插件已加载");
        self.shared.bump_dynamic();
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
        self.shared.bump_dynamic();
    }

    /// Stop a plugin and delete its files. `keep_data` keeps
    /// `plugin-data/<id>`; otherwise that directory is removed as well.
    pub async fn uninstall(&self, id: &str, keep_data: bool) -> Result<Vec<PathBuf>, String> {
        let info = {
            let infos = self.shared.infos.lock().unwrap_or_else(|e| e.into_inner());
            infos.get(id).cloned()
        }
        .ok_or_else(|| format!("插件 {id} 不存在"))?;
        self.unload(id).await;

        let mut removed = Vec::new();
        if info.dir != self.plugins_dir && info.dir.starts_with(&self.plugins_dir) {
            std::fs::remove_dir_all(&info.dir)
                .map_err(|err| format!("删除插件目录 {} 失败：{err}", info.dir.display()))?;
            removed.push(info.dir.clone());
        } else if info.entry.starts_with(&self.plugins_dir) {
            std::fs::remove_file(&info.entry)
                .map_err(|err| format!("删除插件文件 {} 失败：{err}", info.entry.display()))?;
            removed.push(info.entry.clone());
        } else {
            return Err("插件路径不在插件目录内，已停止但未删除文件".into());
        }

        if !keep_data && info.data_dir.starts_with(&self.data_dir) && info.data_dir.exists() {
            std::fs::remove_dir_all(&info.data_dir)
                .map_err(|err| format!("删除插件数据 {} 失败：{err}", info.data_dir.display()))?;
            removed.push(info.data_dir.clone());
        }

        self.shared
            .logs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        self.shared
            .infos
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        self.scan();
        self.shared.bump_dynamic();
        Ok(removed)
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

    /// Call `onProviderRequest` for a non-streaming virtual provider request.
    /// The plugin returns either a response body or `{ status, body }`.
    pub async fn call_provider(
        &self,
        plugin_id: &str,
        payload: Value,
        timeout: Duration,
    ) -> Result<(u16, Value), String> {
        let tx = self
            .senders()
            .into_iter()
            .find(|(id, _)| id == plugin_id)
            .map(|(_, tx)| tx)
            .ok_or_else(|| "插件未运行".to_string())?;
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(WorkerCommand::ProviderRequest {
            payload,
            stream: false,
            chunks: None,
            start: None,
            reply: Some(reply_tx),
        })
        .map_err(|_| "插件线程已退出".to_string())?;
        let result = match tokio::time::timeout(timeout, reply_rx).await {
            Ok(Ok(result)) => result?,
            Ok(Err(_)) => return Err("插件线程没有返回响应".into()),
            Err(_) => return Err(format!("插件响应超时（{}s）", timeout.as_secs())),
        };
        let Some(result) = result else {
            return Err("onProviderRequest 没有返回响应".into());
        };
        let status = result
            .get("status")
            .and_then(Value::as_u64)
            .unwrap_or(200)
            .clamp(100, 599) as u16;
        if let Some(body) = result.get("body") {
            return Ok((status, body.clone()));
        }
        if result.get("choices").is_some()
            || result.get("output").is_some()
            || result.get("content").is_some()
            || result.get("error").is_some()
        {
            return Ok((status, result));
        }
        Err("onProviderRequest 返回值缺少 body 或响应对象".into())
    }

    /// Open a streaming virtual-provider response. The plugin emits raw SSE
    /// frames for its declared protocol; the gateway feeds them through the
    /// normal protocol translator.
    pub async fn call_provider_stream(
        &self,
        plugin_id: &str,
        payload: Value,
    ) -> Result<mpsc::UnboundedReceiver<Result<bytes::Bytes, String>>, String> {
        let tx = self
            .senders()
            .into_iter()
            .find(|(id, _)| id == plugin_id)
            .map(|(_, tx)| tx)
            .ok_or_else(|| "插件未运行".to_string())?;
        let (chunk_tx, chunk_rx) = mpsc::unbounded_channel();
        let (start_tx, start_rx) = oneshot::channel();
        tx.send(WorkerCommand::ProviderRequest {
            payload,
            stream: true,
            chunks: Some(chunk_tx),
            start: Some(start_tx),
            reply: None,
        })
        .map_err(|_| "插件线程已退出".to_string())?;
        match tokio::time::timeout(Duration::from_secs(5), start_rx).await {
            Ok(Ok(true)) => Ok(chunk_rx),
            Ok(Ok(false)) => Err("插件没有导出 onProviderRequest".into()),
            Ok(Err(_)) => Err("插件线程没有确认请求".into()),
            Err(_) => Err("插件确认请求超时".into()),
        }
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

    /// Ask a running plugin for its current model catalogue. The plugin can
    /// implement `onListModels()` and return a provider array, a
    /// `{ providers: [...] }` object, or a plain string array for its first
    /// provider. The result replaces the provider specs and refreshes the
    /// temporary providers/routes.
    pub async fn refresh_models(&self, id: &str) -> Result<usize, String> {
        let current = {
            let infos = self.shared.infos.lock().unwrap_or_else(|e| e.into_inner());
            infos
                .get(id)
                .map(|info| info.providers.clone())
                .unwrap_or_default()
        };
        let value = self
            .invoke(
                id,
                "onListModels",
                serde_json::json!({"event": "onListModels"}),
            )
            .await?
            .ok_or_else(|| "插件未实现 onListModels，或没有返回模型".to_string())?;
        let specs = worker::normalize_provider_specs(parse_model_specs(value, current)?);
        if specs.is_empty() {
            return Err("onListModels 没有返回可用的模型".into());
        }
        let count: usize = specs.iter().map(|spec| spec.models.len()).sum();
        {
            let mut infos = self.shared.infos.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(info) = infos.get_mut(id) {
                info.providers = specs;
            }
        }
        self.shared
            .log(id, "info", format!("模型列表已刷新：{count} 个模型"));
        self.shared.bump_dynamic();
        Ok(count)
    }

    /// Write the bundled sample plugins into the plugins directory.
    pub fn install_samples(&self) -> Result<Vec<String>, String> {
        docs::install_samples(&self.plugins_dir)
    }
}

fn parse_model_specs(
    value: Value,
    current: Vec<PluginProviderSpec>,
) -> Result<Vec<PluginProviderSpec>, String> {
    let array = match &value {
        Value::Array(items) => Some(items.clone()),
        Value::Object(map) => map.get("providers").and_then(Value::as_array).cloned(),
        _ => None,
    };
    if let Some(items) = array {
        if items.iter().all(Value::is_string) {
            let models: Vec<String> = items
                .into_iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect();
            if current.is_empty() {
                return Ok(vec![PluginProviderSpec {
                    name: "插件供应商".into(),
                    models,
                    ..PluginProviderSpec::default()
                }]);
            }
            let mut specs = current;
            specs[0].models = models;
            return Ok(specs);
        }
        let mut specs: Vec<PluginProviderSpec> = serde_json::from_value(Value::Array(items))
            .map_err(|err| format!("onListModels 返回的供应商格式无效：{err}"))?;
        for (index, spec) in specs.iter_mut().enumerate() {
            if let Some(previous) = current.get(index) {
                if spec.name.trim().is_empty() {
                    spec.name = previous.name.clone();
                }
                if spec.description.trim().is_empty() {
                    spec.description = previous.description.clone();
                }
            }
        }
        return Ok(specs);
    }
    if value.get("models").is_some() {
        let mut spec: PluginProviderSpec = serde_json::from_value(value)
            .map_err(|err| format!("onListModels 返回的供应商格式无效：{err}"))?;
        if let Some(previous) = current.first() {
            if spec.name.trim().is_empty() {
                spec.name = previous.name.clone();
            }
        }
        return Ok(vec![spec]);
    }
    Err("onListModels 需要返回字符串数组、供应商数组或 { providers } 对象".into())
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
        assert_eq!(first.len(), 4);
        assert!(plugins.join("request-guard/main.js").is_file());
        assert!(plugins.join("hello-plugin/plugin.json").is_file());
        assert!(plugins.join("echo-provider/main.js").is_file());
        // Keep an existing file untouched.
        std::fs::write(plugins.join("toolkit/main.js"), "// custom").unwrap();
        docs::install_samples(&plugins).unwrap();
        assert_eq!(
            std::fs::read_to_string(plugins.join("toolkit/main.js")).unwrap(),
            "// custom"
        );

        let exports = dir.path().join("exports");
        std::fs::create_dir_all(&exports).unwrap();
        let skill = docs::export_skill(&exports).unwrap();
        assert!(skill.join("SKILL.md").is_file());
        assert!(skill.join("API.md").is_file());
        assert!(skill.join("examples/echo-provider/main.js").is_file());
        let skill_text = std::fs::read_to_string(skill.join("SKILL.md")).unwrap();
        assert!(skill_text.contains("onProviderRequest"));
    }

    #[test]
    fn host_http_and_process_apis_work() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for index in 0..2 {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut buffer = [0u8; 2048];
                    let _ = stream.read(&mut buffer);
                    let response: &[u8] = if index == 0 {
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                    } else {
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 15\r\nConnection: close\r\n\r\ndata: {\"x\":1}\n\n"
                    };
                    let _ = stream.write_all(response);
                }
            }
        });

        let (command, args) = if cfg!(windows) {
            ("cmd", r#"["/C","echo hi"]"#)
        } else {
            ("sh", r#"["-c","echo hi"]"#)
        };
        let code = format!(
            r#"
export const name = "Net Process Test";
export async function netTest() {{
  const response = await host.http.request({{ url: "http://127.0.0.1:{port}/" }});
  return {{ status: response.status, body: response.body }};
}}
export async function procTest() {{
  return await host.process.run({{ command: "{command}", args: {args}, timeoutSecs: 10 }});
}}
export async function sseTest() {{
  let seen = 0;
  const result = await host.net.sse(
    {{ url: "http://127.0.0.1:{port}/" }},
    (event) => {{ seen += 1; }}
  );
  return {{ status: result.status, events: result.events, seen }};
}}
export function storageTest() {{
  host.storage.set("k", {{ a: 1 }});
  host.fs.writeBase64("b.bin", "aGk=");
  return {{
    value: host.storage.get("k"),
    keys: host.storage.keys(),
    now: host.time.now(),
    iso: host.time.nowIso(),
    uuid: host.util.uuid(),
    equal: host.crypto.constantTimeEqual("a", "a"),
    base64: host.fs.readBase64("b.bin"),
  }};
}}
"#
        );

        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("plugins").join("io");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("main.js"), code).unwrap();

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let host = PluginHost::new(dir.path(), tokio::runtime::Handle::current());
            let settings = PluginSettings {
                allow_network: true,
                allow_process: true,
                disabled: Vec::new(),
            };
            host.apply_settings(settings.clone());
            host.load_all_enabled().await;
            let views = host.views();
            assert_eq!(
                views[0].status,
                PluginStatus::Running,
                "{:?}",
                views[0].last_error
            );

            let net = host
                .invoke("io", "netTest", serde_json::json!({}))
                .await
                .expect("netTest")
                .expect("value");
            assert_eq!(net.get("status").and_then(Value::as_u64), Some(200));
            assert_eq!(net.get("body").and_then(Value::as_str), Some("ok"));

            let sse = host
                .invoke("io", "sseTest", serde_json::json!({}))
                .await
                .expect("sseTest")
                .expect("value");
            assert_eq!(sse.get("status").and_then(Value::as_u64), Some(200));
            assert_eq!(sse.get("events").and_then(Value::as_u64), Some(1));
            assert_eq!(sse.get("seen").and_then(Value::as_u64), Some(1));

            let process = host
                .invoke("io", "procTest", serde_json::json!({}))
                .await
                .expect("procTest")
                .expect("value");
            assert_eq!(process.get("success").and_then(Value::as_bool), Some(true));
            assert!(process
                .get("stdout")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("hi"));

            let extra = host
                .invoke("io", "storageTest", serde_json::json!({}))
                .await
                .expect("storageTest")
                .expect("value");
            assert_eq!(extra.pointer("/value/a").and_then(Value::as_u64), Some(1));
            assert_eq!(extra.pointer("/keys/0").and_then(Value::as_str), Some("k"));
            assert!(extra.get("now").and_then(Value::as_f64).unwrap_or(0.0) > 0.0);
            assert!(extra
                .get("iso")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains('T'));
            assert_eq!(
                extra.get("uuid").and_then(Value::as_str).map(str::len),
                Some(36)
            );
            assert_eq!(extra.get("equal").and_then(Value::as_bool), Some(true));
            assert_eq!(extra.get("base64").and_then(Value::as_str), Some("aGk="));

            host.unload_all().await;
        });
    }

    #[test]
    fn websocket_client_and_server_apis_work() {
        use futures_util::{SinkExt, StreamExt};

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            // Plain echo server for the client half of the test.
            let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let echo_port = echo.local_addr().unwrap().port();
            tokio::spawn(async move {
                if let Ok((stream, _)) = echo.accept().await {
                    if let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await {
                        while let Some(Ok(message)) = ws.next().await {
                            if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
                                let reply = format!("echo:{text}");
                                let _ = ws
                                    .send(tokio_tungstenite::tungstenite::Message::Text(
                                        reply.into(),
                                    ))
                                    .await;
                            }
                        }
                    }
                }
            });

            let code = format!(
                r#"
export const name = "WS Test";
export const version = "1.0.0";

export async function clientTest() {{
  const conn = await host.ws.connect({{ url: "ws://127.0.0.1:{echo_port}/echo" }});
  host.ws.send(conn.id, "hello");
  const reply = await host.ws.recv(conn.id, 5);
  host.ws.close(conn.id);
  return reply;
}}

export async function startServer() {{
  return await host.ws.serve({{ host: "127.0.0.1", port: 0, path: "/ws" }});
}}

export function onWebSocketMessage(event) {{
  if (event.type === "text") {{
    host.ws.send(event.connectionId, "echo:" + event.text);
  }}
}}

export function stopServer(event) {{
  return host.ws.stop(event.id);
}}
"#
            );
            let dir = tempfile::tempdir().unwrap();
            let plugin_dir = dir.path().join("plugins").join("ws");
            std::fs::create_dir_all(&plugin_dir).unwrap();
            std::fs::write(plugin_dir.join("main.js"), code).unwrap();

            let host = PluginHost::new(dir.path(), tokio::runtime::Handle::current());
            host.apply_settings(PluginSettings {
                allow_network: true,
                allow_process: false,
                disabled: Vec::new(),
            });
            host.load_all_enabled().await;
            assert_eq!(host.views()[0].status, PluginStatus::Running);

            let reply = host
                .invoke("ws", "clientTest", serde_json::json!({}))
                .await
                .expect("clientTest")
                .expect("value");
            assert_eq!(reply.get("type").and_then(Value::as_str), Some("text"));
            assert_eq!(
                reply.get("text").and_then(Value::as_str),
                Some("echo:hello")
            );

            let server = host
                .invoke("ws", "startServer", serde_json::json!({}))
                .await
                .expect("startServer")
                .expect("value");
            let server_id = server.get("id").and_then(Value::as_u64).unwrap();
            let port = server.get("port").and_then(Value::as_u64).unwrap();
            assert_eq!(port > 0, true);

            let (mut client, _) =
                tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws"))
                    .await
                    .expect("connect to plugin server");
            client
                .send(tokio_tungstenite::tungstenite::Message::Text("hi".into()))
                .await
                .unwrap();
            let message = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
                .await
                .expect("server reply timeout")
                .expect("server reply")
                .expect("valid frame");
            match message {
                tokio_tungstenite::tungstenite::Message::Text(text) => {
                    assert_eq!(text.as_str(), "echo:hi")
                }
                other => panic!("unexpected frame: {other:?}"),
            }
            let _ = client.close(None).await;
            let stopped = host
                .invoke("ws", "stopServer", serde_json::json!({ "id": server_id }))
                .await
                .expect("stopServer")
                .expect("value");
            assert_eq!(stopped.as_bool(), Some(true));

            host.unload_all().await;
        });
    }

    #[test]
    fn on_list_models_refreshes_dynamic_providers() {
        let code = r#"
export const name = "Dynamic Models";
export const version = "1.0.0";
export const providers = [
  { name: "Dyn", protocol: "openai-chat", models: ["m1"] }
];
export function onListModels() {
  return {
    providers: [
      { name: "Dyn", protocol: "openai-chat", models: ["m2", "m3"], streaming: true }
    ]
  };
}
"#;
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("plugins").join("dyn");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("main.js"), code).unwrap();

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let host = PluginHost::new(dir.path(), tokio::runtime::Handle::current());
            host.load_all_enabled().await;
            let views = host.views();
            assert_eq!(views[0].providers[0].models, vec!["m1".to_string()]);
            assert!(host
                .dynamic_entries()
                .routes
                .iter()
                .any(|route| route.client_model == "m1"));

            let count = host.refresh_models("dyn").await.expect("refresh");
            assert_eq!(count, 2);
            let views = host.views();
            assert_eq!(
                views[0].providers[0].models,
                vec!["m2".to_string(), "m3".to_string()]
            );
            assert!(host
                .dynamic_entries()
                .routes
                .iter()
                .any(|route| route.client_model == "m2"));
            host.unload_all().await;
        });
    }

    #[test]
    fn uninstall_removes_files_and_optionally_data() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        let folder = plugins.join("demo");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("main.js"), "export const name = 'Demo';").unwrap();
        let data = dir.path().join("plugin-data").join("demo");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("state.json"), "{}").unwrap();

        let single = plugins.join("single.js");
        std::fs::write(&single, "export const name = 'Single';").unwrap();
        let single_data = dir.path().join("plugin-data").join("single");
        std::fs::create_dir_all(&single_data).unwrap();

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let host = PluginHost::new(dir.path(), tokio::runtime::Handle::current());
            let removed = host.uninstall("demo", true).await.expect("uninstall");
            assert_eq!(removed.len(), 1);
            assert!(!folder.exists());
            assert!(data.join("state.json").is_file());
            assert!(host.views().iter().all(|plugin| plugin.id != "demo"));

            let removed = host.uninstall("single", false).await.expect("uninstall");
            assert_eq!(removed.len(), 2);
            assert!(!single.exists());
            assert!(!single_data.exists());
        });
    }

    const PROVIDER_PLUGIN: &str = r#"
export const name = "Bridge";
export const version = "1.0.0";
export const providers = [
  { name: "Echo Bridge", protocol: "openai-chat", models: ["echo-plugin"], priority: 5 }
];

export function onProviderRequest(event) {
  if (event.request.stream) {
    host.upstream.emitRaw('data: {"choices":[{"delta":{"content":"plugin-stream"}}]}\n\n');
    host.upstream.emitRaw('data: [DONE]\n\n');
    return;
  }
  return {
    status: 200,
    body: {
      id: "chatcmpl-plugin",
      object: "chat.completion",
      model: event.request.model,
      choices: [{ index: 0, message: { role: "assistant", content: "plugin-ok" }, finish_reason: "stop" }],
      usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 }
    }
  };
}
"#;

    #[test]
    fn plugins_can_provide_virtual_providers() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("plugins").join("bridge");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("main.js"), PROVIDER_PLUGIN).unwrap();

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
            assert_eq!(views.len(), 1, "{:?}", views[0].last_error);
            assert_eq!(views[0].status, PluginStatus::Running);
            assert_eq!(views[0].providers.len(), 1);

            let entries = host.dynamic_entries();
            assert_eq!(entries.providers.len(), 1);
            assert_eq!(entries.accounts.len(), 1);
            assert_eq!(entries.routes.len(), 1);
            assert_eq!(entries.routes[0].client_model, "echo-plugin");
            assert_eq!(entries.routes[0].upstream_model, "echo-plugin");
            assert_eq!(entries.providers[0].base_url, "plugin://bridge");
            assert!(entries.streaming.contains(&entries.providers[0].id));

            let mut merged = crate::config::AppConfig::default();
            merged.providers.extend(entries.providers.clone());
            merged.accounts.extend(entries.accounts.clone());
            merged.routes.extend(entries.routes.clone());
            assert!(merged
                .servable_models()
                .iter()
                .any(|model| model == "echo-plugin"));

            let (status, body) = host
                .call_provider(
                    &views[0].id,
                    serde_json::json!({
                        "event": "onProviderRequest",
                        "request": {"model": "echo-plugin", "stream": false, "payload": {}}
                    }),
                    Duration::from_secs(10),
                )
                .await
                .expect("non-stream provider reply");
            assert_eq!(status, 200);
            assert_eq!(
                body.pointer("/choices/0/message/content")
                    .and_then(Value::as_str),
                Some("plugin-ok")
            );

            let mut chunks = host
                .call_provider_stream(
                    &views[0].id,
                    serde_json::json!({
                        "event": "onProviderRequest",
                        "request": {"model": "echo-plugin", "stream": true, "payload": {}}
                    }),
                )
                .await
                .expect("stream provider reply");
            let mut text = String::new();
            while let Some(item) = chunks.recv().await {
                let bytes = item.expect("stream chunk");
                text.push_str(&String::from_utf8_lossy(&bytes));
            }
            assert!(text.contains("plugin-stream"), "{text:?}");

            host.unload_all().await;
        });
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

            for index in 0..600 {
                host.log("test", "info", format!("line {index}"));
            }
            host.set_console_target(Some("test".into()));
            let console = host.console_view().expect("console view");
            assert_eq!(console.logs.len(), MAX_LOGS);
            assert_eq!(console.logs.first().unwrap().message, "line 100");
            assert_eq!(console.logs.last().unwrap().message, "line 599");
            assert_eq!(host.views()[0].log_count, MAX_LOGS);
            host.clear_logs("test");
            assert!(host.console_view().expect("console view").logs.is_empty());
            assert_eq!(host.views()[0].log_count, 0);

            host.unload("test").await;
            let views = host.views();
            assert_eq!(views[0].status, PluginStatus::Stopped);
        });
    }
}
