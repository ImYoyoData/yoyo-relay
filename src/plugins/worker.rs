//! Dedicated plugin thread: owns the Boa `Context`, loads the ES module and
//! executes event handlers and host API calls. Nothing in this module is
//! `Send`; only values crossing the channel boundary are.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Component, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use boa_engine::builtins::promise::PromiseState;
use boa_engine::job::SimpleJobExecutor;
use boa_engine::module::{Module, SimpleModuleLoader};
use boa_engine::object::builtins::JsPromise;
use boa_engine::object::ObjectInitializer;
use boa_engine::property::Attribute;
use boa_engine::{
    js_string, Context, JsError, JsNativeError, JsObject, JsResult, JsValue, NativeFunction, Source,
};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request as WsRequest, Response as WsResponse,
};
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{accept_hdr_async, connect_async};
use tokio_util::sync::CancellationToken;

use super::{PluginProviderSpec, PluginStatus, PluginUiKind, WorkerCommand, WorkerEnv, WorkerMeta};

/// Events the host understands; a module exporting any of these gets them.
const KNOWN_EVENTS: &[&str] = &[
    "onStartup",
    "onEnable",
    "onDisable",
    "onShutdown",
    "onReload",
    "beforeRequest",
    "afterResponse",
    "onRequestCompleted",
    "onAccountFailure",
    "onBalanceUpdated",
    "onProxyStart",
    "onProxyStop",
    "onConfigChanged",
    "onRouteSelected",
    "onError",
    "onProviderRequest",
    "onListModels",
    "onWebSocketConnection",
    "onWebSocketMessage",
    "onWebSocketClosed",
    "selfTest",
];

struct WorkerRuntime {
    env: WorkerEnv,
    name: RefCell<String>,
    /// Queue used by background services (WebSocket servers) to dispatch
    /// events back into the plugin's JS thread.
    events: mpsc::UnboundedSender<WorkerCommand>,
    ws: Arc<WsState>,
}

#[derive(Default)]
struct WsState {
    next_id: AtomicU64,
    clients: Mutex<HashMap<u64, WsClient>>,
    servers: Mutex<HashMap<u64, WsServer>>,
}

struct WsClient {
    tx: mpsc::UnboundedSender<Message>,
    rx: Option<mpsc::UnboundedReceiver<WsIncoming>>,
    reader: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
}

struct WsServer {
    task: JoinHandle<()>,
    cancel: CancellationToken,
    connections: Arc<Mutex<HashMap<u64, WsConnection>>>,
    host: String,
    port: u16,
    path: Option<String>,
}

struct WsConnection {
    tx: mpsc::UnboundedSender<Message>,
    reader: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
}

enum WsIncoming {
    Text(String),
    Binary(Vec<u8>),
    Closed(Option<u16>, String),
}

impl WsState {
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn sender_for(&self, id: u64) -> Option<mpsc::UnboundedSender<Message>> {
        if let Some(client) = self
            .clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
        {
            return Some(client.tx.clone());
        }
        let servers = self.servers.lock().unwrap_or_else(|e| e.into_inner());
        for server in servers.values() {
            if let Some(conn) = server
                .connections
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&id)
            {
                return Some(conn.tx.clone());
            }
        }
        None
    }

    fn close(&self, id: u64) -> bool {
        if let Some(client) = self
            .clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
        {
            close_client(client);
            return true;
        }
        let servers = self.servers.lock().unwrap_or_else(|e| e.into_inner());
        for server in servers.values() {
            if let Some(conn) = server
                .connections
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id)
            {
                close_connection(conn);
                return true;
            }
        }
        false
    }

    fn stop(&self, server_id: u64) -> bool {
        let server = self
            .servers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&server_id);
        let Some(server) = server else {
            return false;
        };
        server.cancel.cancel();
        server.task.abort();
        let connections: Vec<WsConnection> = {
            let mut guard = server.connections.lock().unwrap_or_else(|e| e.into_inner());
            guard.drain().map(|(_, conn)| conn).collect()
        };
        for conn in connections {
            close_connection(conn);
        }
        true
    }

    fn shutdown(&self) {
        let servers: Vec<WsServer> = {
            let mut guard = self.servers.lock().unwrap_or_else(|e| e.into_inner());
            guard.drain().map(|(_, server)| server).collect()
        };
        for server in servers {
            server.cancel.cancel();
            server.task.abort();
            let connections: Vec<WsConnection> = {
                let mut guard = server.connections.lock().unwrap_or_else(|e| e.into_inner());
                guard.drain().map(|(_, conn)| conn).collect()
            };
            for conn in connections {
                abort_connection(conn);
            }
        }
        let clients: Vec<WsClient> = {
            let mut guard = self.clients.lock().unwrap_or_else(|e| e.into_inner());
            guard.drain().map(|(_, client)| client).collect()
        };
        for client in clients {
            abort_client(client);
        }
    }
}

fn close_client(client: WsClient) {
    let WsClient {
        tx,
        rx,
        reader,
        writer,
    } = client;
    let _ = tx.send(Message::Close(None));
    drop(rx);
    drop(tx);
    let _ = tokio::spawn(async move {
        if let Some(writer) = writer {
            let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
        }
        if let Some(reader) = reader {
            reader.abort();
        }
    });
}

fn close_connection(conn: WsConnection) {
    let WsConnection { tx, reader, writer } = conn;
    let _ = tx.send(Message::Close(None));
    drop(tx);
    let _ = tokio::spawn(async move {
        if let Some(writer) = writer {
            let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
        }
        if let Some(reader) = reader {
            reader.abort();
        }
    });
}

fn abort_client(client: WsClient) {
    let WsClient {
        tx,
        rx,
        reader,
        writer,
    } = client;
    let _ = tx.send(Message::Close(None));
    drop(rx);
    drop(tx);
    if let Some(reader) = reader {
        reader.abort();
    }
    if let Some(writer) = writer {
        writer.abort();
    }
}

fn abort_connection(conn: WsConnection) {
    let WsConnection { tx, reader, writer } = conn;
    let _ = tx.send(Message::Close(None));
    drop(tx);
    if let Some(reader) = reader {
        reader.abort();
    }
    if let Some(writer) = writer {
        writer.abort();
    }
}

struct PathGuard {
    path: Option<String>,
}

