
每个插件是一个 ES Module，运行在独立线程与独立 JavaScript Context 中。插件之间不共享状态；卸载时 Context 被销毁，Boa GC 回收全部对象。

## 目录与清单

```
<配置目录>/plugins/
  hello-plugin/
    plugin.json      # 可选
    main.js          # "main" 指向的入口
  single.js          # 或单个 .js 文件（同目录相对 import 可用）
```

plugin.json：

```json
{
  "name": "Hello Plugin",
  "version": "1.0.0",
  "description": "示例",
  "main": "main.js",
  "author": "you",
  "events": ["onStartup", "beforeRequest"]
}
```

模块可以导出 `name`、`version`、`description`、`events` 以及任意事件函数。`host.fs` 的根目录是 `<配置目录>/plugin-data/<插件目录名>/`，只允许该目录内的相对路径。

## 事件通用约定

所有事件处理函数的参数都是一个对象，至少包含 `event` 字段（事件名）。返回值规则见各事件说明。事件按插件列表顺序串行执行；单个插件最多等待 3 秒，超时会被跳过。

### onStartup(event)

模块求值完成后调用，返回值忽略。

```js
{
  event: "onStartup",
  plugin: { id: "hello-plugin", name: "Hello Plugin", version: "1.0.0", dir: "..." }
}
```

### onEnable(event) / onDisable(event) / onShutdown(event) / onReload(event)

启用、禁用、卸载、重载时调用，返回值忽略。

```js
{ event: "onEnable", plugin: { id, name, version, dir } }
```

### onProxyStart(event)

代理开始监听后调用。

```js
{ event: "onProxyStart", endpoint: "http://127.0.0.1:7789/v1", bind: "127.0.0.1", port: 7789 }
```

### onProxyStop(event)

代理停止后调用。

```js
{ event: "onProxyStop" }
```

### onConfigChanged(event)

配置保存后调用。只包含数量，不含任何密钥。

```js
{ event: "onConfigChanged", providers: 2, accounts: 3, routes: 4, models: 2 }
```

### beforeRequest(event)

请求鉴权与模型白名单校验之后、调度之前调用。**可以修改请求体或直接拦截请求。**

```js
{
  event: "beforeRequest",
  request: {
    id: "request-uuid",
    model: "gpt-4o",                 // 客户端模型名
    stream: true,                    // 客户端是否要求流式
    protocol: "openai-chat",         // openai-chat / anthropic-messages / openai-responses
    path: "/v1/chat/completions",
    localKey: { id: "key-id", name: "Claude Code" } | null,
    payload: { ... }                 // 完整客户端请求体
  }
}
```

返回值（按顺序判断）：

| 返回 | 作用 |
|:---|:---|
| `null` / `undefined` | 不修改，继续 |
| `{ payload: {...} }` | 用新的请求体替换（可改 model、messages、stream 等） |
| `{ block: true, status: 403, response: {...} }` | 直接拦截，`response` 作为 JSON 返回 |
| 直接返回带 `model` 或 `messages` 的对象 | 视为新的请求体 |

修改示例：

```js
export function beforeRequest(event) {
  if (event.request.model === "fast") {
    return { payload: { ...event.request.payload, model: "gpt-4o-mini" } };
  }
  return null;
}
```

拦截示例：

```js
export function beforeRequest(event) {
  const text = JSON.stringify(event.request.payload?.messages ?? "");
  if (text.includes("block-me")) {
    return {
      block: true,
      status: 403,
      response: { error: { message: "blocked by plugin", type: "plugin_blocked" } }
    };
  }
  return null;
}
```

### afterResponse(event)

非流式响应返回给客户端之前调用。流式响应不会触发。

```js
{
  event: "afterResponse",
  request: { id, model, protocol, path, localKey, payload },
  response: { ... }   // 已转换为客户端协议的响应对象
}
```

返回 `{ response: {...} }` 替换响应；返回带 `choices` 或 `output` 的对象也会被视为替换响应；`null` 表示不修改。

```js
export function afterResponse(event) {
  return { response: { ...event.response, plugin_tagged: true } };
}
```

### onRouteSelected(event)

每次向上游发起尝试前通知（只读，不等待结果）。

```js
{
  event: "onRouteSelected",
  model: "gpt-4o",
  account: "账号名",
  accountId: "account-uuid",
  provider: "供应商名",
  providerId: "provider-uuid",
  upstreamModel: "gpt-4o-2024",
  protocol: "openai-chat"
}
```

### onRequestCompleted(event)

请求写入用量数据库后通知（只读）。

