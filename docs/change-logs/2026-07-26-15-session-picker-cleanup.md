# Quick 会话选择器清理

## 目的

整理菜单栏 Quick 面板中的会话选择列表，避免脱敏测试记录和无标题记录干扰真实会话判断，同时保留本地数据库中的原始记录和统计数据。

## 受影响文件

- `apps/desktop/src/App.tsx`
- `apps/desktop/src/App.test.tsx`

## 行为变化

- Quick 会话选择器只展示有真实标题的会话。
- 隐藏无标题 / `Unavailable` 会话。
- 精确隐藏脱敏的 `Fixture session`（`codex-session` / `simple-session`），不删除数据库记录。
- 活动会话不可展示时自动选择第一条可展示会话；没有可展示会话时选择器保持明确的 `Unavailable` 状态。
- 重复标题按不同时间保留，因为它们可能对应不同的真实会话。

## 验证

- `pnpm --filter @tokenbuddy/desktop test`：8 项通过。
- `pnpm --filter @tokenbuddy/desktop format:check`：通过。
- `pnpm --filter @tokenbuddy/desktop lint`：通过。
- `pnpm --filter @tokenbuddy/desktop build`：通过。
- `pnpm test`：桌面 8 项、Codex Adapter 8 项、Core 6 项、Phase 4b 集成 1 项、Tauri 13 项、Domain 3 项、Storage 3 项全部通过。
- `pnpm lint`、`pnpm check:rust`、`pnpm format:check`、`git diff --check`：通过。
- `pnpm --filter @tokenbuddy/desktop tauri build --debug`：通过，重新生成 debug `.app` 和 `.dmg`。

## 剩余限制

- 本批次只整理 Quick 面板的展示列表；完整 `/sessions` 历史页仍保留全部 Core 返回记录。
- 数据库中的 `Fixture session` 和无标题会话未删除；如需物理清理，需单独确认删除范围。
