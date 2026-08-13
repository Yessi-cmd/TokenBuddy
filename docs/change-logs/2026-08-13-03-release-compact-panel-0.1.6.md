# 发布紧凑完整面板 v0.1.6

## 目的

将完整面板 Hero 移除和导航位置统一作为 `v0.1.6` 发布，并让 GitHub Release 清楚说明本版本的界面变化。

## 受影响文件

- `Cargo.toml`
- `Cargo.lock`
- `apps/desktop/package.json`
- `apps/desktop/src-tauri/tauri.conf.json`
- `.github/workflows/release.yml`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`
- `docs/change-logs/2026-08-13-03-release-compact-panel-0.1.6.md`

## 行为变化

- 桌面端、Tauri 与 Rust workspace 版本由 `0.1.5` 升级为 `0.1.6`。
- GitHub Release 说明新增完整面板 Hero 移除与共享导航固定在内容区左上角的说明。
- 继续使用现有 `v*` tag 工作流构建 Windows MSI/NSIS、macOS DMG、Windows 自动更新签名及 `latest.json`。

## 验证

- PowerShell 版本契约检查：桌面端、Tauri、Rust workspace 与所有 `tokenbuddy-*` 锁文件包均为 `0.1.6`。
- `pnpm --filter @tokenbuddy/desktop format:check`：通过。
- `pnpm --filter @tokenbuddy/desktop lint`：通过。
- `pnpm --filter @tokenbuddy/desktop test`：58 passed。
- `pnpm --filter @tokenbuddy/desktop build`：TypeScript 与 Vite 生产构建通过。
- `git diff --check`：通过。
- Rust workspace check 未在本机执行：当前 Windows 环境没有可用的 `cargo` 命令；tag Release workflow 会在 Windows/macOS runner 安装项目指定 Rust toolchain 并执行 Tauri 构建。
- GitHub Actions Release run `31701968925`：Windows bundle、macOS bundle 和 Publish release 全部成功。
- GitHub Release `v0.1.6`：已于 2026-08-13 发布为正式版本（非 draft、非 prerelease）。
- 发布附件确认：Windows NSIS EXE、中文 MSI、两个更新签名、`latest.json` 和 macOS ARM64 DMG 均已上传；NSIS 为 4,861,335 bytes，MSI 为 7,028,736 bytes，均低于 50 MiB 上限。

## 剩余限制

- 安装包仍未进行 Windows Authenticode 或 Apple Developer ID 签名；对应系统可能显示安全确认。
- Windows/macOS 真机托盘交互验收仍保持为项目现有未完成项。
