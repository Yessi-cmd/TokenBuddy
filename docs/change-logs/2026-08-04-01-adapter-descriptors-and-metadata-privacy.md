# Adapter descriptors and metadata privacy hardening

## Purpose

将调研得到的 descriptor-driven Adapter 设计和隐私默认值落地到现有 Core/SQLite 链路，确保
`save_request_metadata` 真正是显式 opt-in，同时为后续 OTel 等新来源提供集中式只读能力目录。

## Affected files

- `crates/domain/src/lib.rs`
- `crates/adapters/codex-session/src/lib.rs`
- `crates/adapters/claude-session/src/lib.rs`
- `crates/adapters/cc-switch/src/lib.rs`
- `crates/adapters/cockpit/src/lib.rs`
- `crates/core/src/lib.rs`
- `crates/core/tests/phase3_claude.rs`
- `crates/storage/src/lib.rs`
- `apps/desktop/src/features/settings/SettingsView.tsx`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`

## Behaviour changes

- 新增 `AdapterDescriptor` 和 `AdapterCapabilities`；四个现有 Adapter 声明 usage、Provider/account
  context、quota、file-watch 能力及只读边界，Core 维护统一 catalog。
- Core 生成数据源错误状态时使用 descriptor，避免 Adapter id、类型和展示名称在分支中重复维护。
- 新数据库和默认设置不再把 Adapter 内存中的脱敏 `raw_usage_json` 写入 SQLite。
- 用户显式打开“保存脱敏 usage 元数据”后，后续导入才会保存该字段；关闭设置会清除历史原始 usage
  元数据，但保留 normalized token、归因和精度事实。
- 设置页新增带删除语义说明的显式开关。

## Verification

- `cargo fmt --all -- --check`
- `pnpm format:check`
- `pnpm lint`（ESLint + `cargo clippy --workspace --all-targets --all-features -- -D warnings`）
- `pnpm test`（Vitest 54 tests + `cargo test --workspace --all-targets`）
- `pnpm build:web`
- `cargo check --workspace --all-targets`
- `git diff --check`

新增/更新测试覆盖默认不落盘、显式 opt-in、撤销设置清理历史元数据，以及 Adapter catalog 能力边界。

## Remaining limitations

- OTel Receiver、跨来源 request/response correlation 和可选本地 Proxy 不属于本批次的实现范围；其中
  Receiver 与 correlation 已在后续 `2026-08-04-02` 批次落地，Proxy 仍保持后续 Phase。
- Windows 托盘、隐藏窗口持续采集和路径选择器仍需 Windows 真机或 CI 运行时验收。
