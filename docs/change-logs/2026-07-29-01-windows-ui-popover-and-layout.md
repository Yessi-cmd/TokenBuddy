# 2026-07-29 Windows UI 弹窗定位与窄窗口布局

## 目的

修复 Windows 托盘快速面板在内容自适应高度后与托盘脱离、顶部任务栏场景定位到屏幕外，以及主面板在 Windows 缩放或较窄窗口下数据源路径控件错位的问题。

## 影响文件

- `apps/desktop/src-tauri/src/lib.rs`
- `apps/desktop/src/App.test.tsx`
- `apps/desktop/src/lib/api.ts`
- `apps/desktop/src/lib/api.test.ts`
- `apps/desktop/src/features/dashboard/DashboardView.tsx`
- `apps/desktop/src/styles.css`
- `docs/change-logs/2026-07-29-01-windows-ui-popover-and-layout.md`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`

## 行为变化

- 快速面板内容高度变化现在通过 Tauri 命令统一调整窗口尺寸并重新锚定托盘，Windows 首次打开不会因高度收缩留下底部空隙。
- Windows 快速面板会读取当前显示器工作区：底部任务栏优先显示在托盘上方，顶部任务栏在上方空间不足时改为显示在下方，并继续受工作区边界夹紧。
- 快速面板高度按托盘所在显示器的工作区封顶，窗口内容过长时保留纵向滚动；跨不同 DPI 的显示器定位时，先按目标显示器缩放计算面板物理尺寸，避免移动后因 Windows DPI 调整再次脱离托盘。
- 数据源路径控件改用三列网格，标签、输入框和检测按钮保持同一行；输入框允许收缩，Windows 长路径不会把卡片撑出横向滚动区域。
- 检测结果为空时不再渲染占满一行的空容器；页面也不再用全局 `overflow-x: hidden` 掩盖残余布局问题，浏览器面板仍可访问真实溢出内容。
- 快速面板清除页面最小高度约束，补充圆角、阴影和 `backdrop-filter` 不可用时的实色回退，提升 WebView2 兼容性与可读性。

## 验证

- 浏览器本地回归：`1100×720` 与 `860×600` 窗口尺寸下无横向溢出；`860×600` 时文档宽度等于视口、空检测结果容器数量为 0；`320×260` 快速面板内容高约 `429px`，页面保持 `overflow-y: auto` 且无横向溢出，可在窗口高度受限时滚动。
- `pnpm --filter @tokenbuddy/desktop test`：54 项通过，包含数据源空检测行回归测试。
- `pnpm --filter @tokenbuddy/desktop build`：通过。
- `pnpm --filter @tokenbuddy/desktop lint`：通过。
- `pnpm --filter @tokenbuddy/desktop format:check`：通过。
- `cargo fmt --all -- --check`：通过。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`：通过。
- `cargo test --workspace --all-targets`：全部通过；桌面壳 29 项测试包含任务栏方向、目标显示器 DPI 和工作区高度上限回归测试。

## 剩余限制

- 当前开发机没有 `x86_64-w64-mingw32-gcc`，因此 `cargo check -p tokenbuddy-desktop --target x86_64-pc-windows-gnu` 在 `libsqlite3-sys` 构建阶段无法完成；Windows MSVC 构建和真实托盘交互仍需由 Windows CI/真机验证。
- Windows 高 DPI、顶部/底部任务栏、路径选择器和窗口隐藏后持续采集仍未替代规格中要求的 Windows 真机验收。
