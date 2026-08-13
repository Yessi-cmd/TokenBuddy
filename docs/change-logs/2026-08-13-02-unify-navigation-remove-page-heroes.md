# 统一完整面板导航并移除页面 Hero

## 目的

让完整面板的共享导航在所有路由中始终位于内容区左上角，同时删除各页面重复且占用大量空间的英文眉题、大号标题和说明副标题。

## 受影响文件

- `apps/desktop/src/components/Navigation.tsx`
- `apps/desktop/src/features/dashboard/DashboardView.tsx`
- `apps/desktop/src/features/sessions/SessionsView.tsx`
- `apps/desktop/src/features/sessions/SessionRouteView.tsx`
- `apps/desktop/src/features/providers/ProvidersView.tsx`
- `apps/desktop/src/features/quotas/QuotasView.tsx`
- `apps/desktop/src/features/sources/SourcesView.tsx`
- `apps/desktop/src/features/settings/SettingsView.tsx`
- `apps/desktop/src/styles.css`
- `apps/desktop/src/App.test.tsx`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`
- `docs/change-logs/2026-08-13-02-unify-navigation-remove-page-heroes.md`

## 行为变化

- 总览、会话、会话详情、Providers、官方额度、数据源和设置页面均先显示共享主导航。
- 主导航不再嵌在各页面 Hero 的右侧，因此不会在切换到会话等页面时跳到右上角。
- 删除各路由顶部的英文眉题、大号页面标题和说明副标题。
- 保留页面内部真实功能区的二级标题、操作按钮、状态信息和数据内容。
- 托盘轻量面板保持原样。
- 删除已无调用方的 Hero 大标题、眉题和副标题 CSS。

## 验证

- `pnpm exec vitest run src/App.test.tsx -t "keeps navigation first|renders the dashboard shell"`（在 `apps/desktop` 中执行）：6 passed，35 skipped。
- `pnpm --filter @tokenbuddy/desktop format:check`：通过。
- `pnpm --filter @tokenbuddy/desktop lint`：通过。
- `pnpm --filter @tokenbuddy/desktop test`：58 passed。
- `pnpm --filter @tokenbuddy/desktop build`：TypeScript 与 Vite 生产构建通过。
- `git diff --check`：通过。

## 剩余限制

- “固定在左上角”指所有完整面板路由使用一致的左上角文档流位置；导航不会悬浮覆盖内容，也不会在页面滚动时吸附窗口顶边。
