# YoyoRelay 项目规则（供编码代理与维护者遵循）

这些规则优先级高于个人习惯。涉及分支、版本号或发布流程时，先读本文件。

## 分支模型

- `main`：**只用于发布**。只接受来自 `dev` 的发布合并，禁止日常提交。
- `dev`：**日常开发分支**。所有功能、修复、重构、文档和界面调整都在 `dev` 上完成并推送。
- 功能分支（可选）：从 `dev` 拉出 `feat/...` 或 `fix/...`，完成后合并回 `dev`；不要直接合并到 `main`。
- 禁止对 `main` 执行 force push；不要把 `dev` 的日常提交直接 cherry-pick 到 `main`。

## 发布规则（重要）

**只有用户明确要求“更新版本日志并发布 / 发新版 / 合并主分支发布”时，才允许发布。**
平时的任何修改、推送、测试都不得触发发布。

发布时严格按以下顺序：

1. 在 `dev` 上更新 `Cargo.toml` 的 `version`（SemVer）。
2. 在 `CHANGELOG.md` 顶部新增与版本号完全一致的二级标题小节，例如：
   ```markdown
   ## [0.2.0] - 2026-10-23
   ```
   发布说明会自动从该小节提取，因此必须写清楚新增、修复、变更与已知限制。
3. 运行 `cargo fmt --check && cargo test && cargo build --release`，确认通过。
4. 提交并推送 `dev`：`git push origin dev`。
5. 合并到 `main`，使用非快进合并并写清版本：
   ```bash
   git checkout main
   git pull --ff-only origin main
   git merge --no-ff dev -m "release: v0.2.0"
   git push origin main
   ```
6. 推送 `main` 后，GitHub Actions `Release` 自动读取 `Cargo.toml` 版本、检查标签是否已存在、提取 `CHANGELOG.md` 发布说明，并构建发布包。

规则约束：

- 版本号以 `Cargo.toml` 为唯一来源；标签格式固定为 `vX.Y.Z`。
- 如果同版本标签已存在，工作流会跳过发布，不会重复创建 Release。
- 发布合并提交信息中**不要**包含 `[skip release]`。
- `[skip release]` 仅用于仓库初始化或明确不希望发布的紧急 main 提交。
- 发布包：Windows x64、Linux x64、macOS arm64、macOS x64；Windows x64 与 Linux x64 额外提供 UPX 压缩版。macOS 不提供 UPX（会破坏签名且对 arm64 支持不可靠）。
- 每次发布后检查 Actions 结果与 Release 附件是否齐全；UPX 步骤失败时发布仍会继续，但必须记录并在修复后重新发版。

## 提交与推送

- 提交信息使用 `feat:` / `fix:` / `refactor:` / `docs:` / `chore:` / `release:` 前缀。
- 提交前至少运行 `cargo fmt` 与 `cargo test`；涉及协议、调度或网关时补跑 `--selftest`。
- 日常提交只推 `dev`。**不要**在没有用户明确发布指令时推 `main`。
- 不提交任何真实 API Key、`master.key`、`usage.db`、测试配置目录或 `.tmp-test/` 内容。

## 代码与安全约定

- 账号 Key 与本地 Key 只能以 AES-256-GCM 密文写入配置；任何日志、UI、错误信息都必须脱敏。
- 新增上游协议或转换逻辑时，同时补充单元测试与 `--selftest` 覆盖。
- 调度状态机、冷却退避、熔断阈值和额度窗口的行为变更必须配测试。
- UI 只消费 `runtime` 推送的 `UiSnapshot`，不要在 `update()` 中做 I/O 或重查询。
- 界面遵循现有设计语言：浅色工作台、卡片分层、图标按钮、遮罩弹窗；避免重新引入单汉字按钮。

## 远程仓库

`origin` = `https://github.com/ImYoyoData/yoyo-relay.git`
