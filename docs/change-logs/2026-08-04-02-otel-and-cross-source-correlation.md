# OTLP receiver and cross-source correlation

## Purpose

将调研得到的 OpenTelemetry 语义、轻量本地接收器、来源优先级和跨 Adapter 去重设计落到
TokenBuddy Core，同时保持 OTel 可选、回环监听和隐私默认值。

## Affected files

- `Cargo.toml`
- `Cargo.lock`
- `crates/otel-receiver/Cargo.toml`
- `crates/otel-receiver/src/lib.rs`
- `crates/domain/src/lib.rs`
- `crates/storage/src/lib.rs`
- `crates/core/Cargo.toml`
- `crates/core/src/lib.rs`
- `crates/core/tests/phase5_otel.rs`
- `apps/desktop/src/features/settings/SettingsView.tsx`
- `apps/desktop/src/features/dashboard/DashboardView.tsx`
- `apps/desktop/src/lib/api.ts`
- `apps/desktop/src/App.test.tsx`
- `apps/desktop/src/styles.css`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`

## Behaviour changes

- 新增 `tokenbuddy-otel-receiver`：只绑定 `127.0.0.1`，接收 OTLP/HTTP `/v1/traces`，支持
  protobuf 与 JSON payload；不依赖 Collector、Prometheus、Loki、Docker 或远程服务。
- 只提取规范化 token、request/response/session/model/provider 等必要元数据；未知 OTel 属性、
  Prompt、Completion 和源代码不会进入 raw usage。`save_request_metadata` 仍默认关闭。
- OTel span 经过 Core 的同一 import lock、SQLite transaction、QuickSummary 和查询服务；也会
  创建无正文的会话元数据，使 OTel-only session 出现在会话列表中。
- OTel 端口留空时完全关闭；端口冲突只产生 warning，不阻塞文件 Adapter、桌面启动或设置保存。
- 新增 `correlation_key`、来源 precedence 和 precision precedence。同一 request/response identity
  的跨来源观察只保留更高可信度事实；例如 OTel Verified 会替换 Session ExactSession，并报告
  `reconciled_events`，重复导入仍保持幂等。
- Dashboard 重扫结果显示“校正”数量，Adapter catalog 纳入 OTel 的能力和非外部只读边界。

## Verification

- `cargo fmt --all -- --check`
- `pnpm format:check`
- `pnpm lint`
- `pnpm test`（Vitest 54 tests + workspace Rust tests）
- `pnpm build:web`
- `cargo check --workspace --all-targets`
- `git diff --check`

新增测试覆盖 OTLP protobuf/JSON 解析、回环 HTTP 投递、敏感属性过滤、Core 集成导入、会话落库、
跨来源强事实替换和重复导入幂等。

## Remaining limitations

- 当前只接收 OTLP traces over HTTP，不接收 OTLP gRPC、metrics 或 logs；app 识别依赖
  `service.name`/`tokenbuddy.app` 等属性，属性缺失时保留 `unknown` 并不强行跨 app 关联。
- OTel 不推断官方订阅 quota，也不把未知成本写成零；官方 usage 优先级还需要各 Provider 的
  原生 usage adapter 或未来 provider-reported ingestion。
- 可选本地 Proxy 仍未实现，且不能成为启动或统计前置条件；Windows 托盘和真机运行时验收仍待
  Windows CI/真机。
