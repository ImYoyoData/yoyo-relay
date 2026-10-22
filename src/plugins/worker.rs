//! Dedicated plugin thread: owns the Boa `Context`, loads the ES module and
//! executes event handlers and host API calls. Nothing in this module is
//! `Send`; only values crossing the channel boundary are.

use std::cell::RefCell;
use std::path::{Component, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
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
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio::sync::oneshot;

use super::{PluginStatus, PluginUiKind, WorkerCommand, WorkerEnv, WorkerMeta};

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
    "selfTest",
];

struct WorkerRuntime {
    env: WorkerEnv,
    name: RefCell<String>,
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<WorkerRuntime>>> = const { RefCell::new(None) };
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
) {
    let _enter = env.handle.enter();
    let worker = Arc::new(WorkerRuntime {
        name: RefCell::new(env.id.clone()),
        env,
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
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            }
            WorkerCommand::Shutdown { reply } => {
                let _ = session.invoke("onShutdown", json!({ "event": "onShutdown" }));
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
        );
    let crypto_object = crypto.build();

    let mut http = ObjectInitializer::new(ctx);
    http.function(
        NativeFunction::from_async_fn(http_request),
        js_string!("request"),
        1,
    );
    let http_object = http.build();

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
            .property(js_string!("process"), process_object, Attribute::all())
            .property(js_string!("ui"), ui_object, Attribute::all())
            .property(js_string!("app"), app_object, Attribute::all());
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
