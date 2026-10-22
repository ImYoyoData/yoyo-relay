# Changelog

本项目遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。发布标签格式为 `vX.Y.Z`，发布说明由本文件对应版本的小节自动提取。

## [0.1.0] - 2026-10-22

### 新增

- 三层降级调度：同供应商账号轮询 → 不同供应商候选路由 → 可选的映射级兜底模型。
- 账号状态机：`Healthy` / `Cooling`（指数退避）/ `HalfOpen` / `Exhausted` / `Disabled`；并发满只临时跳过，不误判为故障。
- 支持 OpenAI Chat Completions、Anthropic Messages、OpenAI Responses 三套协议的请求与 SSE 流式事件互转，包含工具调用、图片和 usage 提取。
- 供应商、账号（含草稿 Key 预检）、路由（完整链路预览）、本地 Key 四类连接测试；测试请求不写入正式用量。
- SQLite 用量统计：按账号 / 供应商 / 全局 / 客户端模型四个维度聚合，支持交互式柱状/面积图、悬停提示与点击选中。
- 余额三态：自动 JSON 查询、手动录入（按用量估算递减）、不可用；低余额提醒。
- 本地 Key：`yr-` 前缀、模型白名单、独立 Token/请求额度、AES-256-GCM 加密存储。
- 账号 API Key 加密落盘；界面与日志脱敏；配置导出不包含任何密钥。
- 自定义无边框标题栏、浅色/深色主题、自动 DPI 缩放、图标按钮、遮罩 + 阴影弹窗、响应式窄窗口导航栏。
- 设置页按「通用与启动 / 代理 / 调度 / 安全 / 数据 / 关于」分区，新增开机自启、启动时最小化、关闭到托盘、任务栏实时请求信息。
- 启动参数：`--headless`、`--selftest`、`--page`、`--minimized`、`--autostart on|off`。
- 模拟上游联调工具 `tools/mock_upstream.py`，同时模拟 OpenAI 与 Anthropic（含流式与错误 Key）。

### 说明

- 系统托盘图标尚未实现，当前「关闭到托盘」表现为最小化到任务栏。
- 调试报文记录仅保留开关，尚未记录请求体与响应体。
- 图形界面在 OpenGL / eframe 下的空白窗口基线约 130 MiB，因此 GUI 空闲内存尚未达到 60 MiB 目标；仅代理的 `--headless` 模式实测约 11 MiB。
- macOS 构建未签名；UPX 压缩包仅提供 Windows x64 与 Linux x64。
