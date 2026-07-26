# 2026-07-26 — 菜单栏 Popover 与会话选择器

## 目的

将快速摘要从独立的深色大卡片改为依附状态栏图标的 macOS 风格 popover，并修正会话选择和数据来源表达。

## 影响文件

- `apps/desktop/src-tauri/src/lib.rs`
- `apps/desktop/src-tauri/tauri.conf.json`
- `apps/desktop/src/App.tsx`
- `apps/desktop/src/App.test.tsx`
- `apps/desktop/src/lib/api.ts`
- `apps/desktop/src/styles.css`
- `crates/domain/src/lib.rs`
- `crates/storage/src/lib.rs`
- `crates/core/src/lib.rs`
- `docs/change-logs/2026-07-26-11-native-popover-session-picker.md`

## 行为变化

- 快速面板使用透明窗口、圆角、磨砂层、阴影和紧凑层级；macOS 面板出现在状态栏图标下方，Windows 预留在托盘图标上方，并按工作区边界夹紧。
- `QuickSummary` 新增 `active_session_id`，最近活动会话可与会话列表稳定对应。
- 快速面板加载 Core 的会话列表，提供点击下拉选择；选择后展示该会话的真实标题和 SQLite 汇总 Token。
- 今日 Token 继续使用本地 SQLite 当天事件的已知输入 + 输出求和；缺失值保持 `Unavailable`，不由 UI 伪造。
- 会话列表按最近结束/更新时间优先，避免旧会话长期排在当前活动会话之前。

## 验证

- `pnpm format:check` 通过。
- `pnpm lint` 通过。
- `pnpm test` 通过：前端 7 项、Rust workspace 全部测试通过。
- `pnpm check:rust` 通过。
- `pnpm --filter @tokenbuddy/desktop tauri build --debug` 通过，生成 macOS `.app` 和 `.dmg`。
- 新增会话 ID、下拉选择、Popover 锚点位置和真实会话汇总测试。

## 剩余限制

- 本机已有的应用 SQLite 可能保留前一轮验收导入的脱敏 `Fixture session`；本批次不擅自删除本地数据，需要单独确认后清理。
- 本轮最后一次 Computer Use 视觉检查因 macOS 锁屏无法启动；状态栏直接点击仍需解锁后人工确认。
- Windows 托盘真机交互和真实 P95 仍需 Windows 环境验证。
