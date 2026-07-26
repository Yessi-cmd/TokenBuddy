# 嵌套会话标题与菜单栏面板收起

## 目的

修复真实 Codex App 日志使用嵌套 `payload` schema 导致会话标题仍为 `Unavailable`，以及 quick panel 失去焦点后继续置顶的问题。

## 受影响文件

- `crates/adapters/codex-session/src/lib.rs`
- `fixtures/codex/rollout-indexed-session.jsonl`
- `apps/desktop/src-tauri/src/lib.rs`

## 行为变化

- 适配器现在读取 `payload.session_id`、`payload.id`、`payload.thread_id` 等真实 Codex 字段，并读取嵌套的路径、模型、请求和时间元数据。
- 会话索引标题通过真实嵌套 ID 关联；对已经导入过的日志保留文件名外部 ID，避免重复 usage event 和会话分裂。
- macOS/Windows quick panel 在失去焦点后延迟收起；菜单栏再次点击会取消待收起任务并可靠切换显示状态。
- 关闭窗口仍只隐藏窗口，不会停止 Core 采集；退出菜单仍执行完整退出。

## 验证

- `cargo test -p tokenbuddy-codex-session`：8 项通过。
- `cargo test -p tokenbuddy-desktop --lib`：13 项通过。
- `pnpm --filter @tokenbuddy/desktop test`：7 项通过。
- `pnpm test`：前端与 Rust workspace 全量通过。
- `pnpm lint`、`pnpm check:rust`、`pnpm format:check`、`git diff --check`：通过。
- 新 debug 二进制实际启动并刷新本机应用数据库：有标题会话从 1 条增至 98 条，9,293 条 usage event 已关联到有标题会话，索引游标已写入。
- `pnpm --filter @tokenbuddy/desktop tauri build --debug`：通过，生成 debug `.app` 和 `.dmg`。

## 剩余限制

- 本批次未自动删除数据库中已有的脱敏 `Fixture session`，避免未经确认删除用户本地数据；它不是本次真实 Codex 标题回填的来源。
- Computer Use 本轮读取 macOS UI 时发生超时，因此失焦收起已由 Tauri 事件单测和实际构建验证，仍需用户在当前桌面解锁后手动点验一次。
- Windows 真机托盘交互仍需 Windows 环境补验。
