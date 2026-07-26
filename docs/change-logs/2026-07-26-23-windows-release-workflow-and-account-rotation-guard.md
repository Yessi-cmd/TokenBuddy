# 2026-07-26-23 Windows 发布流程与多账号轮换保护

## 目的

1. 用户需要在 Windows 上直接安装使用，但仓库此前只有 `--no-bundle` 的编译校验，不产出任何可下载的安装包。macOS 上无法交叉编译 Windows 包（需要 WebView2、MSVC 工具链和 WiX/NSIS bundler），因此必须由 GitHub Actions 在 `windows-latest` 上出包。
2. 上一批次的 Codex 官方账号功能假设一个 Codex Home 只对应一个账号。用户实际用 Cockpit Tools 轮换三个账号（含一个 ChatGPT Plus）驱动 Codex，该假设不成立：`auth.json` 只反映此刻登录的账号，按它归因会把三个账号的历史用量和额度全部算到当前这一个头上。

## 影响文件

- `.github/workflows/release.yml`（新增）：`v*` tag 或手动触发；在 `windows-latest` 与 `macos-15` 上执行 `pnpm build`，上传 `.msi` / `-setup.exe` / `.dmg`；tag 触发时用 `gh release create` 发布为 pre-release，说明未签名与未真机验证。
- `crates/adapters/codex-session/src/lib.rs`：新增 `with_account_rotation`。开启后仍上报账号身份（该账号确实存在且已登录），但不把 `account_id` 写入 usage 事件、也不产出额度快照。
- `crates/core/src/lib.rs`：新增 `account_rotation_detected`，探测到 CC-Switch 或 Cockpit 数据库文件即视为存在账号轮换，并据此配置 Codex 适配器。

## 行为变化

- 打 `v*` tag 后，GitHub Releases 会出现 Windows `.msi` / `.exe` 与 macOS `.dmg`；手动触发只产出 Actions 构件不发布。
- 机器上存在 CC-Switch 或 Cockpit 数据库时：账号页仍显示 Codex 官方账号（邮箱、订阅方案、指纹），但 Codex 用量事件的账号保持 `Unavailable`，官方额度窗口不入库。
- 两者都不存在时，行为与上一批次一致：账号关联为 `Correlated`，额度窗口正常记录并出现在托盘摘要。

## 关键取舍

- **保守优先。** 判定条件只看启动器数据库是否存在，不去判断它是否真的在轮换 Codex 账号。因此"只用 CC-Switch 管 Claude"的用户也会失去 Codex 额度显示。丢一个归因可以后续补回，发布一个错误归因不能——错误数字一旦展示就会被当成事实使用。
- **账号身份与用量归因分开处理。** "这个账号已登录"是事实，"这条历史请求属于它"是推断。前者保留，后者在无法证明时放弃，而不是整体隐藏账号。
- 真正的多账号归因需要从 Cockpit `request_logs` 的 `account_id` / `email` 按时间窗关联，属于下一批次。

## 验证

- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`cargo test --workspace --all-targets`：通过。新增 `account_rotation_reports_the_account_but_attributes_neither_usage_nor_quota`（适配器）与 `an_installed_launcher_marks_the_codex_home_as_rotating_accounts`（Core）。
- 前端 `prettier --check`、`eslint --max-warnings 0`、`vitest run`、`tsc -b && vite build`：通过。
- 发布流程本身只能在 GitHub Actions 上验证，本地无法执行。

## 遗留限制

- **Windows 真机从未验证**：托盘交互、开机自启、路径选择器、安装包安装流程都只有编译保证。
- 安装包未签名，Windows 会触发 SmartScreen 警告，macOS 需手动放行。
- Cockpit 的三个账号目前仍不会作为三个账号出现（适配器只映射 `gateway_mode` 到 Provider，未读取 `request_logs.account_id` / `email`）。
- Codex App（区别于 Codex CLI）是否写 `%USERPROFILE%\.codex\sessions\*.jsonl` 尚未确认；若不写，Codex 侧将没有 Token 数据。