impl Callback for PathGuard {
    fn on_request(
        self,
        request: &WsRequest,
        response: WsResponse,
    ) -> Result<WsResponse, ErrorResponse> {
        if let Some(expected) = self.path.as_deref() {
            if request.uri().path() != expected {
                return Err(ErrorResponse::new(Some("WebSocket 路径不匹配".into())));
            }
        }
        Ok(response)
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<WorkerRuntime>>> = const { RefCell::new(None) };
    /// Active virtual-provider emitter. Only set while `onProviderRequest` runs.
    static UPSTREAM: RefCell<Option<tokio::sync::mpsc::UnboundedSender<Result<Bytes, String>>>> =
        const { RefCell::new(None) };
}

fn current() -> JsResult<Arc<WorkerRuntime>> {
    CURRENT.with(|slot| {
        slot.borrow()
            .clone()
            .ok_or_else(|| js_error("插件运行时未初始化"))
    })
}

fn js_error(message: impl Into<String>) -> JsError {
    JsNativeError::typ().with_message(message.into()).into()
}

fn set_current(worker: Arc<WorkerRuntime>) {
    CURRENT.with(|slot| *slot.borrow_mut() = Some(worker));
}

fn clear_current() {
    CURRENT.with(|slot| *slot.borrow_mut() = None);
}

pub(crate) fn run(
    env: WorkerEnv,
    mut rx: mpsc::UnboundedReceiver<WorkerCommand>,
    init_tx: oneshot::Sender<Result<WorkerMeta, String>>,
    events: mpsc::UnboundedSender<WorkerCommand>,
) {
    let _enter = env.handle.enter();
    let worker = Arc::new(WorkerRuntime {
        name: RefCell::new(env.id.clone()),
        env,
        events,
        ws: Arc::new(WsState::default()),
    });
    set_current(worker.clone());

    let mut session = match Session::open(worker.clone()) {
        Ok(session) => session,
        Err(err) => {
            worker
                .env
                .shared
                .log(&worker.env.id, "error", format!("加载失败：{err}"));
            let mut infos = worker
                .env
                .shared
                .infos
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(info) = infos.get_mut(&worker.env.id) {
                info.status = PluginStatus::Error;
                info.last_error = Some(err.clone());
            }
            drop(infos);
            let _ = init_tx.send(Err(err));
            clear_current();
            return;
        }
    };

    let meta = session.meta.clone();
    let _ = init_tx.send(Ok(meta));

    let mut shutdown_reply = None;
    while let Some(command) = rx.blocking_recv() {
        match command {
            WorkerCommand::Event {
                name,
                payload,
                reply,
            } => {
                let result = session.invoke(&name, payload);
                if reply.is_none() {
                    if let Err(err) = &result {
                        session.worker.env.shared.log(
                            &session.worker.env.id,
                            "error",
                            format!("{name}: {err}"),
                        );
                    }
                }
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            }
            WorkerCommand::ProviderRequest {
                payload,
                stream,
                chunks,
                start,
                reply,
            } => {
                if stream {
                    let exists = session.has_handler("onProviderRequest");
                    if let Some(start) = start {
                        let _ = start.send(exists);
                    }
                    if exists {
                        let sender = chunks;
                        set_upstream(sender.clone());
                        let result = session.invoke("onProviderRequest", payload);
                        clear_upstream();
                        if let Err(err) = result {
                            if let Some(tx) = sender {
                                let _ = tx.send(Err(err));
                            }
                        }
                    }
                    if let Some(reply) = reply {
                        let _ = reply.send(Ok(None));
                    }
                } else {
                    let result = session.invoke("onProviderRequest", payload);
                    if let Some(reply) = reply {
                        let _ = reply.send(result);
                    }
                }
            }
            WorkerCommand::Shutdown { reply } => {
                let _ = session.invoke("onShutdown", json!({ "event": "onShutdown" }));
                session.worker.ws.shutdown();
                shutdown_reply = Some(reply);
                break;
            }
        }
    }

    // Dropping the Context is the unload. Boa's GC reclaims every module,
    // global and object that belonged to it.
    session.close();
    if let Some(reply) = shutdown_reply {
        let _ = reply.send(());
    }
    clear_current();
}

struct Session {
    ctx: Context,
    module: Module,
    namespace: JsObject,
    worker: Arc<WorkerRuntime>,
    meta: WorkerMeta,
}

impl Session {
    fn open(worker: Arc<WorkerRuntime>) -> Result<Self, String> {
        let env = &worker.env;
        let source = std::fs::read_to_string(&env.entry)
            .map_err(|err| format!("读取 {} 失败：{err}", env.entry.display()))?;
        let loader = Rc::new(
            SimpleModuleLoader::new(&env.dir)
                .map_err(|err| format!("初始化模块加载器失败：{err}"))?,
        );
        let mut ctx = Context::builder()
            .job_executor(Rc::new(SimpleJobExecutor::new()))
            .module_loader(loader)
            .build()
            .map_err(|err| format!("创建 JS Context 失败：{err}"))?;
        ctx.runtime_limits_mut()
            .set_loop_iteration_limit(20_000_000);
        ctx.runtime_limits_mut().set_recursion_limit(2_000);

        install_globals(&mut ctx, worker.clone())
            .map_err(|err| format!("注入宿主 API 失败：{err}"))?;

        let module = Module::parse(
            Source::from_bytes(&source).with_path(&env.entry),
            None,
            &mut ctx,
        )
        .map_err(|err| format!("解析插件模块失败：{err}"))?;

        let started = Instant::now();
        let load = module.load(&mut ctx);
        drive(&mut ctx, &load, started, Duration::from_secs(15))
            .map_err(|err| format!("加载模块依赖失败：{err}"))?;
        module
            .link(&mut ctx)
            .map_err(|err| format!("链接模块失败：{err}"))?;
        let eval = module
            .evaluate(&mut ctx)
            .map_err(|err| format!("执行模块失败：{err}"))?;
        drive(&mut ctx, &eval, started, Duration::from_secs(15))
            .map_err(|err| format!("模块求值失败：{err}"))?;
        ctx.run_jobs()
            .map_err(|err| format!("运行异步任务失败：{err}"))?;

        let namespace = module.namespace(&mut ctx);
        let name = export_string(&namespace, "name", &mut ctx)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| env.id.clone());
        *worker.name.borrow_mut() = name.clone();
        let version = export_string(&namespace, "version", &mut ctx)
            .filter(|version| !version.trim().is_empty())
            .unwrap_or_else(|| "1.0.0".to_string());
        let description = export_string(&namespace, "description", &mut ctx).unwrap_or_default();

        let providers = export_providers(&namespace, &mut ctx);
        let mut events: Vec<String> = export_string_array(&namespace, "events", &mut ctx);
        for event in KNOWN_EVENTS {
            if namespace
                .get(js_string!(*event), &mut ctx)
                .map(|value| value.is_callable())
                .unwrap_or(false)
                && !events.iter().any(|known| known == event)
            {
                events.push((*event).to_string());
            }
        }
        events.sort();
        events.dedup();

        let mut session = Self {
            ctx,
            module,
            namespace,
            worker,
            meta: WorkerMeta {
                name,
                version,
                description,
                events,
                providers,
            },
        };
        let _ = session.invoke(
            "onStartup",
            json!({
                "event": "onStartup",
                "plugin": {
                    "id": session.worker.env.id,
                    "name": session.meta.name,
                    "version": session.meta.version,
                    "dir": session.worker.env.dir,
                }
            }),
        );
        Ok(session)
    }

    fn has_handler(&mut self, name: &str) -> bool {
        self.namespace
            .get(js_string!(name), &mut self.ctx)
            .map(|value| value.is_callable())
            .unwrap_or(false)
    }

    fn invoke(&mut self, name: &str, payload: Value) -> Result<Option<Value>, String> {
        let value = self
            .namespace
            .get(js_string!(name), &mut self.ctx)
            .map_err(|err| format!("读取 {name} 失败：{err}"))?;
        let Some(function) = value.as_callable() else {
            return Ok(None);
        };
        self.worker.env.shared.count_call(&self.worker.env.id);
        let argument = JsValue::from_json(&payload, &mut self.ctx)
            .map_err(|err| format!("构造事件参数失败：{err}"))?;
        let started = Instant::now();
        let result = function
            .call(&JsValue::undefined(), &[argument], &mut self.ctx)
            .map_err(|err| format!("{name} 执行失败：{err}"))?;
        let result = if let Some(object) = result.as_object() {
            if let Ok(promise) = JsPromise::from_object(object.clone()) {
                drive(&mut self.ctx, &promise, started, Duration::from_secs(20))?
            } else {
                result
            }
        } else {
            result
        };
        self.ctx
            .run_jobs()
            .map_err(|err| format!("运行异步任务失败：{err}"))?;
        if result.is_null_or_undefined() {
            return Ok(None);
        }
        let json = result
            .to_json(&mut self.ctx)
            .map_err(|err| format!("转换返回值失败：{err}"))?;
        Ok(json)
    }

    fn close(self) {
        // Explicit drops make the teardown order obvious: module objects go
        // first, then the Context and everything the GC owns.
        drop(self.namespace);
        drop(self.module);
        drop(self.ctx);
    }
}

fn drive(
    ctx: &mut Context,
    promise: &JsPromise,
    started: Instant,
    timeout: Duration,
) -> Result<JsValue, String> {
    loop {
        ctx.run_jobs()
            .map_err(|err| format!("异步任务失败：{err}"))?;
        match promise.state() {
            PromiseState::Fulfilled(value) => return Ok(value),
            PromiseState::Rejected(error) => {
                let text = error
                    .to_string(ctx)
                    .map(|text| text.to_std_string_escaped())
                    .unwrap_or_else(|_| "Promise rejected".into());
                return Err(text);
            }
            PromiseState::Pending => {
                if started.elapsed() > timeout {
                    return Err("插件执行超时".into());
                }
                std::thread::sleep(Duration::from_millis(3));
            }
        }
    }
}

fn export_string(namespace: &JsObject, name: &str, ctx: &mut Context) -> Option<String> {
    namespace
        .get(js_string!(name), ctx)
        .ok()
        .and_then(|value| value.as_string())
        .map(|value| value.to_std_string_escaped())
}

fn export_providers(namespace: &JsObject, ctx: &mut Context) -> Vec<PluginProviderSpec> {
    let spec = namespace
        .get(js_string!("providers"), ctx)
        .ok()
        .and_then(|value| value.to_json(ctx).ok().flatten())
        .and_then(|value| serde_json::from_value::<Vec<PluginProviderSpec>>(value).ok())
        .unwrap_or_default();
    normalize_provider_specs(spec)
}

/// Clamp and normalise provider specs from the module's `providers` export or
/// from an `onListModels` result.
pub(crate) fn normalize_provider_specs(spec: Vec<PluginProviderSpec>) -> Vec<PluginProviderSpec> {
    let mut out = Vec::new();
    for mut provider in spec.into_iter().take(8) {
        provider.models = provider
            .models
            .into_iter()
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty())
            .take(16)
            .collect();
        provider.models.sort();
        provider.models.dedup();
        if provider.models.is_empty() {
            continue;
        }
        let protocol = provider.protocol.trim().to_ascii_lowercase();
        provider.protocol = match protocol.as_str() {
            "anthropic" | "anthropic-messages" | "anthropic_messages" => {
                "anthropic-messages".to_string()
            }
            "responses" | "openai-responses" | "openai_responses" => "openai-responses".to_string(),
            _ => "openai-chat".to_string(),
        };
        if provider.name.trim().is_empty() {
            provider.name = "插件供应商".to_string();
        }
        out.push(provider);
    }
    out
}

