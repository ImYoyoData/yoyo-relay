# Changelog

本项目遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。发布标签格式为 `vX.Y.Z`，发布说明由本文件对应版本的小节自动提取。

## [0.1.0] - 2026-10-22

首个公开版本。

### 新增

- **三层降级调度**：同供应商账号轮询 → 不同供应商候选路由 → 可选的映射级兜底模型。
- **账号状态机**：`Healthy` / `Cooling`（指数退避，60 秒起、30 分钟封顶）/ `HalfOpen` / `Exhausted` / `Disabled`；并发满只临时跳过，不误判为故障。
- **协议转换**：OpenAI Chat Completions、Anthropic Messages、OpenAI Responses 三套协议的请求与 SSE 流式事件互转，包含工具调用、图片与 usage 提取；另支持原样透传。
- **四类连接测试**：供应商、账号（含编辑中的草稿 Key 预检）、路由（完整链路预览与降级演练）、本地 Key（含白名单拒绝校验）；测试请求不写入正式用量。
- **用量统计**：SQLite 记录请求、Token、延迟、花费；按账号 / 供应商 / 全局 / 客户端模型四维聚合；可交互柱状/面积图，支持切换指标、悬停提示与点击固定详情。
- **余额三态**：自动 JSON 路径查询、手动录入（按用量估算递减）、不可用；支持低余额提醒与刷新间隔配置。
- **本地 Key**：`yr-` 前缀、模型白名单、独立 Token/请求额度、AES-256-GCM 加密存储。
- **安全**：账号 API Key 与本地 Key 加密落盘；界面与日志脱敏；本地端点 Bearer 鉴权；配置导出不包含任何密钥。
- **界面**：无边框自定义标题栏、浅色/深色主题、自动 DPI 缩放与手动缩放、图标按钮、遮罩 + 阴影弹窗、响应式窄窗口图标导航。
- **设置页**：按「通用与启动 / 代理 / 调度与熔断 / 安全 / 数据 / 关于」六个分区组织，修改即保存。
- **启动、托盘与单实例**：开机自启（Windows 注册表 / macOS LaunchAgent / Linux XDG autostart）；启动后直接驻留系统托盘；关闭窗口隐藏到托盘（Windows 上窗口与任务栏按钮一起隐藏，代理继续运行）；托盘使用应用 Logo，左键恢复主窗口，右键菜单可显示主窗口或退出；重复启动程序会唤出已有窗口并置于前台，不会报错或启动第二个实例。
- **任务栏信息**：窗口标题显示活跃请求、今日 Token 与代理状态。
- **命令行**：`--headless`、`--selftest`、`--page <页面>`、`--minimized`、`--autostart on|off`。
- **发布流水线**：`main` 分支自动读取版本号、提取本文件对应小节作为发布说明，并构建 Windows x64、Linux x64、macOS arm64、macOS x64 发布包；Windows 与 Linux 额外提供 UPX 压缩版。
- **联调工具**：`tools/mock_upstream.py` 同时模拟 OpenAI 与 Anthropic（含流式与错误 Key）。

### 说明

- Windows 11 默认把新托盘图标收进「^」溢出菜单，可在系统设置中拖到任务栏常显。
- macOS / Linux 的关闭按钮目前回退为「最小化到任务栏」；托盘图标与「退出」菜单可用，原生隐藏/恢复待后续版本完善。
- `--headless` 与图形界面共用单实例锁；headless 运行时再次启动程序会直接退出，不会启动第二个代理。
- 调试报文记录仅保留开关，尚未记录请求体与响应体。
- 图形界面在 OpenGL / eframe 下的空白窗口基线约 130 MiB，GUI 空闲内存尚未达到 60 MiB 目标；仅代理的 `--headless` 模式实测约 11 MiB。
- macOS 构建未签名；UPX 压缩包仅提供 Windows x64 与 Linux x64（macOS 上 UPX 会破坏签名且 arm64 支持不可靠）。
- Linux 托盘依赖桌面状态栏宿主；GNOME 需要 AppIndicator/KStatusNotifierItem 扩展才能显示图标。
