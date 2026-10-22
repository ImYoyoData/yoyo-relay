# YoyoRelay 插件系统

YoyoRelay 使用 [Boa](https://github.com/boa-dev/boa) 作为 JavaScript 引擎。插件是标准 ES Module，运行在各自的专用线程和 `Context` 中；插件之间没有共享的 JS 状态，卸载时会直接丢弃 `Context` 并由 Boa GC 回收全部对象。

## 目录结构

插件目录位于配置目录下：

```
<配置目录>/plugins/
  hello-plugin/
    plugin.json      # 可选清单
    main.js          # ES Module 入口
  single-file.js     # 也可以直接放单个 .js 文件
```

`plugin.json`：

```json
{
  "name": "Hello Plugin",
  "version": "1.0.0",
  "description": "示例插件",
  "main": "main.js",
  "author": "you",
  "events": ["onStartup", "beforeRequest"]
}
```

`host.fs` 的根目录是 `<配置目录>/plugin-data/<插件目录名>/`，插件只能访问该目录内的相对路径。插件页右上角可以一键写入三个示例插件，或把插件目录放到上述路径后点击「重新扫描」。

## 运行模型

- 每个插件一个专用 `std::thread`；`Context` 只在该线程创建、使用、销毁。
- 宿主通过 `mpsc` 发送事件命令，插件线程串行执行；卸载命令会先调用 `onShutdown`，再释放 `Context`。
- 事件默认最多等待 3 秒；超时会跳过该插件，不会阻塞整个代理。
- 循环约 2000 万次、递归约 2000 层后抛出 JS 错误，而不是挂死进程。
- `host.http`、`host.process`、`host.ui.*` 与 `host.sleep` 返回 Promise，插件里可以直接 `await`。

## 生命周期与事件

| 事件 | 时机 | 返回值 |
|:---|:---|:---|
| `onStartup(event)` | 模块求值完成后 | 忽略 |
| `onEnable(event)` | 启用插件后 | 忽略 |
| `onDisable(event)` | 禁用前 | 忽略 |
| `onShutdown(event)` | 卸载前 | 忽略 |
| `onRequestCompleted(event)` | 请求写入用量后 | 忽略（只读） |
| `onAccountFailure(event)` | 上游账号失败时 | 忽略（只读） |
| `beforeRequest(event)` | 请求进入调度前 | 修改请求体或拦截 |
| `afterResponse(event)` | 非流式响应返回前 | 修改响应 JSON |
| `selfTest(event)` | 插件页「自检」按钮 | 任意 JSON，显示在通知与日志 |

`beforeRequest` 返回值：

```js
// 不修改
return null;

// 替换请求体
return { payload: { ...event.request.payload, model: "another-model" } };

// 直接拦截：response 会作为 JSON 返回给客户端
return {
  block: true,
  status: 403,
  response: { error: { message: "blocked", type: "plugin_blocked" } }
};
```

`afterResponse` 返回值：

```js
return { response: { ...event.response, plugin_tagged: true } };
```

`event.request` 包含 `id`、`model`、`stream`、`protocol`、`path`、`localKey` 与 `payload`；完成事件还包含 `usage.promptTokens/cachedTokens/completionTokens/totalTokens/costMicros/latencyMs`。

## 宿主 API

```js
host.log(level, message)              // info / warn / error / debug
await host.sleep(ms)

host.fs.readText(path)
host.fs.writeText(path, text)
host.fs.appendText(path, text)
host.fs.exists(path)
host.fs.list(path)
host.fs.remove(path)
host.fs.mkdir(path)

host.encoding.base64Encode(text)
host.encoding.base64Decode(text)
host.encoding.base64UrlEncode(text)
host.encoding.base64UrlDecode(text)
host.encoding.urlEncode(text)
host.encoding.urlDecode(text)
host.encoding.hexEncode(text)
host.encoding.hexDecode(text)

host.crypto.sha256(text)              // 十六进制
host.crypto.hmacSha256(secret, text)  // 十六进制
host.crypto.randomBytes(n)            // 十六进制
host.crypto.aesGcmEncrypt(keyHex, text)             // { nonce, ciphertext }
host.crypto.aesGcmDecrypt(keyHex, nonceHex, ciphertext)

await host.http.request({ url, method, headers, body, timeoutSecs })
// -> { status, headers, body }

await host.process.run({ command, args, cwd, env, timeoutSecs })
// -> { code, success, stdout, stderr }（默认关闭）

host.ui.toast(level, message)
await host.ui.confirm(message, title)          // -> boolean
await host.ui.prompt(message, default, title)  // -> string | null

host.app.version / host.app.endpoint / host.app.models
```

网络与进程调用由插件页的「插件权限」开关控制，默认允许网络、禁止进程调用。修改权限会自动重载正在运行的插件。

## 示例

`plugins/request-guard/main.js`：

```js
export const name = "Request Guard";

export function beforeRequest(event) {
  const text = JSON.stringify(event.request.payload?.messages ?? "");
  if (text.includes("block-me")) {
    return {
      block: true,
      status: 403,
      response: { error: { message: "请求被插件拦截" } },
    };
  }
  return null;
}
```

`plugins/hello-plugin/main.js`：

```js
export const name = "Hello Plugin";

export function onStartup() {
  host.log("info", "started");
}

export function onRequestCompleted(event) {
  host.fs.appendText("requests.jsonl", JSON.stringify(event.usage) + "\n");
}
```

## 限制

- Boa 是解释执行，插件调用适合低频事件与轻量逻辑；重计算应放在 Rust 侧或异步宿主 API 中。
- `beforeRequest` / `afterResponse` 每个插件最多等待 3 秒。
- 流式响应不会触发 `afterResponse`；`beforeRequest` 对流式和非流式都生效。
- 插件进程调用可以执行本机程序，只应启用可信插件。
