# Contributing

YoyoRelay 使用两个长期分支：

| 分支 | 用途 |
|---|---|
| `dev` | 日常开发。请把功能、修复与文档提交到这里。 |
| `main` | 发布分支。只接受来自 `dev` 的发布合并。 |

## 本地开发

```bash
cargo run --release
cargo test
```

无界面联调：

```bash
python tools/mock_upstream.py 18080
YOYO_RELAY_HOME=/tmp/yoyo-relay-dev cargo run -- --selftest
```

## 提交变更

1. 从 `dev` 拉出功能分支（可选）：`git checkout -b feat/my-change origin/dev`。
2. 完成修改并运行 `cargo fmt`、`cargo test`。
3. 提交并推送到 `dev`（通过 PR 时目标分支选 `dev`）。
4. 不要在普通 PR 中修改 `Cargo.toml` 的版本号或 `CHANGELOG.md` 的发布小节。

## 发布

发布由维护者在用户明确要求时执行，详细步骤见 [AGENTS.md](AGENTS.md) 的「发布规则」：

1. 更新 `Cargo.toml` 的 `version` 与 `CHANGELOG.md` 对应小节。
2. 推送 `dev`。
3. 合并到 `main` 并推送，触发 `.github/workflows/release.yml`。

发布产物：

- Windows x64（`.zip`，含 UPX 压缩版）
- Linux x64（`.tar.gz`，含 UPX 压缩版）
- macOS arm64 / x64（`.tar.gz`，未签名）