fn export_string_array(namespace: &JsObject, name: &str, ctx: &mut Context) -> Vec<String> {
    namespace
        .get(js_string!(name), ctx)
        .ok()
        .and_then(|value| value.to_json(ctx).ok().flatten())
        .and_then(|value| value.as_array().cloned())
        .map(|items| {
            items
                .into_iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Host API
// ---------------------------------------------------------------------------

fn install_globals(ctx: &mut Context, worker: Arc<WorkerRuntime>) -> JsResult<()> {
    let env = &worker.env;

    let mut fs = ObjectInitializer::new(ctx);
    fs.function(
        NativeFunction::from_fn_ptr(fs_read_text),
        js_string!("readText"),
        1,
    )
    .function(
        NativeFunction::from_fn_ptr(fs_write_text),
        js_string!("writeText"),
        2,
    )
    .function(
        NativeFunction::from_fn_ptr(fs_append_text),
        js_string!("appendText"),
        2,
    )
    .function(
        NativeFunction::from_fn_ptr(fs_exists),
        js_string!("exists"),
        1,
    )
    .function(NativeFunction::from_fn_ptr(fs_list), js_string!("list"), 1)
    .function(
        NativeFunction::from_fn_ptr(fs_remove),
        js_string!("remove"),
        1,
    )
    .function(
        NativeFunction::from_fn_ptr(fs_mkdir),
        js_string!("mkdir"),
        1,
    )
    .function(
        NativeFunction::from_fn_ptr(fs_read_base64),
        js_string!("readBase64"),
        1,
    )
    .function(
        NativeFunction::from_fn_ptr(fs_write_base64),
        js_string!("writeBase64"),
        2,
    );
    let fs_object = fs.build();

    let mut encoding = ObjectInitializer::new(ctx);
    encoding
        .function(
            NativeFunction::from_fn_ptr(enc_base64_encode),
            js_string!("base64Encode"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(enc_base64_decode),
            js_string!("base64Decode"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(enc_base64url_encode),
            js_string!("base64UrlEncode"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(enc_base64url_decode),
            js_string!("base64UrlDecode"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(enc_url_encode),
            js_string!("urlEncode"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(enc_url_decode),
            js_string!("urlDecode"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(enc_hex_encode),
            js_string!("hexEncode"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(enc_hex_decode),
            js_string!("hexDecode"),
            1,
        );
    let encoding_object = encoding.build();

    let mut crypto = ObjectInitializer::new(ctx);
    crypto
        .function(
            NativeFunction::from_fn_ptr(crypto_sha256),
            js_string!("sha256"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(crypto_hmac_sha256),
            js_string!("hmacSha256"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(crypto_random_bytes),
            js_string!("randomBytes"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(crypto_aes_gcm_encrypt),
            js_string!("aesGcmEncrypt"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(crypto_aes_gcm_decrypt),
            js_string!("aesGcmDecrypt"),
            3,
        )
        .function(
            NativeFunction::from_fn_ptr(crypto_constant_time_equal),
            js_string!("constantTimeEqual"),
            2,
        );
    let crypto_object = crypto.build();

    let mut http = ObjectInitializer::new(ctx);
    http.function(
        NativeFunction::from_async_fn(http_request),
        js_string!("request"),
        1,
    )
    .function(
        NativeFunction::from_async_fn(http_download),
        js_string!("download"),
        2,
    )
    .function(
        NativeFunction::from_async_fn(http_sse),
        js_string!("sse"),
        2,
    );
    let http_object = http.build();

    let mut ws = ObjectInitializer::new(ctx);
    ws.function(
        NativeFunction::from_async_fn(ws_connect),
        js_string!("connect"),
        1,
    )
    .function(
        NativeFunction::from_async_fn(ws_serve),
        js_string!("serve"),
        1,
    )
    .function(
        NativeFunction::from_async_fn(ws_recv),
        js_string!("recv"),
        2,
    )
    .function(NativeFunction::from_fn_ptr(ws_send), js_string!("send"), 2)
    .function(
        NativeFunction::from_fn_ptr(ws_send_binary),
        js_string!("sendBinary"),
        2,
    )
    .function(
        NativeFunction::from_fn_ptr(ws_close),
        js_string!("close"),
        1,
    )
    .function(NativeFunction::from_fn_ptr(ws_stop), js_string!("stop"), 1)
    .function(NativeFunction::from_fn_ptr(ws_list), js_string!("list"), 0);
    let ws_object = ws.build();

    let mut net = ObjectInitializer::new(ctx);
    net.function(
        NativeFunction::from_async_fn(http_sse),
        js_string!("sse"),
        2,
    )
    .property(js_string!("ws"), ws_object.clone(), Attribute::all());
    let net_object = net.build();

    let mut process = ObjectInitializer::new(ctx);
    process.function(
        NativeFunction::from_async_fn(process_run),
        js_string!("run"),
        1,
    );
    let process_object = process.build();

    let mut ui = ObjectInitializer::new(ctx);
    ui.function(
        NativeFunction::from_fn_ptr(ui_toast),
        js_string!("toast"),
        2,
    )
    .function(
        NativeFunction::from_async_fn(ui_confirm),
        js_string!("confirm"),
        1,
    )
    .function(
        NativeFunction::from_async_fn(ui_prompt),
        js_string!("prompt"),
        2,
    )
    .function(
        NativeFunction::from_async_fn(ui_alert),
        js_string!("alert"),
        2,
    );
    let ui_object = ui.build();

    let models_value = JsValue::from_json(&json!(env.models), ctx)?;
    let app_object = {
        let mut app = ObjectInitializer::new(ctx);
        app.property(
            js_string!("version"),
            JsValue::from(js_string!(env!("CARGO_PKG_VERSION"))),
            Attribute::all(),
        )
        .property(
            js_string!("endpoint"),
            JsValue::from(js_string!(env.endpoint.as_str())),
            Attribute::all(),
        )
        .property(js_string!("models"), models_value, Attribute::all());
        app.build()
    };

    let upstream_object = {
        let mut upstream = ObjectInitializer::new(ctx);
        upstream
            .function(
                NativeFunction::from_fn_ptr(upstream_emit_raw),
                js_string!("emitRaw"),
                1,
            )
            .function(
                NativeFunction::from_fn_ptr(upstream_emit_json),
                js_string!("emitJson"),
                1,
            )
            .function(
                NativeFunction::from_fn_ptr(upstream_error),
                js_string!("error"),
                1,
            )
            .function(
                NativeFunction::from_fn_ptr(upstream_finish),
                js_string!("finish"),
                0,
            );
        upstream.build()
    };

    let storage_object = {
        let mut storage = ObjectInitializer::new(ctx);
        storage
            .function(
                NativeFunction::from_fn_ptr(storage_get),
                js_string!("get"),
                1,
            )
            .function(
                NativeFunction::from_fn_ptr(storage_set),
                js_string!("set"),
                2,
            )
            .function(
                NativeFunction::from_fn_ptr(storage_remove),
                js_string!("remove"),
                1,
            )
            .function(
                NativeFunction::from_fn_ptr(storage_keys),
                js_string!("keys"),
                0,
            );
        storage.build()
    };

    let time_object = {
        let mut time = ObjectInitializer::new(ctx);
        time.function(NativeFunction::from_fn_ptr(time_now), js_string!("now"), 0)
            .function(
                NativeFunction::from_fn_ptr(time_now_iso),
                js_string!("nowIso"),
                0,
            );
        time.build()
    };

    let util_object = {
        let mut util = ObjectInitializer::new(ctx);
        util.function(
            NativeFunction::from_fn_ptr(util_uuid),
            js_string!("uuid"),
            0,
        );
        util.build()
    };

    let host_object = {
        let mut host = ObjectInitializer::new(ctx);
        host.function(NativeFunction::from_fn_ptr(host_log), js_string!("log"), 2)
            .function(
                NativeFunction::from_async_fn(host_sleep),
                js_string!("sleep"),
                1,
            )
            .property(js_string!("fs"), fs_object, Attribute::all())
            .property(js_string!("encoding"), encoding_object, Attribute::all())
            .property(js_string!("crypto"), crypto_object, Attribute::all())
            .property(js_string!("http"), http_object, Attribute::all())
            .property(js_string!("ws"), ws_object, Attribute::all())
            .property(js_string!("net"), net_object, Attribute::all())
            .property(js_string!("process"), process_object, Attribute::all())
            .property(js_string!("ui"), ui_object, Attribute::all())
            .property(js_string!("app"), app_object, Attribute::all())
            .property(js_string!("storage"), storage_object, Attribute::all())
            .property(js_string!("upstream"), upstream_object, Attribute::all())
            .property(js_string!("time"), time_object, Attribute::all())
            .property(js_string!("util"), util_object, Attribute::all());
        host.build()
    };
    ctx.register_global_property(js_string!("host"), host_object, Attribute::all())
}

fn arg_string(args: &[JsValue], index: usize) -> String {
    args.get(index)
        .and_then(|value| value.as_string())
        .map(|value| value.to_std_string_escaped())
        .unwrap_or_default()
}

fn arg_json(args: &[JsValue], index: usize, ctx: &mut Context) -> Value {
    args.get(index)
        .map(|value| value.to_json(ctx))
        .transpose()
        .ok()
        .flatten()
        .flatten()
        .unwrap_or_else(|| json!({}))
}

fn arg_number(args: &[JsValue], index: usize) -> f64 {
    args.get(index)
        .and_then(|value| value.as_number())
        .unwrap_or(0.0)
}

fn json_value(value: &Value, ctx: &mut Context) -> JsResult<JsValue> {
    JsValue::from_json(value, ctx)
}

// -- logging / app ----------------------------------------------------------

fn host_log(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    let level = arg_string(args, 0);
    let message = arg_string(args, 1);
    let level = match level.to_ascii_lowercase().as_str() {
        "error" => "error",
        "warn" | "warning" => "warn",
        "debug" => "debug",
        _ => "info",
    };
    worker.env.shared.log(&worker.env.id, level, message);
    Ok(JsValue::undefined())
}

async fn host_sleep(
    _this: &JsValue,
    args: &[JsValue],
    _ctx: &RefCell<&mut Context>,
) -> JsResult<JsValue> {
    let millis = arg_number(args, 0).clamp(0.0, 60_000.0) as u64;
    tokio::time::sleep(Duration::from_millis(millis)).await;
    Ok(JsValue::undefined())
}

// -- filesystem -------------------------------------------------------------

fn data_root(worker: &WorkerRuntime) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&worker.env.data_dir)
        .map_err(|err| format!("创建插件数据目录失败：{err}"))?;
    Ok(worker.env.data_dir.clone())
}

fn safe_path(worker: &WorkerRuntime, relative: &str) -> Result<PathBuf, String> {
    let root = data_root(worker)?;
    let mut path = PathBuf::from(relative);
    if path.is_absolute() {
        return Err("只允许插件数据目录内的相对路径".into());
    }
    let mut resolved = root.clone();
    for component in path.components() {
        match component {
            Component::Normal(part) => resolved.push(part),
            Component::CurDir => {}
            _ => return Err("路径不能包含 .. 或根目录".into()),
        }
    }
    path.clear();
    Ok(resolved)
}

fn fs_read_text(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    let path = safe_path(&worker, &arg_string(args, 0)).map_err(js_error)?;
    let text = std::fs::read_to_string(&path)
        .map_err(|err| js_error(format!("读取 {} 失败：{err}", path.display())))?;
    Ok(JsValue::from(js_string!(text.as_str())))
}

fn fs_write_text(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    let path = safe_path(&worker, &arg_string(args, 0)).map_err(js_error)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| js_error(format!("创建目录失败：{err}")))?;
    }
    std::fs::write(&path, arg_string(args, 1))
        .map_err(|err| js_error(format!("写入 {} 失败：{err}", path.display())))?;
    Ok(JsValue::undefined())
}

fn fs_append_text(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    use std::io::Write;
    let worker = current()?;
    let path = safe_path(&worker, &arg_string(args, 0)).map_err(js_error)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| js_error(format!("创建目录失败：{err}")))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| js_error(format!("打开 {} 失败：{err}", path.display())))?;
    file.write_all(arg_string(args, 1).as_bytes())
        .map_err(|err| js_error(format!("追加写入失败：{err}")))?;
    Ok(JsValue::undefined())
}

fn fs_exists(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    let path = safe_path(&worker, &arg_string(args, 0)).map_err(js_error)?;
    Ok(JsValue::new(path.exists()))
}

fn fs_list(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    let path = safe_path(&worker, &arg_string(args, 0)).map_err(js_error)?;
    let mut names = Vec::new();
    if path.exists() {
        for entry in std::fs::read_dir(&path)
            .map_err(|err| js_error(format!("读取目录失败：{err}")))?
            .flatten()
        {
            names.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    names.sort();
    JsValue::from_json(&json!(names), _ctx)
}

fn fs_remove(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    let path = safe_path(&worker, &arg_string(args, 0)).map_err(js_error)?;
    if path.is_dir() {
        std::fs::remove_dir_all(&path).map_err(|err| js_error(format!("删除目录失败：{err}")))?;
    } else if path.exists() {
        std::fs::remove_file(&path).map_err(|err| js_error(format!("删除文件失败：{err}")))?;
    }
    Ok(JsValue::undefined())
}

fn fs_mkdir(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    let path = safe_path(&worker, &arg_string(args, 0)).map_err(js_error)?;
    std::fs::create_dir_all(&path).map_err(|err| js_error(format!("创建目录失败：{err}")))?;
    Ok(JsValue::undefined())
}

// -- encoding ---------------------------------------------------------------

fn enc_base64_encode(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let text = arg_string(args, 0);
    Ok(JsValue::from(js_string!(STANDARD
        .encode(text.as_bytes())
        .as_str())))
}

fn enc_base64_decode(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let text = arg_string(args, 0);
    let bytes = STANDARD
        .decode(text.trim())
        .map_err(|err| js_error(format!("Base64 解码失败：{err}")))?;
    Ok(JsValue::from(js_string!(String::from_utf8_lossy(&bytes)
        .to_string()
        .as_str())))
}

fn enc_base64url_encode(
    _this: &JsValue,
    args: &[JsValue],
    _ctx: &mut Context,
) -> JsResult<JsValue> {
    let text = arg_string(args, 0);
    Ok(JsValue::from(js_string!(URL_SAFE_NO_PAD
        .encode(text.as_bytes())
        .as_str())))
}

fn enc_base64url_decode(
    _this: &JsValue,
    args: &[JsValue],
    _ctx: &mut Context,
) -> JsResult<JsValue> {
    let text = arg_string(args, 0);
    let bytes = URL_SAFE_NO_PAD
        .decode(text.trim())
        .map_err(|err| js_error(format!("Base64URL 解码失败：{err}")))?;
    Ok(JsValue::from(js_string!(String::from_utf8_lossy(&bytes)
        .to_string()
        .as_str())))
}

fn enc_url_encode(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let text = arg_string(args, 0);
    Ok(JsValue::from(js_string!(urlencoding::encode(&text)
        .to_string()
        .as_str())))
}

fn enc_url_decode(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let text = arg_string(args, 0);
    let decoded = urlencoding::decode(&text)
        .map_err(|err| js_error(format!("URL 解码失败：{err}")))?
        .to_string();
    Ok(JsValue::from(js_string!(decoded.as_str())))
}

fn enc_hex_encode(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let text = arg_string(args, 0);
    Ok(JsValue::from(js_string!(
        hex_encode(text.as_bytes()).as_str()
    )))
}

fn enc_hex_decode(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let text = arg_string(args, 0);
    let bytes = hex_decode(&text).map_err(js_error)?;
    Ok(JsValue::from(js_string!(String::from_utf8_lossy(&bytes)
        .to_string()
        .as_str())))
}

// -- crypto -----------------------------------------------------------------

fn crypto_sha256(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let text = arg_string(args, 0);
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    Ok(JsValue::from(js_string!(
        hex_encode(&hasher.finalize()).as_str()
    )))
}

fn crypto_hmac_sha256(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    use hmac::{Hmac, Mac};
    let secret = arg_string(args, 0);
    let message = arg_string(args, 1);
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|err| js_error(format!("HMAC 初始化失败：{err}")))?;
    mac.update(message.as_bytes());
    Ok(JsValue::from(js_string!(hex_encode(
        &mac.finalize().into_bytes()
    )
    .as_str())))
}

fn crypto_random_bytes(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    use rand::RngCore;
    let count = arg_number(args, 0).clamp(1.0, 4096.0) as usize;
    let mut bytes = vec![0u8; count];
    rand::thread_rng().fill_bytes(&mut bytes);
    Ok(JsValue::from(js_string!(hex_encode(&bytes).as_str())))
}

fn crypto_aes_gcm_encrypt(
    _this: &JsValue,
    args: &[JsValue],
    _ctx: &mut Context,
) -> JsResult<JsValue> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    use rand::RngCore;
    let key = hex_decode(&arg_string(args, 0)).map_err(js_error)?;
    if key.len() != 32 {
        return Err(js_error("AES-256 密钥必须是 64 位十六进制"));
    }
    let plaintext = arg_string(args, 1);
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|err| js_error(format!("密钥无效：{err}")))?;
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes())
        .map_err(|err| js_error(format!("加密失败：{err}")))?;
    Ok(JsValue::from_json(
        &json!({
            "nonce": hex_encode(&nonce),
            "ciphertext": STANDARD.encode(ciphertext),
        }),
        _ctx,
    )?)
}

fn crypto_aes_gcm_decrypt(
    _this: &JsValue,
    args: &[JsValue],
    _ctx: &mut Context,
) -> JsResult<JsValue> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    let key = hex_decode(&arg_string(args, 0)).map_err(js_error)?;
    let nonce = hex_decode(&arg_string(args, 1)).map_err(js_error)?;
    let ciphertext = STANDARD
        .decode(arg_string(args, 2).trim())
        .map_err(|err| js_error(format!("密文不是有效 Base64：{err}")))?;
    if key.len() != 32 || nonce.len() != 12 {
        return Err(js_error("密钥需 32 字节，nonce 需 12 字节"));
    }
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|err| js_error(format!("密钥无效：{err}")))?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|err| js_error(format!("解密失败：{err}")))?;
    Ok(JsValue::from(js_string!(String::from_utf8_lossy(
        &plaintext
    )
    .to_string()
    .as_str())))
}

// -- virtual provider stream ------------------------------------------------

fn set_upstream(chunks: Option<tokio::sync::mpsc::UnboundedSender<Result<Bytes, String>>>) {
    UPSTREAM.with(|slot| *slot.borrow_mut() = chunks);
}

fn clear_upstream() {
    UPSTREAM.with(|slot| *slot.borrow_mut() = None);
}

fn upstream_sender() -> Option<tokio::sync::mpsc::UnboundedSender<Result<Bytes, String>>> {
    UPSTREAM.with(|slot| slot.borrow().clone())
}

fn upstream_emit_raw(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let text = arg_string(args, 0);
    let Some(tx) = upstream_sender() else {
        return Err(js_error("host.upstream 只能在 onProviderRequest 中使用"));
    };
    tx.send(Ok(Bytes::from(text)))
        .map_err(|_| js_error("上游流已关闭"))?;
    Ok(JsValue::undefined())
}

fn upstream_emit_json(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let Some(tx) = upstream_sender() else {
        return Err(js_error("host.upstream 只能在 onProviderRequest 中使用"));
    };
    let value = args
        .get(0)
        .map(|value| value.to_json(ctx))
        .transpose()?
        .flatten()
        .unwrap_or(Value::Null);
    let text =
        serde_json::to_string(&value).map_err(|err| js_error(format!("序列化失败：{err}")))?;
    tx.send(Ok(Bytes::from(format!("data: {text}\n\n"))))
        .map_err(|_| js_error("上游流已关闭"))?;
    Ok(JsValue::undefined())
}

fn upstream_error(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let message = arg_string(args, 0);
    if let Some(tx) = upstream_sender() {
        let _ = tx.send(Err(message));
    }
    Ok(JsValue::undefined())
}

fn upstream_finish(_this: &JsValue, _args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

/// Fetch an SSE endpoint and invoke `callback({ event, data, raw })` for each
/// event. The callback can forward frames with `host.upstream.emitRaw`.
async fn http_sse(
    _this: &JsValue,
    args: &[JsValue],
    context: &RefCell<&mut Context>,
) -> JsResult<JsValue> {
    let worker = current()?;
    if !worker.env.settings.allow_network {
        return Err(js_error("插件网络访问已在设置中关闭"));
    }
    let options = {
        let mut ctx = context.borrow_mut();
        arg_json(args, 0, &mut ctx)
    };
    let callback = args.get(1).cloned().unwrap_or_else(JsValue::undefined);
    if !callback.is_callable() {
        return Err(js_error("host.http.sse / host.net.sse 需要第二个回调参数"));
    }
    let url = options
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| url.starts_with("http://") || url.starts_with("https://"))
        .ok_or_else(|| js_error("host.http.sse / host.net.sse 需要 http(s) url"))?
        .to_string();
    let method = options
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let method = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|err| js_error(format!("HTTP 方法无效：{err}")))?;
    let client = if crate::gateway::is_loopback_url(&url) {
        &worker.env.http_local
    } else {
        &worker.env.http
    };
    let mut request = client
        .request(method, &url)
        .header(reqwest::header::ACCEPT, "text/event-stream");
    if let Some(headers) = options.get("headers").and_then(Value::as_object) {
        for (name, value) in headers {
            if let Some(value) = value.as_str() {
                request = request.header(name.as_str(), value);
            }
        }
    }
    if let Some(body) = options.get("body") {
        request = if let Some(text) = body.as_str() {
            request.body(text.to_string())
        } else {
            request.json(body)
        };
    }
    let timeout = options
        .get("timeoutSecs")
        .and_then(Value::as_u64)
        .unwrap_or(120)
        .clamp(1, 600);
    let response = tokio::time::timeout(Duration::from_secs(timeout), request.send())
        .await
        .map_err(|_| js_error(format!("SSE 连接超时（{timeout}s）")))?
        .map_err(|err| js_error(format!("SSE 连接失败：{err}")))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(js_error(format!("SSE 返回 HTTP {status}")));
    }
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let mut decoder = crate::protocol::SseDecoder::default();
    let mut events = 0u64;
    let mut stream = response.bytes_stream();
    'outer: loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(js_error("SSE 读取超时"));
        }
        match tokio::time::timeout(remaining, stream.next()).await {
            Err(_) => return Err(js_error("SSE 读取超时")),
            Ok(None) => break,
            Ok(Some(Err(err))) => return Err(js_error(format!("SSE 读取失败：{err}"))),
            Ok(Some(Ok(chunk))) => {
                for event in decoder.push(&chunk) {
                    emit_sse_callback(&callback, &event, context)?;
                    events += 1;
                    if Instant::now() >= deadline {
                        break 'outer;
                    }
                }
            }
        }
    }
    if let Some(event) = decoder.flush() {
        emit_sse_callback(&callback, &event, context)?;
        events += 1;
    }
    let mut ctx = context.borrow_mut();
    json_value(&json!({ "status": status, "events": events }), &mut ctx)
}

