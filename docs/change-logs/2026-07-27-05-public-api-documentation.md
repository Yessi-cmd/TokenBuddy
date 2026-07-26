# 2026-07-27-05 全部公开接口补文档并锁定

## 目的

`missing_docs` 全仓库有 436 项待补。这些是跨 crate 与跨语言边界的公开词汇——`domain` 的类型
同时被存储层、四个适配器、Tauri 命令层和前端 TypeScript 使用，未文档化意味着每个使用者只能
去读调用点反推语义。

## 影响文件

按 crate 补齐并在各自 crate 根开启 `#![warn(missing_docs)]`：

| Crate | 补充项 |
|---|---|
| `tokenbuddy-domain` | 274（上一批次完成） |
| `tokenbuddy-core` | 65 |
| `tokenbuddy-storage` | 49 |
| `tokenbuddy-codex-session` | 20 |
| `tokenbuddy-claude-session` | 14 |
| `tokenbuddy-cockpit` | 13 |
| `tokenbuddy-cc-switch` | 12 |
| `tokenbuddy-sqlite-source` | 0（新建时即完备） |
| `tokenbuddy-desktop` | 4 |

## 写法取向

文档写的是**为什么**，不是重述签名：

- `apply_import_batch` 说明落库顺序为何是"来源 → Provider → 账号 → 归因/时间窗 → 事件 →
  cursor"，以及 cursor 放最后是为了让失败重读而非跳过。
- `StorageError::MigrationVersion` 说明为何拒绝而不是继续使用比自己新的库文件。
- `Core::start` 说明第一次导入为何是同步的（托盘一出现就该有真实数字），以及为何等到
  watcher 注册完才返回。
- `update_app_settings` 说明未在新设置中出现的来源会被清空而非沿用。
- 适配器的 `import_history_sync` 说明重复调用为何不产生事件。
- `LocalWebServer` 说明只绑定 loopback 的原因。

## 验证

- 全部六个库 crate + 桌面壳的 `missing_docs` 计数为 0。
- 各 crate 已开启 `#![warn(missing_docs)]`，配合 CI 的 `clippy -D warnings`，
  **新增未文档化的公开项会直接构建失败**。
- `cargo fmt --check`、`cargo clippy --workspace --all-targets --all-features -D warnings`、
  `cargo test --workspace --all-targets`（14 个套件）：通过。
- 前端 `prettier --check`、`eslint --max-warnings 0`、`vitest run`、`tsc -b`：通过。

## 遗留限制

- 三个集成测试文件的 crate 根仍有 `missing_docs` 提示。测试 crate 没有对外接口，未开启该
  lint，也未为其加 crate 级文档。
