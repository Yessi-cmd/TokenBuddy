# Implement the Tray-first minimal loop

## Purpose

把现有显式 Codex 扫描收进单实例后台 `tokenbuddy-core`，先验证
Core 持续增量采集和 `QuickSummary`，再提供共享的托盘 / 菜单栏轻量入口、
隐藏启动的完整窗口和按需 loopback Web API。此批不扩展 Claude、OTel、
CC Switch、Cockpit 或官方额度数据源。

## Affected files

- `Cargo.toml`、`Cargo.lock`
- `crates/domain/src/lib.rs`
- `crates/storage/src/lib.rs`
- `crates/core/Cargo.toml`、`crates/core/src/lib.rs`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/src/lib.rs`
- `apps/desktop/src-tauri/src/web.rs`
- `apps/desktop/src-tauri/tauri.conf.json`
- `apps/desktop/src-tauri/capabilities/default.json`
- `apps/desktop/src/App.tsx`、`apps/desktop/src/App.test.tsx`
- `apps/desktop/src/lib/api.ts`、`apps/desktop/src/styles.css`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`

## Behaviour changes

- Tauri 启动时创建唯一 `Arc<Core>`，Core 持有 SQLite 查询 / 写入边界，先导入一次 Codex Session，随后由单个后台 worker 按 cursor 增量轮询；重复导入保持幂等。
- 新增 `QuickSummary`、`CollectionStatus` 和官方额度不可用语义。轻量窗口只查询摘要，不扫描 JSONL、不加载历史会话、不执行复杂聚合；未知值仍返回 `None` / `Unavailable`。
- macOS 和 Windows 共用 Tauri tray 入口：单击打开轻量摘要，双击打开完整面板，菜单提供导入、本地网页和退出；完整窗口启动隐藏，关闭时隐藏而不会停止 Core。
- 新增隐藏的 `/quick` React 入口，以及桌面 IPC / loopback HTTP 共用的 API 传输层。按需 Web API 显式绑定 `127.0.0.1`，不直接暴露 SQLite。
- 本地 Web API 提供 QuickSummary、今日摘要、会话、事件、数据源、Codex 检测 / 重扫和静态 SPA fallback 的最小查询闭环。

## Verification performed

- `pnpm --filter @tokenbuddy/desktop format:check`
- `pnpm --filter @tokenbuddy/desktop lint`
- `pnpm --filter @tokenbuddy/desktop test` — 2 frontend tests passed
- `pnpm --filter @tokenbuddy/desktop build`
- `cargo fmt --all`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-targets` — 17 Rust tests passed
- `cargo check --workspace --all-targets`
- `pnpm --filter @tokenbuddy/desktop tauri build --debug --no-bundle`
- 新增 Core 后台追加 JSONL、QuickSummary 更新和重复 shutdown 测试。
- 新增 loopback API 测试，确认 `/api/quick-summary` 从 Core 返回数据。

## Remaining limitations

- 当前持续采集使用低频轮询；原生 `notify` 文件事件、macOS / Windows 真机交互和性能指标尚未验证。
- 当前环境只安装了 `aarch64-apple-darwin` Rust target，Windows target / GitHub Actions 尚未执行；本批只完成了跨平台 Tauri 代码路径和本机 macOS 构建。
- 共享 SPA 目前覆盖完整 Dashboard 与 `/quick` 最小入口，Provider、额度、设置等完整 Phase 4b 路由仍待补齐。
- 官方额度仍保持 `Unavailable`，Claude Session、OTel、CC Switch、Cockpit 和本地代理未实现。