```js
{
  event: "onRequestCompleted",
  request: {
    id: "request-uuid",
    model: "gpt-4o",
    upstreamModel: "gpt-4o-2024",
    account: "账号名",
    provider: "供应商名",
    protocol: "openai-chat",
    status: 200,
    ok: true,
    stream: true
  },
  usage: {
    promptTokens: 1200,       // 输入 token（含缓存命中）
    cachedTokens: 800,        // 命中缓存的输入 token
    completionTokens: 300,    // 输出 token
    totalTokens: 1500,
    costMicros: 12345,        // 微美元
    latencyMs: 850
  }
}
```

### onAccountFailure(event)

账号尝试失败时通知（只读）。

```js
{
  event: "onAccountFailure",
  account: "账号名",
  accountId: "account-uuid",
  provider: "供应商名",
  providerId: "provider-uuid",
  model: "gpt-4o",
  upstreamModel: "gpt-4o-2024",
  status: 429,
  kind: "rate_limited",   // auth / rate_limited / server ...
  message: "上游错误摘要"
}
```

### onError(event)

所有上游尝试都失败后通知。

```js
{ event: "onError", model: "gpt-4o", status: 502, attempts: 4, message: "失败原因" }
```

### onBalanceUpdated(event)

余额刷新完成后通知。

```js
{
  event: "onBalanceUpdated",
  count: 2,
  results: [
    { accountId: "account-uuid", ok: true, error: null },
    { accountId: "other-id", ok: false, error: "余额接口 HTTP 401" }
  ]
}
```

### selfTest(event)

插件页「自检」按钮调用，返回任意 JSON，显示在通知与日志中。适合验证配置或连通性。

```js
export async function selfTest() {
  return { ok: true, now: host.time.nowIso() };
}
```

## 虚拟供应商（插件提供模型）

插件可以导出 `providers`，YoyoRelay 会自动为每个供应商创建**临时供应商 + 虚拟账号 + 路由**，并把模型加入 `/v1/models`。插件卸载、禁用或加载失败时，这些临时条目会自动移除，不会写入用户配置。

```js
export const providers = [
  {
    // 显示名称
    name: "My Bridge",
    description: "把本地模型桥接进 YoyoRelay",
    // 插件接收/返回的协议：openai-chat | anthropic-messages | openai-responses
    protocol: "openai-chat",
    // 这些名称同时作为客户端模型名与上游模型名
    models: ["my-model", "my-model-mini"],
    // 路由优先级，数值越大越优先
    priority: 50,
    // 是否支持流式；false 时即使客户端要求 stream 也按非流式返回
    streaming: true
  }
];
```

### onProviderRequest(event)

当请求被调度到插件供应商时调用。`event.request.payload` 已经转换为声明的 `protocol` 方言。

```js
{
  event: "onProviderRequest",
  provider: { id, name, model: "my-model", streaming: true },
  request: {
    payload: { ... },        // 协议对应的完整上游请求体
    model: "my-model",       // 上游模型名
    clientModel: "my-model", // 客户端模型名
    stream: true,            // 客户端是否要求流式
    protocol: "openai-chat"
  }
}
```

**非流式**：返回 `{ status?, body }`，`body` 必须是该协议的非流式响应对象：

```js
export async function onProviderRequest(event) {
  const res = await host.http.request({
    url: "https://my-backend.example.com/v1/chat/completions",
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: event.request.payload,
    timeoutSecs: 120
  });
  return { status: res.status, body: JSON.parse(res.body) };
}
```

**流式**：通过 `host.upstream` 逐帧发给 YoyoRelay，必须使用声明的协议 SSE 格式，并在最后包含 usage（用于 token 与费用统计）：

```js
export async function onProviderRequest(event) {
  await host.http.sse(
    {
      url: "https://my-backend.example.com/v1/chat/completions",
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "Accept": "text/event-stream"
      },
      body: event.request.payload,
      timeoutSecs: 300
    },
    (sse) => {
      // 原样转发上游 SSE 帧
      host.upstream.emitRaw(sse.raw);
    }
  );
  // 函数返回后流自动结束
}
```

`host.upstream`（只在 `onProviderRequest` 流式调用中可用）：

| 方法 | 参数 | 说明 |
|:---|:---|:---|
| `emitRaw(frame)` | string | 发送原始 SSE 帧（可一次发送多帧） |
| `emitJson(value)` | any JSON | 自动包装为 `data: <json>\n\n` |
| `error(message)` | string | 以错误终止上游流 |
| `finish()` | — | 可选；函数返回时自动结束 |

