# 统一额度页导航基线

## 目的

修复从其他完整面板页面切换到“额度”时，主导航因额度页单独使用更小顶部间距而视觉上向上移动的问题。

## 受影响文件

- `apps/desktop/src/features/quotas/QuotasView.tsx`
- `apps/desktop/src/styles.css`
- `apps/desktop/src/App.test.tsx`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`
- `docs/change-logs/2026-08-13-05-align-quota-navigation-baseline.md`

## 行为变化

- 移除额度页多余的 `quota-page` 外层容器。
- 删除额度页桌面端 `30px` 和窄窗口 `26px` 的独立顶部 padding。
- 额度页现在与会话、Providers、数据源和设置页统一使用 `.app-shell` 的标准顶部间距。
- 切换到额度页时导航栏不再向窗口顶部跳动。
- 额度页面板仍使用内容驱动高度，账号、额度快照和刷新交互不变。
- 新增路由结构回归断言，确保额度页直接使用标准页面根节点且不恢复独立外层。

## 验证

- `pnpm exec vitest run src/App.test.tsx -t "keeps navigation first"`（在 `apps/desktop` 中执行）：5 passed，36 skipped。
- `pnpm --filter @tokenbuddy/desktop format:check`：通过。
- `pnpm --filter @tokenbuddy/desktop lint`：通过。
- `pnpm --filter @tokenbuddy/desktop test`：58 passed。
- `pnpm --filter @tokenbuddy/desktop build`：TypeScript 与 Vite 生产构建通过。
- `git diff --check`：通过。

## 剩余限制

- 本批次统一页面内容区的顶部基线；Windows 原生标题栏高度仍由系统和显示缩放设置决定。
