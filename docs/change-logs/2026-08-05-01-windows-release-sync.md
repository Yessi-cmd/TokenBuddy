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
- `crates/otel-receiver/src/lib.rs`
- `Cargo.lock`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`

## 行为变化

- 应用版本从 `0.1.1` 升至 `0.1.2`，与新 release tag 对齐。
- `v*` tag 继续由 GitHub Actions 在 `windows-latest` 和 `macos-15` 上分别构建安装包，并发布 `.msi`、NSIS `.exe` 和 `.dmg`。
- release notes 更新为当前已实现的 OTel、费用估算和 Windows 修复状态。
- Windows 自启动路径引号、StartupApproved 同步和最小化窗口唤回修复随本次版本进入 release。
- Tauri MockRuntime 命令契约测试保留在 macOS/Linux；由于其 Windows 测试二进制在 `windows-latest` 启动阶段返回 `STATUS_ENTRYPOINT_NOT_FOUND`，Windows CI 改为运行桌面纯函数测试并继续执行完整 Tauri Windows 构建。
- OTel loopback HTTP 集成测试改为关闭请求写端、读取明确的 HTTP 响应后再等待 batch，避免慢速 CI 调度造成偶发超时。

## 验证

- 本机 `pnpm format:check`、`pnpm lint`、`pnpm test`、`pnpm build:web`、`pnpm check:rust` 和 `pnpm --filter @tokenbuddy/desktop tauri build --debug --no-bundle` 均通过；OTel loopback 测试连续运行 30 次通过。
- GitHub CI 首轮验证发现 Windows Tauri MockRuntime 测试进程在执行测试前返回 `0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND`；修复为平台条件测试后，Windows 依赖范围问题已修复。最新 CI run `30963683421` 的 Windows job 仍在验证，macOS job 曾因 OTel 测试时序失败，待修复后的新 run 作为最终验证。

## 剩余限制

当前工作机不是 Windows，无法本地运行 MSVC、WebView2、托盘和安装器实机验收；安装包未签名，Windows SmartScreen 仍会提示。OTel 仍只支持 loopback OTLP/HTTP traces，本地代理未实现。