fn emit_sse_callback(
    callback: &JsValue,
    event: &crate::protocol::SseEvent,
    context: &RefCell<&mut Context>,
) -> JsResult<()> {
    let mut raw = String::new();
    if let Some(name) = &event.event {
        raw.push_str("event: ");
        raw.push_str(name);
        raw.push('\n');
    }
    for line in event.data.split('\n') {
        raw.push_str("data: ");
        raw.push_str(line);
        raw.push('\n');
    }
    raw.push('\n');
    let payload = json!({
        "event": event.event,
        "data": event.data,
        "raw": raw,
    });
    let mut ctx = context.borrow_mut();
    let value = JsValue::from_json(&payload, &mut ctx)?;
    if let Some(function) = callback.as_callable() {
        function.call(&JsValue::undefined(), &[value], &mut ctx)?;
    }
    Ok(())
}

// -- WebSocket --------------------------------------------------------------

fn require_network(worker: &WorkerRuntime) -> Result<(), String> {
    if worker.env.settings.allow_network {
        Ok(())
    } else {
        Err("插件网络访问已在设置中关闭".into())
    }
}

fn send_ws_event(events: &mpsc::UnboundedSender<WorkerCommand>, name: &str, payload: Value) {
    let _ = events.send(WorkerCommand::Event {
        name: name.to_string(),
        payload,
        reply: None,
    });
}

