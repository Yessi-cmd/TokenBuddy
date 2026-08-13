# 对齐总览页顶部工具栏

## 目的

修复完整面板总览页顶部左侧导航与右侧状态、操作按钮不在同一水平线的问题，使左右两侧形成一条清晰、稳定的工具栏。

## 受影响文件

- `apps/desktop/src/features/dashboard/DashboardView.tsx`
- `apps/desktop/src/styles.css`
- `apps/desktop/src/App.test.tsx`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`
- `docs/change-logs/2026-08-13-04-align-dashboard-toolbar.md`

## 行为变化

- 总览页导航和状态/操作区合并到同一个语义化工具栏容器。
- 宽窗口下导航靠左，加载状态、扫描全部来源和本地网页按钮靠右，并保持垂直居中。
- 窄于 980px 时工具栏切换为两行：导航在上、操作区在下，二者均左对齐。
- 会话、Providers、额度、数据源和设置页面的左上角导航位置保持不变。
- 新增回归断言，验证导航与扫描按钮属于同一个总览工具栏。

## 验证

- `pnpm exec vitest run src/App.test.tsx -t "renders the dashboard shell"`（在 `apps/desktop` 中执行）：1 passed，40 skipped。
- `pnpm --filter @tokenbuddy/desktop format:check`：通过。
- `pnpm --filter @tokenbuddy/desktop lint`：通过。
- `pnpm --filter @tokenbuddy/desktop test`：58 passed。
- `pnpm --filter @tokenbuddy/desktop build`：TypeScript 与 Vite 生产构建通过。
- `git diff --check`：通过。

## 剩余限制

- 本批次调整的是响应式文档流对齐，不将工具栏改为滚动时吸附窗口顶部的 sticky 元素。
