# Current model pricing coverage

## Purpose

补齐本机实际出现的 GPT-5.6 Sol/Terra/Luna、Claude Opus 5 和 Claude Fable 5 的 API-equivalent 费用估算，并让已有 SQLite 事件在应用打开时使用最新价格卡重新计算。

## Affected files

- `crates/storage/src/pricing.rs`
- `crates/storage/src/lib.rs`
- `apps/desktop/src/features/dashboard/DashboardView.tsx`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`

## Behaviour changes

- OpenAI `gpt-5.6`/`gpt-5.6-sol`、`gpt-5.6-terra`、`gpt-5.6-luna` 使用各自的输入、缓存命中、缓存写入和输出价格；缓存写入在会话日志提供该字段时才计入，不把缺失字段伪造成 0。
- Anthropic `claude-opus-5` 和 `claude-fable-5` 使用官方 5 分钟缓存写入、缓存命中、输入和输出价格，并继续要求缓存写入字段存在后才给出完整估算。
- 应用打开时扫描已有无供应商实报费用的事件，按当前价格卡回填 `estimated_cost` 和 USD 货币；供应商实报费用不会被覆盖。
- 费用说明明确标识 `~` 为估算；当 OpenAI 会话日志没有拆分缓存写入时，估算仅包含已记录的输入、缓存命中和输出部分。

## Verification

- `cargo fmt --all -- --check` 通过。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` 通过。
- `cargo test -p tokenbuddy-storage -- --nocapture` 通过，20 项测试通过；`cargo test --workspace --quiet` 通过。
- `pnpm --filter @tokenbuddy/desktop format:check`、`lint`、`test -- --run` 通过，前端 55 项测试通过；生产构建通过。
- `sh scripts/build-app.sh` 成功产出 macOS `.app`/`.dmg`；首次启动遇到一次 macOS `-600` 进程接管竞争，重试后 `/Applications/TokenBuddy.app` 正常运行。
- 本机 SQLite 检查确认 `claude-opus-5` 1,413/1,413、`claude-fable-5` 982/982、GPT-5.6 Luna 4,920/4,920、Sol 8,301/8,301、Terra 1,511/1,511 条事件已有 `estimated_cost`。

## Remaining limitations

- 这是按官方 API 单价折算的估算，不代表 ChatGPT/Codex 或 Claude 订阅额度、优惠、Batch、Fast mode、区域加价或第三方中转实际账单。
- 第三方 Provider 不套用 OpenAI/Anthropic 官方价格；未知模型、缺少必要 token 字段或无法区分的缓存写入继续显示 `N/A`/部分估算。
- Windows 真机运行与安装验证仍需 Windows CI 或真机完成。