fn close_wire(
    frame: Option<tokio_tungstenite::tungstenite::protocol::CloseFrame>,
) -> (Option<u16>, String) {
    frame
        .map(|frame| (Some(u16::from(frame.code)), frame.reason.to_string()))
        .unwrap_or((None, String::new()))
}

fn incoming_json(message: WsIncoming) -> Value {
    match message {
        WsIncoming::Text(text) => json!({ "type": "text", "text": text }),
        WsIncoming::Binary(data) => json!({
            "type": "binary",
            "base64": STANDARD.encode(&data),
            "size": data.len(),
        }),
        WsIncoming::Closed(code, reason) => {
            json!({ "type": "close", "code": code, "reason": reason })
        }
    }
}

async fn ws_connect(
    _this: &JsValue,
    args: &[JsValue],
    context: &RefCell<&mut Context>,
) -> JsResult<JsValue> {
    let worker = current()?;
    require_network(&worker).map_err(js_error)?;
    let options = {
        let mut ctx = context.borrow_mut();
        arg_json(args, 0, &mut ctx)
    };
    let result = ws_connect_impl(&worker, options).await;
    let mut ctx = context.borrow_mut();
    match result {
        Ok(value) => json_value(&value, &mut ctx),
        Err(err) => Err(js_error(err)),
    }
}

