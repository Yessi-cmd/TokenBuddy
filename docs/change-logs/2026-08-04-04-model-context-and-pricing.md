# 2026-08-04 模型上下文与费用估算

## 目的

修复 Dashboard 中中文「数据源」标签沿用英文全大写字距导致的字形比例问题，并补齐 Codex/Claude 会话日志的模型上下文传播、旧记录回填和按模型价格表估算费用能力。

## 影响文件

- `apps/desktop/src/features/dashboard/DashboardView.tsx`
- `apps/desktop/src/styles.css`
- `crates/domain/src/lib.rs`
- `crates/storage/src/migrations.rs`
- `crates/storage/migrations/0007_model_cursor_and_reimport.sql`
- `crates/storage/migrations/0008_model_context_backfill.sql`
- `crates/storage/src/lib.rs`
- `crates/storage/src/pricing.rs`
- `crates/adapters/codex-session/src/lib.rs`
- `crates/adapters/claude-session/src/lib.rs`
- `crates/adapters/cc-switch/src/lib.rs`
- `crates/adapters/cockpit/src/lib.rs`
- `crates/adapters/official-quota/src/lib.rs`
- `fixtures/codex/model_inherited_usage.jsonl`
- `fixtures/codex/model_after_usage.jsonl`
- `fixtures/claude/reported_cost.jsonl`
- `fixtures/claude/model_after_usage.jsonl`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`

## 行为变化

- 「数据源」使用中文专用字距和字号，不再套用英文 `uppercase + letter-spacing` 规则；按模型/供应商面板补充费用精度说明。
- Codex/Claude 解析器会把 session metadata、turn context 和响应中的明确模型带到后续用量行；同一会话只有一个明确模型时，即使模型记录晚于 token snapshot，也会回填更早的用量事件；多模型会话不做猜测。
- import cursor 持久化当前模型，增量导入跨越 JSONL 头部后仍能保持模型上下文。
- SQLite 0007/0008 migration 会让原有 native session cursor 安全重读；相同 `raw_event_hash` 的历史行只补缺失模型、Provider 和费用，不新增重复事件。
- Claude `costUSD` 等明确的供应商实报费用进入 `provider_reported_cost`；已报告费用优先于估算费用。
- 新增严格的 Provider + Model 价格表：OpenAI `gpt-5-codex` 和 Anthropic Claude 3.7 Sonnet 按未缓存输入、缓存读取/写入和输出分别计算；缺少必要字段、未知模型或第三方 relay Provider 时继续显示 `N/A`。估算只作为 API-equivalent estimate，不宣称是订阅实际账单。

## 验证

- `cargo fmt --all -- --check`：通过。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`：通过。
- `cargo test --workspace`：全量通过；新增模型传播、费用解析、价格计算和历史行回填测试通过。
- `pnpm --filter @tokenbuddy/desktop format:check`：通过。
- `pnpm --filter @tokenbuddy/desktop lint`：通过。
- `pnpm --filter @tokenbuddy/desktop test -- --run`：55 项通过。
- `pnpm --filter @tokenbuddy/desktop build`：通过。
- `sh scripts/build-app.sh`：macOS `.app` 与 `.dmg` 均构建成功。
- 已将新 `.app` 安装到 `/Applications/TokenBuddy.app` 并启动；本机数据库已迁移到 schema version 8，旧包保留在带时间戳的 `.backup-*` 路径中。

## 剩余限制

- 日志本身没有模型或对应历史文件已经不存在的旧事件，仍会保持 `Unavailable`；不根据相邻会话或模型前缀猜测。
- 价格表目前只覆盖明确版本的 OpenAI `gpt-5-codex` 与 Anthropic Claude 3.7 Sonnet；Codex 订阅的 credits rate card 与 API USD 价格不是同一计费单位，未混入 USD 估算。
- Windows 真机托盘、高 DPI 和完整面板交互仍属于项目规范中尚未完成的跨平台验收项。
