# Windows 分发与自动更新补全

## 目的

按 Windows 版优化清单落地：修复 IPv6 禁用机器上本地 Web 面板整体不可用的问题；让 Windows 安装包首次安装完全离线（内嵌 WebView2）；接入 Windows 应用内自动更新；为 CI 加 Cargo 构建缓存；并修掉 Windows 上 Tauri MockRuntime 测试二进制加载即崩溃（`STATUS_ENTRYPOINT_NOT_FOUND`）的问题，恢复 Windows CI 的命令契约测试覆盖。

## 受影响文件

- `apps/desktop/src-tauri/src/web.rs` — IPv6 绑定降级 + 新单元测试
- `apps/desktop/src-tauri/tauri.conf.json` — `bundle.windows`（offlineInstaller / NSIS SimpChinese / WiX zh-CN）、`bundle.createUpdaterArtifacts`、`plugins.updater`（pubkey + endpoints）
- `apps/desktop/src-tauri/src/lib.rs` — Windows-only updater 插件注册与后台更新检查（原生对话框 → 下载安装）
- `apps/desktop/src-tauri/Cargo.toml`、`Cargo.toml` — 新增 `tauri-plugin-updater` 2.10.1（workspace 钉版）
- `apps/desktop/src-tauri/build.rs`、`apps/desktop/src-tauri/windows-app-manifest.xml`（新增）— 为所有目标（含测试二进制）注入 Common Controls v6 manifest
- `apps/desktop/src-tauri/capabilities/default.json` — `updater:default`
- `.github/workflows/ci.yml` — Cargo 构建缓存
- `.github/workflows/release.yml` — 签名、updater 工件上传、`latest.json` 生成、去掉 pre-release 标记
- `Cargo.lock`

## 行为变化

- **IPv6 降级**：`::1` 绑定失败（IPv6 被禁用的企业网/VPN 机器）只丢弃 IPv6 监听，`127.0.0.1` 面板照常可用；`loopback_urls` 相应只报 IPv4。
- **离线安装**：Windows NSIS 与 MSI 安装包内嵌 WebView2 离线安装器（体积约 +127MB），目标机器无需联网即可完成首次安装；updater 更新包仍用内嵌 bootstrapper（约 +1.8MB，WebView2 已装时自动跳过）。
- **自动更新（仅 Windows）**：启动 10 秒后在后台检查 GitHub Releases 的 `latest.json`；发现新版本弹原生对话框"立即更新 / 稍后"；下载安装后进程退出并由 NSIS 以 `/UPDATE` 模式自动重启。检查失败（离线、GitHub 不可达、签名不符）只记日志，不阻塞采集。
- **发布流程**：Release 不再标记 pre-release——GitHub `/releases/latest/download`（更新检查的 endpoint）不解析 pre-release；`TAURI_SIGNING_PRIVATE_KEY` 作为仓库 secret 注入 bundle job（值为 minisign 密钥盒 base64；密钥由 `tauri signer generate --ci` 生成、无密码，同时显式传空 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 避免 bundler 交互式询问；公钥写入 `plugins.updater.pubkey`）。Windows job 构建后生成并上传 `latest.json`（引用 NSIS updater zip + 其 `.sig`）。
- **踩坑记录**：Tauri 2 的 updater 签名是 minisign（Ed25519）而非 RSA——openssl 生成的 PKCS#8 密钥无法通过 bundler 校验；bundler 只读 `TAURI_SIGNING_PRIVATE_KEY`（值可以是文件路径，`TAURI_SIGNING_PRIVATE_KEY_PATH` 在打包流程中不被读取）；`latest.json` 由 bundler 生成，需 workflow 自行产出。
- **Windows 测试恢复**：`build.rs` 为 windows-msvc 的所有目标注入与 tauri-build 默认相同的 Common Controls v6 manifest（此前只有 `bin` 目标通过 winres 获得，测试二进制在加载阶段因 `TaskDialogIndirect` 无法在 comctl32 v5 中定位而以 `0xc0000139` 退出）；`command_tests` 与 `tauri` test feature 不再按平台排除，Windows CI 重新运行命令契约测试。
- **CI 提速**：`ci.yml` 增加 Cargo registry/git/target 缓存（key 含 `Cargo.lock` hash）。

## 验证

- 本机：`pnpm format:check`、`pnpm lint`（eslint + clippy `-D warnings`）、`pnpm test`（vitest 55 项 + cargo workspace 全目标；OTel loopback 集成测试在全量并发下偶发超时一次，单独重跑 5 次全过，属已知时序抖动）、`pnpm build:web`、`pnpm check:rust`、`pnpm --filter @tokenbuddy/desktop tauri build --debug --no-bundle` 均通过。
- 桌面 crate 31 项测试通过，含新增 `loopback_url_list_omits_ipv6_when_its_listener_could_not_bind`。
- 签名链路已用 `TAURI_SIGNING_PRIVATE_KEY` 做 release 打包验证：`TokenBuddy.app.tar.gz (updater)` 与 `.sig` 正常产出，签名 key_id 与 `plugins.updater.pubkey` 的公钥前 8 字节一致。
- 待 CI 验证：Windows MSVC 下 MockRuntime 测试是否随 manifest 修复恢复运行、NSIS 中文语言与 WiX zh-CN 本地化是否在 `windows-latest` 上打包成功、offlineInstaller 是否正常内嵌（这些只能在 Windows runner 上确认）。

## 剩余限制

- Windows-only 更新代码（`spawn_update_checker`）在 macOS 上不参与编译，已按插件 2.10.1 源码逐项核对 API（`download_and_install` 回调签名与文档示例不同：`FnMut(usize, Option<u64>)` + `FnOnce()`），最终由 Windows CI 编译确认。
- `latest.json` 的 `endpoints` 固定在 `/releases/latest/download/latest.json`，依赖 Release 非 pre-release；若恢复 pre-release 发布，更新检查会 404（静默降级为"无更新"）。
- 安装包仍未签名，SmartScreen 提示保留。
- `Phase 4b 跨平台真机交互补验` 仍未完成：托盘、自启动、高 DPI、隐藏窗口持续采集与更新对话框的真机体验待 Windows 实机验收。