async fn ws_connect_impl(worker: &WorkerRuntime, options: Value) -> Result<Value, String> {
    let url = options
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| url.starts_with("ws://") || url.starts_with("wss://"))
        .ok_or_else(|| "host.ws.connect 需要 ws:// 或 wss:// url".to_string())?
        .to_string();
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|err| format!("WebSocket URL 无效：{err}"))?;
    if let Some(headers) = options.get("headers").and_then(Value::as_object) {
        for (name, value) in headers {
            let Some(value) = value.as_str() else {
                continue;
            };
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|err| format!("Header 名称无效：{err}"))?;
            let value =
                HeaderValue::from_str(value).map_err(|err| format!("Header 值无效：{err}"))?;
            request.headers_mut().insert(name, value);
        }
    }
    let timeout = options
        .get("timeoutSecs")
        .and_then(Value::as_u64)
        .unwrap_or(15)
        .clamp(1, 120);
    let (stream, response) =
        tokio::time::timeout(Duration::from_secs(timeout), connect_async(request))
            .await
            .map_err(|_| format!("WebSocket 连接超时（{timeout}s）"))?
            .map_err(|err| format!("WebSocket 连接失败：{err}"))?;
    let status = response.status().as_u16();

    let (mut sink, mut source) = stream.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            if sink.send(message).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });
    let (in_tx, in_rx) = mpsc::unbounded_channel::<WsIncoming>();
    let pong_tx = out_tx.clone();
    let reader = tokio::spawn(async move {
        let mut sent_close = false;
        while let Some(message) = source.next().await {
            match message {
                Ok(Message::Text(text)) => {
                    let _ = in_tx.send(WsIncoming::Text(text.to_string()));
                }
                Ok(Message::Binary(data)) => {
                    let _ = in_tx.send(WsIncoming::Binary(data.to_vec()));
                }
                Ok(Message::Close(frame)) => {
                    let (code, reason) = close_wire(frame);
                    let _ = in_tx.send(WsIncoming::Closed(code, reason));
                    sent_close = true;
                    break;
                }
                Ok(Message::Ping(payload)) => {
                    let _ = pong_tx.send(Message::Pong(payload));
                }
                Ok(_) => {}
                Err(err) => {
                    let _ = in_tx.send(WsIncoming::Closed(None, err.to_string()));
                    sent_close = true;
                    break;
                }
            }
        }
        if !sent_close {
            let _ = in_tx.send(WsIncoming::Closed(None, String::new()));
        }
    });
    let id = worker.ws.next_id();
    worker
        .ws
        .clients
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            id,
            WsClient {
                tx: out_tx,
                rx: Some(in_rx),
                reader: Some(reader),
                writer: Some(writer),
            },
        );
    Ok(json!({ "id": id, "url": url, "status": status }))
}

async fn ws_serve(
    _this: &JsValue,
    args: &[JsValue],
    context: &RefCell<&mut Context>,
) -> JsResult<JsValue> {
    let worker = current()?;
    require_network(&worker).map_err(js_error)?;
    let options = {
        let mut ctx = context.borrow_mut();
        arg_json(args, 0, &mut ctx)
    };
    let result = ws_serve_impl(&worker, options).await;
    let mut ctx = context.borrow_mut();
    match result {
        Ok(value) => json_value(&value, &mut ctx),
        Err(err) => Err(js_error(err)),
    }
}

async fn ws_serve_impl(worker: &WorkerRuntime, options: Value) -> Result<Value, String> {
    let host = options
        .get("host")
        .and_then(Value::as_str)
        .unwrap_or("127.0.0.1")
        .trim()
        .to_string();
    if host.is_empty() {
        return Err("host.ws.serve 的 host 不能为空".into());
    }
    let port = options.get("port").and_then(Value::as_u64).unwrap_or(0);
    if port > u16::MAX as u64 {
        return Err("host.ws.serve 的 port 必须在 0–65535 之间".into());
    }
    let port = port as u16;
    let path = options
        .get("path")
        .and_then(Value::as_str)
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty());
    if let Some(path) = &path {
        if !path.starts_with('/') {
            return Err("host.ws.serve 的 path 必须以 / 开头".into());
        }
    }
    let listener = TcpListener::bind(format!("{host}:{port}"))
        .await
        .map_err(|err| format!("WebSocket 监听 {host}:{port} 失败：{err}"))?;
    let local = listener
        .local_addr()
        .map_err(|err| format!("读取 WebSocket 监听地址失败：{err}"))?;
    let server_id = worker.ws.next_id();
    let cancel = CancellationToken::new();
    let connections: Arc<Mutex<HashMap<u64, WsConnection>>> = Arc::new(Mutex::new(HashMap::new()));
    let task_connections = connections.clone();
    let task_cancel = cancel.clone();
    let events = worker.events.clone();
    let state = worker.ws.clone();
    let task_path = path.clone();
    let task = tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                _ = task_cancel.cancelled() => break,
                result = listener.accept() => result,
            };
            let (tcp, remote) = match accepted {
                Ok(value) => value,
                Err(_) => break,
            };
            let connections = task_connections.clone();
            let cancel = task_cancel.clone();
            let events = events.clone();
            let state = state.clone();
            let path = task_path.clone();
            tokio::spawn(async move {
                let handshake = accept_hdr_async(tcp, PathGuard { path: path.clone() }).await;
                let ws = match handshake {
                    Ok(ws) => ws,
                    Err(_) => return,
                };
                if cancel.is_cancelled() {
                    return;
                }
                let conn_id = state.next_id();
                let (mut sink, mut source) = ws.split();
                let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
                let writer = tokio::spawn(async move {
                    while let Some(message) = out_rx.recv().await {
                        if sink.send(message).await.is_err() {
                            break;
                        }
                    }
                    let _ = sink.close().await;
                });
                connections
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(
                        conn_id,
                        WsConnection {
                            tx: out_tx.clone(),
                            reader: None,
                            writer: Some(writer),
                        },
                    );
                send_ws_event(
                    &events,
                    "onWebSocketConnection",
                    json!({
                        "event": "onWebSocketConnection",
                        "serverId": server_id,
                        "connectionId": conn_id,
                        "remote": remote.to_string(),
                        "path": path,
                    }),
                );
                let reader_events = events.clone();
                let reader_connections = connections.clone();
                let pong_tx = out_tx.clone();
                let reader = tokio::spawn(async move {
                    let mut sent_close = false;
                    while let Some(message) = source.next().await {
                        match message {
                            Ok(Message::Text(text)) => {
                                let text = text.to_string();
                                send_ws_event(
                                    &reader_events,
                                    "onWebSocketMessage",
                                    json!({
                                        "event": "onWebSocketMessage",
                                        "serverId": server_id,
                                        "connectionId": conn_id,
                                        "type": "text",
                                        "text": text,
                                        "size": text.len(),
                                    }),
                                );
                            }
                            Ok(Message::Binary(data)) => {
                                let size = data.len();
                                send_ws_event(
                                    &reader_events,
                                    "onWebSocketMessage",
                                    json!({
                                        "event": "onWebSocketMessage",
                                        "serverId": server_id,
                                        "connectionId": conn_id,
                                        "type": "binary",
                                        "base64": STANDARD.encode(&data),
                                        "size": size,
                                    }),
                                );
                            }
                            Ok(Message::Close(frame)) => {
                                let (code, reason) = close_wire(frame);
                                send_ws_event(
                                    &reader_events,
                                    "onWebSocketClosed",
                                    json!({
                                        "event": "onWebSocketClosed",
                                        "serverId": server_id,
                                        "connectionId": conn_id,
                                        "code": code,
                                        "reason": reason,
                                    }),
                                );
                                sent_close = true;
                                break;
                            }
                            Ok(Message::Ping(payload)) => {
                                let _ = pong_tx.send(Message::Pong(payload));
                            }
                            Ok(_) => {}
                            Err(err) => {
                                send_ws_event(
                                    &reader_events,
                                    "onWebSocketClosed",
                                    json!({
                                        "event": "onWebSocketClosed",
                                        "serverId": server_id,
                                        "connectionId": conn_id,
                                        "code": null,
                                        "reason": err.to_string(),
                                    }),
                                );
                                sent_close = true;
                                break;
                            }
                        }
                    }
                    if !sent_close {
                        send_ws_event(
                            &reader_events,
                            "onWebSocketClosed",
                            json!({
                                "event": "onWebSocketClosed",
                                "serverId": server_id,
                                "connectionId": conn_id,
                                "code": null,
                                "reason": "",
                            }),
                        );
                    }
                    reader_connections
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&conn_id);
                });
                if let Some(connection) = connections
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get_mut(&conn_id)
                {
                    connection.reader = Some(reader);
                }
            });
        }
    });
    worker
        .ws
        .servers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            server_id,
            WsServer {
                task,
                cancel,
                connections,
                host: host.clone(),
                port: local.port(),
                path: path.clone(),
            },
        );
    Ok(json!({
        "id": server_id,
        "host": host,
        "port": local.port(),
        "path": path,
    }))
}

