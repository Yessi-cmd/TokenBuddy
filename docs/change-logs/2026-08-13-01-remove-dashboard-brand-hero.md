# 移除完整面板品牌 Hero

## 目的

删除完整桌面面板总览页顶部占用大量纵向空间的品牌展示，让用户打开面板后更快看到导航和 Token 统计内容。

## 受影响文件

- `apps/desktop/src/features/dashboard/DashboardView.tsx`
- `apps/desktop/src/styles.css`
- `apps/desktop/src/App.test.tsx`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`
- `docs/change-logs/2026-08-13-01-remove-dashboard-brand-hero.md`

## 行为变化

- 从总览页移除英文眉题、大号 `TokenBuddy` 标题和中文品牌副标题。
- 原顶部操作区保留采集状态、扫描全部来源和本地网页入口，并在宽屏下靠右排列。
- 托盘轻量面板中的紧凑 `TokenBuddy` 标题保持不变。
- 新增回归断言，确保被移除的三段品牌文案不会重新出现在总览页。

## 验证

- `pnpm exec vitest run src/App.test.tsx -t "renders the dashboard shell"`（在 `apps/desktop` 中执行）：1 passed，38 skipped。
- `pnpm --filter @tokenbuddy/desktop format:check`：通过。
- `pnpm --filter @tokenbuddy/desktop lint`：通过。
- `pnpm --filter @tokenbuddy/desktop test`：56 passed。
- `pnpm --filter @tokenbuddy/desktop build`：TypeScript 与 Vite 生产构建通过。
- `git diff --check`：通过。

## 剩余限制

- 本批次只调整完整面板总览页；窗口原生标题栏与托盘轻量面板继续显示 TokenBuddy，符合保留应用身份和轻量入口可识别性的要求。
