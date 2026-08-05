# Windows GUI subsystem startup

## 目的

修复 Windows 直接启动 `tokenbuddy-desktop.exe` 时额外弹出并持续占用命令行窗口的问题，并作为 `v0.1.3` 发布。

## 受影响文件

- `apps/desktop/src-tauri/src/main.rs`
- `Cargo.toml`
- `Cargo.lock`
- `apps/desktop/package.json`
- `apps/desktop/src-tauri/tauri.conf.json`
- `.github/workflows/release.yml`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`

## 行为变化

- Windows 二进制显式链接为 GUI subsystem，不再创建控制台窗口。
- macOS、Linux 和其他平台的入口行为保持不变。
- 应用版本统一升级到 `0.1.3`，Release workflow 会生成对应版本的安装包。

## 验证

- 已运行 `cargo check --workspace --all-targets`、`pnpm format:check`、`pnpm lint`、`pnpm test`、`pnpm build:web` 和 `pnpm --filter @tokenbuddy/desktop tauri build --debug --no-bundle`；Windows PE subsystem 仍需下一次 Windows CI 或实机确认。

## 剩余限制

当前工作机不是 Windows，无法本地确认具体系统版本上的窗口表现；Windows CI 可验证编译与链接配置，最终桌面体验仍建议在 Windows 实机启动安装后的程序确认。