async fn ws_recv(
    _this: &JsValue,
    args: &[JsValue],
    context: &RefCell<&mut Context>,
) -> JsResult<JsValue> {
    let worker = current()?;
    require_network(&worker).map_err(js_error)?;
    let id = arg_number(args, 0) as u64;
    let timeout = args
        .get(1)
        .map(|_| arg_number(args, 1))
        .unwrap_or(30.0)
        .clamp(0.1, 600.0);
    let rx = {
        let mut clients = worker.ws.clients.lock().unwrap_or_else(|e| e.into_inner());
        clients.get_mut(&id).and_then(|client| client.rx.take())
    };
    let Some(mut rx) = rx else {
        return Err(js_error("WebSocket 连接不存在或已有 recv 在等待"));
    };
    let result = tokio::time::timeout(Duration::from_secs_f64(timeout), rx.recv()).await;
    let closed = matches!(result, Ok(Some(WsIncoming::Closed(..))) | Ok(None));
    {
        let mut clients = worker.ws.clients.lock().unwrap_or_else(|e| e.into_inner());
        if closed {
            if let Some(client) = clients.remove(&id) {
                abort_client(client);
            }
        } else if let Some(client) = clients.get_mut(&id) {
            client.rx = Some(rx);
        }
    }
    let payload = match result {
        Ok(Some(message)) => incoming_json(message),
        Ok(None) => json!({ "type": "close" }),
        Err(_) => json!({ "type": "timeout" }),
    };
    let mut ctx = context.borrow_mut();
    json_value(&payload, &mut ctx)
}

fn ws_send(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    require_network(&worker).map_err(js_error)?;
    let id = arg_number(args, 0) as u64;
    let text = arg_string(args, 1);
    let tx = worker
        .ws
        .sender_for(id)
        .ok_or_else(|| js_error("WebSocket 连接不存在"))?;
    tx.send(Message::Text(text.into()))
        .map_err(|_| js_error("WebSocket 已关闭"))?;
    Ok(JsValue::from(true))
}

fn ws_send_binary(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    require_network(&worker).map_err(js_error)?;
    let id = arg_number(args, 0) as u64;
    let data = STANDARD
        .decode(arg_string(args, 1))
        .map_err(|err| js_error(format!("二进制数据不是有效的 base64：{err}")))?;
    let tx = worker
        .ws
        .sender_for(id)
        .ok_or_else(|| js_error("WebSocket 连接不存在"))?;
    tx.send(Message::Binary(data.into()))
        .map_err(|_| js_error("WebSocket 已关闭"))?;
    Ok(JsValue::from(true))
}

fn ws_close(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    require_network(&worker).map_err(js_error)?;
    let id = arg_number(args, 0) as u64;
    Ok(JsValue::from(worker.ws.close(id)))
}

fn ws_stop(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    require_network(&worker).map_err(js_error)?;
    let id = arg_number(args, 0) as u64;
    Ok(JsValue::from(worker.ws.stop(id)))
}

fn ws_list(_this: &JsValue, _args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    let clients: Vec<u64> = worker
        .ws
        .clients
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .copied()
        .collect();
    let servers: Vec<Value> = worker
        .ws
        .servers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|(id, server)| {
            let connections = server
                .connections
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len();
            json!({
                "id": id,
                "host": server.host,
                "port": server.port,
                "path": server.path,
                "connections": connections,
            })
        })
        .collect();
    let payload = json!({ "clients": clients, "servers": servers });
    json_value(&payload, _ctx)
}

// -- filesystem (base64) ----------------------------------------------------

fn fs_read_base64(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    let path = safe_path(&worker, &arg_string(args, 0)).map_err(js_error)?;
    let bytes = std::fs::read(&path)
        .map_err(|err| js_error(format!("读取 {} 失败：{err}", path.display())))?;
    Ok(JsValue::from(js_string!(STANDARD.encode(bytes).as_str())))
}

fn fs_write_base64(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    let path = safe_path(&worker, &arg_string(args, 0)).map_err(js_error)?;
    let bytes = STANDARD
        .decode(arg_string(args, 1).trim())
        .map_err(|err| js_error(format!("Base64 解码失败：{err}")))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| js_error(format!("创建目录失败：{err}")))?;
    }
    std::fs::write(&path, bytes)
        .map_err(|err| js_error(format!("写入 {} 失败：{err}", path.display())))?;
    Ok(JsValue::undefined())
}

// -- crypto helpers ---------------------------------------------------------

fn crypto_constant_time_equal(
    _this: &JsValue,
    args: &[JsValue],
    _ctx: &mut Context,
) -> JsResult<JsValue> {
    let left = arg_string(args, 0);
    let right = arg_string(args, 1);
    let a = left.as_bytes();
    let b = right.as_bytes();
    let mut diff = (a.len() ^ b.len()) as u8;
    // Compare up to the longer length without early exit.
    let len = a.len().max(b.len());
    for index in 0..len {
        let x = a.get(index).copied().unwrap_or(0);
        let y = b.get(index).copied().unwrap_or(0);
        diff |= x ^ y;
    }
    Ok(JsValue::new(diff == 0))
}

// -- persistent storage -----------------------------------------------------

fn storage_path(worker: &WorkerRuntime) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&worker.env.data_dir)
        .map_err(|err| format!("创建插件数据目录失败：{err}"))?;
    Ok(worker.env.data_dir.join("storage.json"))
}

fn storage_load(worker: &WorkerRuntime) -> Result<serde_json::Map<String, Value>, String> {
    let path = storage_path(worker)?;
    if !path.exists() {
        return Ok(serde_json::Map::new());
    }
    let text = std::fs::read_to_string(&path).map_err(|err| format!("读取插件存储失败：{err}"))?;
    if text.trim().is_empty() {
        return Ok(serde_json::Map::new());
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err("插件存储文件不是 JSON 对象".into()),
    }
}

fn storage_save(
    worker: &WorkerRuntime,
    map: &serde_json::Map<String, Value>,
) -> Result<(), String> {
    let path = storage_path(worker)?;
    let text = serde_json::to_string_pretty(map).map_err(|err| format!("序列化存储失败：{err}"))?;
    std::fs::write(&path, text).map_err(|err| format!("写入插件存储失败：{err}"))
}

fn storage_get(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    let key = arg_string(args, 0);
    let map = storage_load(&worker).map_err(js_error)?;
    match map.get(&key) {
        Some(value) => JsValue::from_json(value, ctx),
        None => Ok(JsValue::null()),
    }
}

fn storage_set(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    let key = arg_string(args, 0);
    if key.is_empty() {
        return Err(js_error("storage.set 缺少 key"));
    }
    let value = args
        .get(1)
        .map(|value| value.to_json(ctx))
        .transpose()?
        .flatten()
        .unwrap_or(Value::Null);
    let mut map = storage_load(&worker).map_err(js_error)?;
    map.insert(key, value);
    storage_save(&worker, &map).map_err(js_error)?;
    Ok(JsValue::undefined())
}

fn storage_remove(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    let key = arg_string(args, 0);
    let mut map = storage_load(&worker).map_err(js_error)?;
    let removed = map.remove(&key).is_some();
    storage_save(&worker, &map).map_err(js_error)?;
    Ok(JsValue::new(removed))
}

fn storage_keys(_this: &JsValue, _args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    let map = storage_load(&worker).map_err(js_error)?;
    let keys: Vec<String> = map.keys().cloned().collect();
    JsValue::from_json(&json!(keys), ctx)
}

// -- time / util ------------------------------------------------------------

fn time_now(_this: &JsValue, _args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::new(crate::config::now_ts() as f64 * 1000.0))
}

fn time_now_iso(_this: &JsValue, _args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let text = chrono::Local::now().to_rfc3339();
    Ok(JsValue::from(js_string!(text.as_str())))
}

fn util_uuid(_this: &JsValue, _args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let text = uuid::Uuid::new_v4().to_string();
    Ok(JsValue::from(js_string!(text.as_str())))
}

// -- network ----------------------------------------------------------------

async fn http_request(
    _this: &JsValue,
    args: &[JsValue],
    context: &RefCell<&mut Context>,
) -> JsResult<JsValue> {
    let worker = current()?;
    if !worker.env.settings.allow_network {
        return Err(js_error("插件网络访问已在设置中关闭"));
    }
    let options = {
        let mut ctx = context.borrow_mut();
        arg_json(args, 0, &mut ctx)
    };
    let result = http_request_impl(&worker, options).await;
    let mut ctx = context.borrow_mut();
    match result {
        Ok(value) => json_value(&value, &mut ctx),
        Err(err) => Err(js_error(err)),
    }
}

