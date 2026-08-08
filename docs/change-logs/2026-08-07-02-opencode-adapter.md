# 2026-08-07-02 OpenCode 只读适配

## 目的

项目规范 §1.3 将 OpenCode 列入"后续支持"。本批把 OpenCode 提升为兼容支持：TokenBuddy 现在可以
只读读取 OpenCode 的 SQLite 会话数据库，把每次模型调用的 token 用量导入统一统计模型。

## 改动文件

- `crates/domain/src/lib.rs`：`AppKind` 新增 `OpenCode`（持久化值 `open_code`）；`AppSettings`
  新增 `opencode_db_path`。
- `crates/adapters/opencode/`（新 crate `tokenbuddy-opencode`）：
  - `session` 表 → 会话（标题、项目目录、模型、父会话链）；
  - `part` 表 `type = "step-finish"` 行 → 每个模型调用一个 usage 事件；
  - 增量 cursor 为 `part.time_created` 高位水位（同毫秒桶重读，靠稳定 part-id 哈希幂等）；
  - 事件哈希 = `SHA256(source_id, part_id)`；
  - `default_opencode_db()`：macOS/Linux `~/.local/share/opencode/opencode.db`，Windows
    `%LOCALAPPDATA%\opencode\opencode.db`。
- `crates/adapters/sqlite-source/`：未改动（复用 `open_read_only`、`table_exists`、`column_names`
  等）。
- `Cargo.toml`、`crates/core/Cargo.toml`、`apps/desktop/src-tauri/Cargo.toml`：注册新 crate。
- `crates/storage/`：新增 migration `0009_opencode_db_path`；`app_from_str` 支持 `open_code`；
  `provider_family` 对 OpenCode 事件默认派生 `unknown/Unknown`（模型名绝不暗示真实上游）。
- `crates/core/src/lib.rs`：`CoreConfig.opencode_db`、`set/get/detect/rescan_opencode`、
  `update_app_settings` 接线、worker 增量导入、`ADAPTER_DESCRIPTORS` 增加 OpenCode，
  `record_source_error` 识别 sqlite 来源。
- `crates/otel-receiver/src/lib.rs`：`stable_session_id` 支持 OpenCode 命名空间。
- `apps/desktop/src-tauri/src/lib.rs`：Tauri commands `detect_opencode_path` / `rescan_opencode`、
  默认路径接线、settings 测试。
- `apps/desktop/src-tauri/src/web.rs`：loopback `GET /api/detect-opencode`、`POST /api/rescan-opencode`。
- `apps/desktop/src/lib/api.ts`：`AppKind` 增加 `open_code`、`AppSettings.opencode_db_path`、
  `detectOpenCodePath` / `rescanOpenCode`。
- `apps/desktop/src/features/settings/SettingsView.tsx`：OpenCode 数据库路径字段（原生文件选择器）。
- `apps/desktop/src/features/dashboard/DashboardView.tsx`、`src/lib/filters.ts`：应用筛选增加
  OpenCode。
- `apps/desktop/src/App.test.tsx`、`src/lib/api.test.ts`：settings 形状与 rescan/detect 契约测试。
- `crates/core/tests/phase8_opencode.rs`：Core 集成测试（导入、幂等、删库降级、异常 Schema）。
- 规范与文档：`AI_Coding_Token_Observatory_PROJECT_SPEC.md`（§1.3、§7、§19.5、§23.2、§26、
  实施状态）、`README.md`、本变更日志。

## 行为变化

- OpenCode 会话与请求级 Token 出现在 Dashboard、会话列表/详情、QuickSummary 与导出中，
  应用筛选可只选 OpenCode。
- 精度：Token/会话 `ExactSession`（请求记录与归属均来自 OpenCode 自身），Provider/账号
  `Unavailable`（`providerID` 只描述配置的 Provider 插件，不构成真实上游证据）。
- `cost` 来自 OpenCode 自带模型价格表的计算，记入 `estimated_cost`，绝不冒充 Provider 实报费用。
- 归一化按 Anthropic 式分离语义：`input` = 未缓存输入，缓存读/写独立字段；
  `tokens.total` 仅是各字段之和，不单独存储。
- OpenCode 数据库缺失/被删除时来源显示 `not_found`，Schema 不支持时显示 `error`，都不影响
  Codex/Claude 等其他来源。

## 验证

- 真实数据验证：本机 OpenCode 数据库中，会话累计计数器与全部 step-finish 请求之和逐项相等
  （input 107604 / output 6710 / reasoning 16812 / cache_read 4277760），确认无累计快照重复计数。
- `cargo test --workspace --all-targets`：全部通过（含新增 8 项适配器单测、5 项 Core 集成测试）。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`：通过。
- `cargo fmt --all -- --check`：通过。
- 前端 `pnpm format:check`、`eslint --max-warnings 0`、`vitest run`（55 项）、生产构建：通过。

## 限制

- 依赖 OpenCode `session` / `message` / `part` 三张表；OpenCode 大版本迁移后 schema 变化需更新
  解析器（按 §23.4 规则新增 fixture）。
- `file_watch` 未启用：OpenCode 更新靠 Core 的 30 秒轮询兜底。
- 会话级模型取自 `session.model`；消息 `data.model` 提供时按消息逐请求覆盖（支持会话中途换模型），
  均缺失时保持 `Unavailable`。
- Windows `%LOCALAPPDATA%` 默认路径与只读打开行为仍需 Windows 真机验收。
