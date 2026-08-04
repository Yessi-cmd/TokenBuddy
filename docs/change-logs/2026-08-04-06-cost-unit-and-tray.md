# Cost units and tray summary

## Purpose

让费用展示带上明确的 USD 单位，并把今日费用与当前活动会话费用带入托盘快速面板和托盘 tooltip。

## Affected files

- `crates/domain/src/lib.rs`
- `crates/storage/src/lib.rs`
- `apps/desktop/src-tauri/src/lib.rs`
- `apps/desktop/src/lib/api.ts`
- `apps/desktop/src/lib/format.ts`
- `apps/desktop/src/features/dashboard/DashboardView.tsx`
- `apps/desktop/src/features/providers/ProvidersView.tsx`
- `apps/desktop/src/features/quick/QuickSummaryView.tsx`
- `apps/desktop/src/App.test.tsx`

## Behaviour changes

- 完整面板指标、Provider 摘要和按模型/供应商表格的费用显示为 `$... USD` 或 `~$... USD`，`~` 仍表示 API-equivalent 估算。
- `QuickSummary` 新增当前会话与本地日的供应商实报费用、估算费用字段；托盘显示“费用（USD）”行。
- 托盘 tooltip 包含今日费用；供应商实报费用优先于估算费用，字段缺失保持 `N/A`。
- 今日 Token 统计保留原有语义：无事件时为 `0`，有事件但必要字段不完整时为 `Unavailable`。

## Verification

- `cargo fmt --all -- --check` 通过。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` 通过。
- `cargo test --workspace --quiet` 通过；包含 Tauri 29 项、Storage 20 项及 Core/Adapter 测试。
- `pnpm --filter @tokenbuddy/desktop format:check`、lint、55 项前端测试和生产构建通过。
- `sh scripts/build-app.sh` 成功重建 macOS `.app`/`.dmg`；新应用已安装并运行，SQLite schema version 为 8，已支持价格模型的 17,194 条事件均有估算费用。

## Remaining limitations

- 托盘今日费用遵循完整字段聚合规则；当天包含无法识别模型或缺失费用的事件时显示 `N/A`，不伪造部分总额。
- Claude 缓存写入来源未携带 5 分钟/1 小时期限时，价格层只能按已记录的缓存字段估算；第三方 Provider 不套用官方价格。
- Windows 真机托盘布局仍需 Windows CI 或真机验收。