async fn http_request_impl(worker: &WorkerRuntime, options: Value) -> Result<Value, String> {
    let url = options
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| !url.trim().is_empty())
        .ok_or_else(|| "host.http.request 缺少 url".to_string())?;
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("只允许 http:// 或 https:// 地址".into());
    }
    let method = options
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let method = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|err| format!("HTTP 方法无效：{err}"))?;
    // Loopback targets (local mock servers, LM Studio, ...) must not go
    // through the system proxy.
    let client = if crate::gateway::is_loopback_url(url) {
        &worker.env.http_local
    } else {
        &worker.env.http
    };
    let mut request = client.request(method, url);
    if let Some(headers) = options.get("headers").and_then(Value::as_object) {
        for (name, value) in headers {
            if let Some(value) = value.as_str() {
                request = request.header(name, value);
            }
        }
    }
    if let Some(body) = options.get("body") {
        request = if let Some(text) = body.as_str() {
            request.body(text.to_string())
        } else {
            request.json(body)
        };
    }
    let timeout = options
        .get("timeoutSecs")
        .and_then(Value::as_u64)
        .unwrap_or(30)
        .clamp(1, 120);
    let response = tokio::time::timeout(Duration::from_secs(timeout), request.send())
        .await
        .map_err(|_| format!("HTTP 请求超时（{timeout}s）"))?
        .map_err(|err| format!("HTTP 请求失败：{err}"))?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_string(), Value::String(value.to_string())))
        })
        .collect::<serde_json::Map<_, _>>();
    let body = response
        .text()
        .await
        .map_err(|err| format!("读取响应失败：{err}"))?;
    Ok(json!({ "status": status, "headers": headers, "body": body }))
}

async fn http_download(
    _this: &JsValue,
    args: &[JsValue],
    context: &RefCell<&mut Context>,
) -> JsResult<JsValue> {
    let worker = current()?;
    if !worker.env.settings.allow_network {
        return Err(js_error("插件网络访问已在设置中关闭"));
    }
    let url = arg_string(args, 0);
    let path = arg_string(args, 1);
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(js_error("只允许 http:// 或 https:// 地址"));
    }
    let client = if crate::gateway::is_loopback_url(&url) {
        &worker.env.http_local
    } else {
        &worker.env.http
    };
    let response = client
        .get(&url)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|err| js_error(format!("下载失败：{err}")))?;
    let status = response.status().as_u16();
    let bytes = response
        .bytes()
        .await
        .map_err(|err| js_error(format!("读取下载内容失败：{err}")))?;
    if !(200..300).contains(&status) {
        return Err(js_error(format!("下载返回 HTTP {status}")));
    }
    let target = safe_path(&worker, &path).map_err(js_error)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|err| js_error(format!("创建目录失败：{err}")))?;
    }
    std::fs::write(&target, &bytes)
        .map_err(|err| js_error(format!("写入 {} 失败：{err}", target.display())))?;
    let value = json!({ "status": status, "bytes": bytes.len() });
    let mut ctx = context.borrow_mut();
    json_value(&value, &mut ctx)
}

// -- process ----------------------------------------------------------------

async fn process_run(
    _this: &JsValue,
    args: &[JsValue],
    context: &RefCell<&mut Context>,
) -> JsResult<JsValue> {
    let worker = current()?;
    if !worker.env.settings.allow_process {
        return Err(js_error("插件进程调用已在设置中关闭"));
    }
    let options = {
        let mut ctx = context.borrow_mut();
        arg_json(args, 0, &mut ctx)
    };
    let result = process_run_impl(options).await;
    let mut ctx = context.borrow_mut();
    match result {
        Ok(value) => json_value(&value, &mut ctx),
        Err(err) => Err(js_error(err)),
    }
}

async fn process_run_impl(options: Value) -> Result<Value, String> {
    use std::process::Stdio;
    let command = options
        .get("command")
        .and_then(Value::as_str)
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| "host.process.run 缺少 command".to_string())?;
    let mut child = std::process::Command::new(command);
    if let Some(items) = options.get("args").and_then(Value::as_array) {
        for item in items {
            if let Some(value) = item.as_str() {
                child.arg(value);
            } else {
                child.arg(item.to_string());
            }
        }
    }
    if let Some(cwd) = options.get("cwd").and_then(Value::as_str) {
        child.current_dir(cwd);
    }
    if let Some(envs) = options.get("env").and_then(Value::as_object) {
        for (name, value) in envs {
            if let Some(value) = value.as_str() {
                child.env(name, value);
            }
        }
    }
    child.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = child
        .spawn()
        .map_err(|err| format!("启动进程失败：{err}"))?;
    let timeout = options
        .get("timeoutSecs")
        .and_then(Value::as_u64)
        .unwrap_or(30)
        .clamp(1, 300);
    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("进程执行超时（{timeout}s）"));
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(err) => return Err(format!("等待进程失败：{err}")),
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|err| format!("读取进程输出失败：{err}"))?;
    Ok(json!({
        "code": output.status.code(),
        "success": output.status.success(),
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
    }))
}

// -- UI ---------------------------------------------------------------------

fn ui_toast(_this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let worker = current()?;
    let level = arg_string(args, 0);
    let message = arg_string(args, 1);
    let level = match level.to_ascii_lowercase().as_str() {
        "warn" | "warning" => crate::snapshot::NoticeLevel::Warning,
        "error" => crate::snapshot::NoticeLevel::Error,
        "success" => crate::snapshot::NoticeLevel::Success,
        _ => crate::snapshot::NoticeLevel::Info,
    };
    worker
        .env
        .shared
        .notice(level, format!("插件 {}：{message}", worker.env.id));
    Ok(JsValue::undefined())
}

async fn ui_confirm(
    _this: &JsValue,
    args: &[JsValue],
    context: &RefCell<&mut Context>,
) -> JsResult<JsValue> {
    let worker = current()?;
    let (message, title) = {
        let mut ctx = context.borrow_mut();
        let message = arg_string(args, 0);
        let title = args
            .get(1)
            .and_then(|value| value.as_string())
            .map(|value| value.to_std_string_escaped())
            .unwrap_or_else(|| "插件确认".to_string());
        let _ = &mut ctx;
        (message, title)
    };
    let receiver = worker.env.shared.request_ui(
        &worker.env.id,
        &worker.name.borrow(),
        PluginUiKind::Confirm,
        title,
        message,
        String::new(),
    );
    match tokio::time::timeout(Duration::from_secs(120), receiver).await {
        Ok(Ok(answer)) => Ok(JsValue::new(answer.accepted)),
        _ => Ok(JsValue::new(false)),
    }
}

async fn ui_prompt(
    _this: &JsValue,
    args: &[JsValue],
    context: &RefCell<&mut Context>,
) -> JsResult<JsValue> {
    let worker = current()?;
    let (message, default_value, title) = {
        let mut ctx = context.borrow_mut();
        let message = arg_string(args, 0);
        let default_value = args
            .get(1)
            .and_then(|value| value.as_string())
            .map(|value| value.to_std_string_escaped())
            .unwrap_or_default();
        let title = args
            .get(2)
            .and_then(|value| value.as_string())
            .map(|value| value.to_std_string_escaped())
            .unwrap_or_else(|| "插件输入".to_string());
        let _ = &mut ctx;
        (message, default_value, title)
    };
    let receiver = worker.env.shared.request_ui(
        &worker.env.id,
        &worker.name.borrow(),
        PluginUiKind::Prompt,
        title,
        message,
        default_value,
    );
    match tokio::time::timeout(Duration::from_secs(120), receiver).await {
        Ok(Ok(answer)) if answer.accepted => Ok(JsValue::from(js_string!(answer.text.as_str()))),
        _ => Ok(JsValue::null()),
    }
}

async fn ui_alert(
    _this: &JsValue,
    args: &[JsValue],
    context: &RefCell<&mut Context>,
) -> JsResult<JsValue> {
    let worker = current()?;
    let (message, title) = {
        let mut ctx = context.borrow_mut();
        let message = arg_string(args, 0);
        let title = args
            .get(1)
            .and_then(|value| value.as_string())
            .map(|value| value.to_std_string_escaped())
            .unwrap_or_else(|| "插件提示".to_string());
        let _ = &mut ctx;
        (message, title)
    };
    let receiver = worker.env.shared.request_ui(
        &worker.env.id,
        &worker.name.borrow(),
        PluginUiKind::Alert,
        title,
        message,
        String::new(),
    );
    let _ = tokio::time::timeout(Duration::from_secs(120), receiver).await;
    Ok(JsValue::undefined())
}

// -- small helpers ----------------------------------------------------------

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(text: &str) -> Result<Vec<u8>, String> {
    let text = text.trim();
    if text.len() % 2 != 0 {
        return Err("十六进制字符串长度必须为偶数".into());
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    for index in (0..bytes.len()).step_by(2) {
        let high = (bytes[index] as char)
            .to_digit(16)
            .ok_or_else(|| "十六进制字符无效".to_string())?;
        let low = (bytes[index + 1] as char)
            .to_digit(16)
            .ok_or_else(|| "十六进制字符无效".to_string())?;
        out.push(((high << 4) | low) as u8);
    }
    Ok(out)
}
