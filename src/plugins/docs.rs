//! Embedded plugin API documentation and sample plugin installer.

use std::path::Path;

pub const API_DOC: &str = r#"# YoyoRelay 插件 API（ES Module）

每个插件是一个目录（或单个 .js 文件），重启后自动发现。

目录结构：
  plugins/
    hello/
      plugin.json      // 可选：{ "name", "version", "description", "main", "events" }
      main.js          // ES Module 入口
    single.js          // 或单文件插件

插件数据目录（host.fs 的根）：
  <配置目录>/plugin-data/<插件 id>/

生命周期与事件：
  onStartup(event)           加载并求值模块后调用
  onEnable(event)            启用时调用
  onDisable(event)           禁用前调用
  onShutdown(event)          卸载前调用
  onReload(event)            重新加载时调用
  beforeRequest(event)       请求转发前；可修改请求体或拦截
  afterResponse(event)       非流式响应返回前；可修改响应 JSON
  onRequestCompleted(event)  请求记录完成后通知（只读）
  onAccountFailure(event)    账号失败时通知（只读）
  onBalanceUpdated(event)    余额刷新后通知（只读）
  selfTest(event)            自定义；插件页的「自检」按钮会调用

beforeRequest 返回值：
  null / undefined                 不修改
  { payload: {...} }               替换请求体
  { block: true, status, response } 直接拦截，response 作为 JSON 返回
  直接返回一个带 model/messages 的对象       视为替换请求体

afterResponse 返回值：
  null / undefined          不修改
  { response: {...} }       替换响应
  直接返回带 choices/output 的对象            视为替换响应

宿主 API（全局 host 对象）：
  host.log(level, message)                日志（info/warn/error/debug）
  await host.sleep(ms)                    异步等待
  host.app.version / endpoint / models    运行时信息（只读）

  host.fs.readText(path)                  读文本
  host.fs.writeText(path, text)           写文本
  host.fs.appendText(path, text)          追加文本
  host.fs.exists(path)                    是否存在
  host.fs.list(path)                      列出目录
  host.fs.remove(path)                    删除文件/目录
  host.fs.mkdir(path)                     创建目录
  路径只能是数据目录内的相对路径，不能包含 ..

  host.encoding.base64Encode(text) / base64Decode(text)
  host.encoding.base64UrlEncode(text) / base64UrlDecode(text)
  host.encoding.urlEncode(text) / urlDecode(text)
  host.encoding.hexEncode(text) / hexDecode(text)

  host.crypto.sha256(text)                返回十六进制
  host.crypto.hmacSha256(secret, text)    返回十六进制
  host.crypto.randomBytes(n)              返回十六进制
  host.crypto.aesGcmEncrypt(keyHex, text) 返回 { nonce, ciphertext }
  host.crypto.aesGcmDecrypt(keyHex, nonceHex, ciphertext)

  await host.http.request({ url, method, headers, body, timeoutSecs })
      返回 { status, headers, body }；受「允许网络」开关限制

  await host.process.run({ command, args, cwd, env, timeoutSecs })
      返回 { code, success, stdout, stderr }；默认关闭，需在插件页开启

  host.ui.toast(level, message)           在界面弹出提示
  await host.ui.confirm(message, title)   返回 boolean
  await host.ui.prompt(message, default, title) 返回 string 或 null

限制：
  - 每个插件独立线程与 JS Context，插件之间不共享状态
  - 循环约 2000 万次、递归约 2000 层后抛出 JS 错误
  - beforeRequest / afterResponse 每个插件最多等待 3 秒，超时会跳过
  - async/await 可用；host.sleep、http、process、ui 均返回 Promise
"#;

const HELLO_JS: &str = r#"export const name = "Hello Plugin";
export const version = "1.0.0";
export const description = "记录启动事件并把完成的请求追加到数据目录。";
export const events = ["onStartup", "onRequestCompleted"];

export function onStartup(event) {
  host.log("info", `Hello from ${event.plugin.name} on ${host.app.version}`);
}