流式响应会经过 YoyoRelay 的协议转换器，因此插件只需输出自己声明的协议；客户端使用其他协议时会自动转换。虚拟账号的并发不受限制，但插件线程串行执行，多个请求会排队。

## 宿主 API

所有 API 都在全局 `host` 对象下。异步 API 返回 Promise，可直接 `await`。

### host.log(level, message)

写入插件运行日志（插件页可查看）。

| 参数 | 类型 | 说明 |
|:---|:---|:---|
| level | string | `info` / `warn` / `error` / `debug` |
| message | string | 日志内容 |

### await host.sleep(ms)

等待指定毫秒，返回 `undefined`。

### host.fs

路径必须是数据目录内的相对路径，不能包含 `..` 或绝对路径。

| 方法 | 参数 | 返回 | 说明 |
|:---|:---|:---|:---|
| `readText(path)` | path: string | string | 读取 UTF-8 文本，文件不存在时抛错 |
| `writeText(path, text)` | path, text: string | undefined | 覆盖写入，自动创建父目录 |
| `appendText(path, text)` | path, text: string | undefined | 追加写入 |
| `readBase64(path)` | path: string | string | 读取二进制并返回 Base64 |
| `writeBase64(path, b64)` | path, b64: string | undefined | 从 Base64 写入二进制 |
| `exists(path)` | path: string | boolean | 文件或目录是否存在 |
| `list(path)` | path: string | string[] | 目录项名称，路径不存在时返回空数组 |
| `remove(path)` | path: string | undefined | 删除文件或目录（目录递归删除） |
| `mkdir(path)` | path: string | undefined | 递归创建目录 |

```js
host.fs.writeText("logs/today.txt", "hello\n");
const text = host.fs.readText("logs/today.txt");
```

### host.encoding

| 方法 | 参数 | 返回 |
|:---|:---|:---|
| `base64Encode(text)` / `base64Decode(text)` | string | string |
| `base64UrlEncode(text)` / `base64UrlDecode(text)` | string | string（无填充 URL 安全） |
| `urlEncode(text)` / `urlDecode(text)` | string | string |
| `hexEncode(text)` / `hexDecode(text)` | string | string |

### host.crypto

| 方法 | 参数 | 返回 |
|:---|:---|:---|
| `sha256(text)` | string | 64 位十六进制 |
| `hmacSha256(secret, message)` | string, string | 64 位十六进制 |
| `randomBytes(n)` | number (1–4096) | 十六进制字符串 |
| `aesGcmEncrypt(keyHex, plaintext)` | 64 位十六进制密钥, string | `{ nonce, ciphertext }` |
| `aesGcmDecrypt(keyHex, nonceHex, ciphertext)` | string | 原文 |
| `constantTimeEqual(a, b)` | string, string | boolean |

`nonce` 为 24 位十六进制（12 字节），`ciphertext` 为 Base64。

### await host.http.request(options)

发起 HTTP 请求。网络权限关闭时会抛错；`localhost` / 回环地址自动绕过系统代理。

| 字段 | 类型 | 必填 | 说明 |
|:---|:---|:---|:---|
| url | string | 是 | 仅 `http://` 或 `https://` |
| method | string | 否 | 默认 `GET` |
| headers | object | 否 | 字符串键值 |
| body | string / object | 否 | 字符串按原文发送，对象按 JSON 发送 |
| timeoutSecs | number | 否 | 1–120，默认 30 |

返回：

```js
{ status: 200, headers: { "content-type": "application/json" }, body: "..." }
```

```js
const res = await host.http.request({
  url: "https://api.example.com/v1/status",
  headers: { "Authorization": "Bearer xxx" },
  timeoutSecs: 10,
});
const data = JSON.parse(res.body);
```

### await host.http.download(url, path)

下载文件到数据目录中的相对路径。返回 `{ status, bytes }`。

### await host.http.sse(options, onEvent)

读取 SSE 流，并为每个事件调用同步回调。回调参数：

```js
{
  event: "content_block_delta" | null, // 上游 event: 字段
  data: "...",                         // 原始 data 内容
  raw: "event: ...\ndata: ...\n\n"     // 可原样转发给 host.upstream.emitRaw
}
```

`options` 与 `host.http.request` 相同，另外默认 `Accept: text/event-stream`。返回 `{ status, events }`。网络权限关闭时抛错；回调抛错会终止读取。回调必须是同步函数，不要在回调里 `await`。

```js
await host.http.sse(
  { url: "https://example.com/stream", headers: { Accept: "text/event-stream" } },
  (sse) => {
    host.upstream.emitRaw(sse.raw);
  }
);
```

### await host.process.run(options)

执行本机程序。默认关闭，需要在插件页开启「允许进程调用」。

