# Windows 小体积安装包

## 目的

修复 `v0.1.4` Windows MSI 与 NSIS 安装器达到约 207 MB 的分发问题。体积膨胀来自每种安装包都内嵌了完整 WebView2 离线安装器，并非 TokenBuddy 应用本体。

## 受影响文件

- `apps/desktop/src-tauri/tauri.conf.json`
- `apps/desktop/package.json`
- `Cargo.toml`
- `Cargo.lock`
- `.github/workflows/release.yml`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`
- `docs/change-logs/2026-08-12-03-compact-windows-installer.md`

## 行为变化

- WebView2 安装模式由 `offlineInstaller` 改为 `downloadBootstrapper`，不再内嵌约 127 MB 的离线 Runtime。
- Windows 已有 WebView2 时直接安装；缺失时安装器静默联网下载 Microsoft WebView2 Bootstrapper。
- Release workflow 会检查 MSI 与 NSIS 安装器，任一文件超过 50 MiB 即终止发布。
- Release 说明不再声称 Windows 首次安装完全离线，并明确缺少 WebView2 时需要网络。
- 桌面端、Tauri、Rust workspace 与锁文件版本统一升级到 `0.1.5`。

## 验证

- Tauri 官方 Windows Installer 文档确认：`downloadBootstrapper` 是默认模式，额外安装包体积为 0 MB；`offlineInstaller` 会增加约 127 MB。
- `pnpm --filter @tokenbuddy/desktop format:check`：通过。
- `pnpm --filter @tokenbuddy/desktop lint`：通过。
- `pnpm --filter @tokenbuddy/desktop test`：56 passed。
- `pnpm --filter @tokenbuddy/desktop build`：TypeScript 与 Vite 生产构建通过。
- `pnpm --filter @tokenbuddy/desktop exec prettier --check ../../.github/workflows/release.yml`：Release workflow 格式检查通过。
- PowerShell JSON 契约检查：Tauri 版本为 `0.1.5`，`webviewInstallMode.type = downloadBootstrapper`，`silent = true`。
- `git diff --check`：通过。
- 当前本机未安装或未暴露 `cargo`；Rust/Tauri 跨平台检查及真实 MSI/NSIS 体积由 GitHub Actions 执行。

## 剩余限制

- 缺少 WebView2 且完全离线的 Windows 机器无法完成首次安装；已有 WebView2 的机器不受影响。
- 安装包仍未进行 Windows Authenticode 代码签名，SmartScreen 提示保持不变。
