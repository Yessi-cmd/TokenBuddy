# Windows 自启动与窗口恢复修复

## 目的

补强 Windows 版本在安装路径包含空格时的自启动可靠性，并修复托盘或单实例唤回最小化主窗口后仍不可见的问题。

## 受影响文件

- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/src/lib.rs`
- `Cargo.lock`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`

## 行为变化

- Windows 自启动直接写入当前用户 `Run` 注册表项，并为可执行文件路径加引号，兼容常见的 `Program Files` 安装路径。
- 启用自启动时同步恢复可用的 `StartupApproved` 状态；关闭自启动时对不存在的条目保持幂等。
- 托盘双击、单实例转发和快速面板唤回前调用 `unminimize`，再显示和聚焦窗口。
- 新增 Windows 安装路径引号回归测试；非 Windows 仍使用现有 `tauri-plugin-autostart` 实现。

## 验证

- `cargo test -p tokenbuddy-desktop --lib`：30 passed。
- `pnpm format:check`：通过。
- `pnpm lint`：通过。
- `pnpm test`：通过，前端 55 tests 与 Rust workspace tests 全部通过（含 OTel 4 tests）。
- `pnpm check:rust`：通过。
- `pnpm build:web`：通过。
- `pnpm --filter @tokenbuddy/desktop tauri build --debug --no-bundle`：通过（当前 macOS 主机）。
- `cargo check -p tokenbuddy-desktop --lib --target x86_64-pc-windows-gnu`：被当前 macOS 环境缺少 `x86_64-w64-mingw32-gcc` 阻断，未将其计为 Windows 构建通过。

## 剩余限制

当前工作机不是 Windows，尚未完成 Windows MSVC/Tauri 打包、真实托盘交互、自启动注册表实机、路径选择器、高 DPI、隐藏窗口持续采集及 CPU/P95 验收；规格中的 `Phase 4b 跨平台真机交互补验` 保持未完成。`opencode.json` 为既有未跟踪用户文件，本批次未修改。
