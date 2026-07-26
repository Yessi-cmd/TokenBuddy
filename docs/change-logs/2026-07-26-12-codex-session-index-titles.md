# Codex 会话索引标题

## 目的

让共享会话列表和菜单栏面板显示 Codex App 中的真实会话名称，而不是在日志没有内嵌标题时统一显示 `Unavailable`。

## 受影响文件

- `crates/adapters/codex-session/src/lib.rs`
- `fixtures/codex/indexed_session.jsonl`
- `fixtures/codex/session_index.jsonl`

## 行为变化

- Codex 只读适配器读取 `~/.codex/session_index.jsonl` 的 `id` 与 `thread_name`，通过会话 ID 补充标题。
- 索引标题优先作为 Codex App 当前线程名称使用；索引缺失时保留日志内嵌标题，并且不会读取或保存 prompt、completion 或源码正文。
- 当历史日志游标已经位于文件末尾时，适配器会在索引存在时回读元数据并回填会话标题，不会重复产生 usage event。
- 索引本身也记录轻量游标；只有索引首次接入或内容变化时才回读历史元数据，普通轮询不会反复扫描全部日志。
- 索引文件不存在、含有坏行或没有有效标题时，usage 导入继续按原路径运行，缺失标题仍显示 `Unavailable`。

## 验证

- `cargo test -p tokenbuddy-codex-session`：7 项通过（包含升级回填和稳定索引游标场景）。
- `pnpm test`：前端 7 项、Rust workspace 全部测试通过（适配器 7、Core 6、Phase 4b 集成 1、桌面 12、Domain 3、Storage 3）。
- `pnpm lint`：ESLint 与 Clippy（`-D warnings`）通过。
- `pnpm check:rust`：通过。
- `pnpm format:check` 与 `git diff --check`：通过。
- `pnpm --filter @tokenbuddy/desktop tauri build --debug --no-bundle`：通过，生成 `target/debug/tokenbuddy-desktop`。

## 剩余限制

- 现有本机数据库需要应用启动后的正常刷新或手动“立即导入 Codex”才能回填已导入会话的标题。
- Codex 索引只提供线程名称；没有名称的线程仍保持 `Unavailable`，不会根据 prompt 内容猜测标题。
- Windows 真机托盘行为、隐藏窗口持续采集和 CPU/P95 验收仍属于原 Phase 4b 的未完成外部环境验证。