export function onRequestCompleted(event) {
  const line = JSON.stringify({
    at: new Date().toISOString(),
    model: event.request.model,
    account: event.request.account,
    totalTokens: event.usage.totalTokens,
    ok: event.request.ok,
  });
  host.fs.appendText("requests.jsonl", line + "\n");
}
"#;

const GUARD_JS: &str = r#"export const name = "Request Guard";
export const version = "1.0.0";
export const description = "拦截包含 block-me 的请求，演示 beforeRequest 的阻止能力。";
export const events = ["beforeRequest"];

export function beforeRequest(event) {
  const messages = event.request.payload?.messages;
  const text = JSON.stringify(messages ?? "");
  if (text.includes("block-me")) {
    return {
      block: true,
      status: 403,
      response: {
        error: {
          message: "请求被 Request Guard 插件拦截",
          type: "plugin_blocked",
        },
      },
    };
  }
  return null;
}
"#;

const TOOLKIT_JS: &str = r#"export const name = "Toolkit Self Test";
export const version = "1.0.0";
export const description = "自检 fs、编码、哈希与异步 API。";
export const events = ["selfTest"];

export async function selfTest() {
  host.fs.writeText("demo.txt", "hello-plugin");
  await host.sleep(5);
  const read = host.fs.readText("demo.txt");
  const b64 = host.encoding.base64Encode("插件");
  const roundtrip = host.encoding.base64Decode(b64);
  const sha = host.crypto.sha256("yoyo");
  return {
    read,
    b64,
    roundtrip,
    sha,
    random: host.crypto.randomBytes(6),
  };
}
"#;

const HELLO_MANIFEST: &str = r#"{
  "name": "Hello Plugin",
  "version": "1.0.0",
  "description": "记录启动事件并把完成的请求追加到数据目录。",
  "main": "main.js",
  "events": ["onStartup", "onRequestCompleted"]
}
"#;

const GUARD_MANIFEST: &str = r#"{
  "name": "Request Guard",
  "version": "1.0.0",
  "description": "拦截包含 block-me 的请求，演示 beforeRequest 的阻止能力。",
  "main": "main.js",
  "events": ["beforeRequest"]
}
"#;

const TOOLKIT_MANIFEST: &str = r#"{
  "name": "Toolkit Self Test",
  "version": "1.0.0",
  "description": "自检 fs、编码、哈希与异步 API。",
  "main": "main.js",
  "events": ["selfTest"]
}
"#;

fn write_plugin(
    root: &Path,
    dir: &str,
    manifest: &str,
    code: &str,
    installed: &mut Vec<String>,
) -> Result<(), String> {
    let plugin_dir = root.join(dir);
    std::fs::create_dir_all(&plugin_dir).map_err(|err| format!("创建示例插件失败：{err}"))?;
    let manifest_path = plugin_dir.join("plugin.json");
    let main_path = plugin_dir.join("main.js");
    if !manifest_path.exists() {
        std::fs::write(&manifest_path, manifest)
            .map_err(|err| format!("写入示例插件失败：{err}"))?;
    }
    if !main_path.exists() {
        std::fs::write(&main_path, code).map_err(|err| format!("写入示例插件失败：{err}"))?;
    }
    installed.push(dir.to_string());
    Ok(())
}

/// Write the bundled samples into `plugins_dir`, never overwriting existing
/// files. Returns the plugin directory names touched.
pub fn install_samples(plugins_dir: &Path) -> Result<Vec<String>, String> {
    let mut installed = Vec::new();
    write_plugin(
        plugins_dir,
        "hello-plugin",
        HELLO_MANIFEST,
        HELLO_JS,
        &mut installed,
    )?;
    write_plugin(
        plugins_dir,
        "request-guard",
        GUARD_MANIFEST,
        GUARD_JS,
        &mut installed,
    )?;
    write_plugin(
        plugins_dir,
        "toolkit",
        TOOLKIT_MANIFEST,
        TOOLKIT_JS,
        &mut installed,
    )?;
    Ok(installed)
}
