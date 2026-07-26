# 2026-07-26 — 状态栏紧凑图标与面板切换

## 目的

修复 macOS 状态栏面板打开后再次点击图标无法隐藏的问题，并减少状态栏占用空间。

## 影响文件

- `apps/desktop/src-tauri/src/lib.rs`
- `docs/change-logs/2026-07-26-10-tray-toggle-icon.md`

## 行为变化

- 单击状态栏图标现在会在快速摘要面板的显示与隐藏之间切换。
- macOS 状态栏不再显示 `Today {token_count}` 长标题，只保留应用图标。
- 完整的今日 Token、采集状态和 Provider 信息继续保留在状态栏 tooltip 中。
- 双击打开完整面板、右键菜单项行为保持不变。

## 验证

- 新增 Rust 单元测试，覆盖 tooltip 摘要内容和快速面板显示/隐藏切换决策。
- `pnpm format:check` 通过。
- `pnpm lint` 通过。
- `pnpm test` 通过：前端 6 项、Rust workspace 全部测试通过，桌面 Rust 测试 11 项通过。
- `pnpm build:web` 通过。
- `pnpm check:rust` 通过。
- `pnpm --filter @tokenbuddy/desktop tauri build --debug` 通过，生成 macOS `.app` 和 `.dmg`。
- 通过 Computer Use 启动 debug 包并确认快速面板可加载；状态栏 `SystemUIServer` 无障碍树不可访问，未将直接点击操作记为自动化真机证据。

## 剩余限制

- macOS 状态栏图标的直接点击/再次点击仍需人工确认一次；代码已将单击映射为快速面板显示/隐藏切换。
- Windows 托盘的真机交互仍需 Windows 环境验证。
