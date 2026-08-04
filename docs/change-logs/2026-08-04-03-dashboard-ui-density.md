# 2026-08-04 完整面板 UI 密度重构

## 目的

收紧完整 Dashboard 的布局密度，消除导航左侧无意义的固定空白，以及筛选区、按模型/供应商区由全局固定最小高度造成的大面积空白。

## 影响文件

- `apps/desktop/src/styles.css`
- `apps/desktop/src/features/dashboard/DashboardView.tsx`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`
- `docs/change-logs/2026-08-04-03-dashboard-ui-density.md`

## 行为变化

- 主导航改为内容自适应宽度并从左侧排列；Dashboard 导航与数据源区之间恢复明确的区块间距。
- 通用 `.panel` 改为内容驱动高度；只有明确的空状态组件保留自己的最小展示空间，筛选区和数据表不会再被撑到 480px。
- 筛选网格显式按内容高度排列，搜索字段扩展到整行，模型/供应商区增加独立的底部节奏间距，数据少时仍保持清晰的层级关系。
- 未修改筛选查询、导出、扫描、会话详情或任何 Rust/SQLite 数据逻辑。

## 验证

- `pnpm --filter @tokenbuddy/desktop format:check`：通过。
- `pnpm --filter @tokenbuddy/desktop lint`：通过。
- `pnpm --filter @tokenbuddy/desktop test`：55 项通过。
- `pnpm --filter @tokenbuddy/desktop build`：通过。
- 本地 Vite 页面视觉回归：导航宽度约 346px；筛选区高度约 294px；按模型/供应商区在无数据时约 120px，均不再继承 480px 固定最小高度。

## 剩余限制

- 浏览器预览没有 Tauri IPC，只能验证布局和空数据状态；真实本地数据仍需在桌面壳中查看。
- Windows 真机托盘、高 DPI 和完整面板交互仍属于项目规范中尚未完成的跨平台验收项。
