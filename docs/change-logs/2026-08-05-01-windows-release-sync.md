# Windows release synchronization

## 目的

将 GitHub `agent/overnight` 上最近的 OTel、官方额度、模型/费用和 Dashboard 改动与 Windows 发布路径统一，并生成下一版 Windows 安装包。

## 受影响文件

- `Cargo.toml`
- `apps/desktop/package.json`
- `apps/desktop/src-tauri/tauri.conf.json`
- `.github/workflows/release.yml`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/src/lib.rs`
- `Cargo.lock`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`

## 行为变化

- 应用版本从 `0.1.1` 升至 `0.1.2`，与新 release tag 对齐。
- `v*` tag 继续由 GitHub Actions 在 `windows-latest` 和 `macos-15` 上分别构建安装包，并发布 `.msi`、NSIS `.exe` 和 `.dmg`。
- release notes 更新为当前已实现的 OTel、费用估算和 Windows 修复状态。
- Windows 自启动路径引号、StartupApproved 同步和最小化窗口唤回修复随本次版本进入 release。

## 验证

将在提交前记录本机格式、lint、测试、Rust check、前端构建和 macOS Tauri 构建；Windows 安装包以 GitHub Actions `windows-latest` release job 的成功结果作为最终验证。

## 剩余限制

当前工作机不是 Windows，无法本地运行 MSVC、WebView2、托盘和安装器实机验收；安装包未签名，Windows SmartScreen 仍会提示。OTel 仍只支持 loopback OTLP/HTTP traces，本地代理未实现。
