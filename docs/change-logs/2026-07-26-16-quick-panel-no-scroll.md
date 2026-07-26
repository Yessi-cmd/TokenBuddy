# Quick 面板完整展示

## 目的

修复 Quick 面板内容超过窗口高度后必须拖动滚动的问题。菜单栏面板应在点击后作为一个完整的 popover 展示，而不是把内部滚动条交给用户。

## 受影响文件

- `apps/desktop/src-tauri/tauri.conf.json`
- `apps/desktop/src-tauri/src/lib.rs`
- `apps/desktop/src/styles.css`

## 行为变化

- Quick 窗口高度从 460 调整为 540，并同步提高最小高度，给完整摘要、会话选择器、四项指标和额度状态预留空间。
- 移除 Quick 面板内部纵向滚动和滚动条样式，窗口内容一次完整呈现。
- 保留固定的菜单栏锚点定位和边界避让；fallback 几何尺寸与新的窗口高度同步。

## 验证

- `pnpm --filter @tokenbuddy/desktop test`：8 项通过。
- `pnpm --filter @tokenbuddy/desktop format:check`、`pnpm --filter @tokenbuddy/desktop lint`、`cargo fmt --all -- --check`：通过。
- `cargo test --workspace --all-targets`：Codex Adapter 8 项、Core 6 项、Phase 4b 集成 1 项、Tauri 13 项、Domain 3 项、Storage 3 项全部通过。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`：通过。
- `pnpm --filter @tokenbuddy/desktop build`：通过。
- `pnpm --filter @tokenbuddy/desktop tauri build --debug`：通过，重新生成 debug `.app` 和 `.dmg`。

## 剩余限制

- Quick 面板目前使用固定 540 logical px 高度；若系统显示缩放或字体设置显著放大，仍需在对应真机环境复核。
- 当前环境未完成 Computer Use 截图复验，因此最终视觉效果需要用户重新打开新 bundle 确认。