| 字段 | 类型 | 必填 | 说明 |
|:---|:---|:---|:---|
| command | string | 是 | 可执行文件 |
| args | string[] | 否 | 参数数组 |
| cwd | string | 否 | 工作目录 |
| env | object | 否 | 追加环境变量 |
| timeoutSecs | number | 否 | 1–300，默认 30；超时会终止进程 |

返回：

```js
{ code: 0, success: true, stdout: "hi\n", stderr: "" }
```

### host.ui

| 方法 | 参数 | 返回 | 说明 |
|:---|:---|:---|:---|
| `toast(level, message)` | string, string | undefined | 右上角通知，level 支持 info/warn/error/success |
| `await confirm(message, title?)` | string, string | boolean | 确认弹窗 |
| `await prompt(message, defaultValue?, title?)` | string, string, string | string \| null | 输入弹窗，取消返回 null |
| `await alert(message, title?)` | string, string | undefined | 单按钮提示弹窗 |

### host.storage

插件私有键值存储（`plugin-data/<id>/storage.json`），值可以是任意 JSON。

| 方法 | 参数 | 返回 |
|:---|:---|:---|
| `get(key)` | string | 值或 `null` |
| `set(key, value)` | string, any JSON | undefined |
| `remove(key)` | string | boolean（是否存在并删除） |
| `keys()` | — | string[] |

### host.time / host.util

| 方法 | 返回 |
|:---|:---|
| `host.time.now()` | 毫秒时间戳 |
| `host.time.nowIso()` | 本地时区 RFC3339 字符串 |
| `host.util.uuid()` | UUID v4 |

### host.app

| 属性 | 类型 | 说明 |
|:---|:---|:---|
| `version` | string | 当前 YoyoRelay 版本 |
| `endpoint` | string | 本机 API Base URL |
| `models` | string[] | 当前可服务的客户端模型 |

## 虚拟供应商模型刷新

插件可以额外导出 `onListModels()`，在「供应商」页临时供应商卡片上点击「刷新模型」时调用，返回模型数组、供应商数组或 `{ providers: [...] }`。刷新后会同步临时供应商、虚拟账号、自动路由与 `/v1/models`。

```js
export async function onListModels() {
  const res = await host.http.request({ url: "https://backend.example.com/v1/models" });
  const data = JSON.parse(res.body);
  return {
    providers: [
      { name: "My Bridge", protocol: "openai-chat", models: data.data.map((m) => m.id), streaming: true }
    ]
  };
}
```

供应商页会显示插件注入的「临时供应商」，路由编辑器可以直接选用它；临时供应商、虚拟账号和自动路由不会写入用户配置。

## WebSocket 与 SSE

- `host.net.sse(options, onEvent)` 与 `host.http.sse` 相同，读取 SSE 客户端流。
- `host.ws`（或 `host.net.ws`）提供 WebSocket 客户端与服务端。

客户端：

```js
const conn = await host.ws.connect({ url: "wss://example.com/socket", headers: { Authorization: "Bearer ..." } });
host.ws.send(conn.id, "hello");
const frame = await host.ws.recv(conn.id, 30); // { type: "text"|"binary"|"close"|"timeout", ... }
host.ws.close(conn.id);
```

服务端：

```js
export async function start() {
  return await host.ws.serve({ host: "127.0.0.1", port: 0, path: "/ws" });
}
export function onWebSocketConnection(event) { host.ws.send(event.connectionId, "welcome"); }
export function onWebSocketMessage(event) {
  if (event.type === "text") host.ws.send(event.connectionId, "echo:" + event.text);
}
export function onWebSocketClosed(event) { /* event.code / event.reason */ }
```

所有网络调用都受「允许网络访问」开关控制；`serve` 默认只监听 `127.0.0.1`，监听 `0.0.0.0` 时请自行鉴权。插件停止或卸载时会自动关闭全部连接与服务端。

## 卸载与数据

插件页的垃圾桶按钮是真正的卸载：停止插件并删除插件目录或单文件；确认弹窗可勾选是否保留 `plugin-data/<id>`。取消勾选时数据目录一并删除。

## 运行限制

- 每个插件独立线程与 Context，事件串行执行。
- 循环约 2000 万次、递归约 2000 层后抛出 JS 错误。
- `beforeRequest` / `afterResponse` 单个插件最多等待 3 秒。
- 流式响应不会触发 `afterResponse`；`beforeRequest` 对流式和非流式都生效。
- 进程调用可以执行本机程序，只应对可信插件开启。
- 插件访问网络、进程与界面弹窗均可在插件页查看来源与日志。
