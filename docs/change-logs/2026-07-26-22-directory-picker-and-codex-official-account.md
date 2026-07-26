# 2026-07-26-22 目录选择器与 Codex 官方账号适配

## 目的

1. 设置页的四个数据源路径此前只能手工输入，规格 §29.3 要求 macOS 与 Windows 都支持「手动选择目录」。补上原生目录 / 文件选择器。
2. 规格 §8.1、§8.4、§15.3 要求识别 Codex 官方账号并把官方额度作为独立数据类型保存。此前 `accounts` 只有从模型名推断出的占位账号，`quota_snapshots` 只有读取路径而没有任何写入方，额度页永远为空。本批次接入 Codex `auth.json` 的账号身份与 rollout 日志中的官方额度窗口。

## 影响文件

### 目录选择器

- `Cargo.toml`、`apps/desktop/src-tauri/Cargo.toml`：新增 `tauri-plugin-dialog 2.7.2`（workspace 统一版本）。
- `apps/desktop/src-tauri/src/lib.rs`：注册 dialog 插件；新增 `pick_directory`、`pick_file` 两个异步命令，通过单槽 channel 接收回调结果，UI 线程与命令任务互不阻塞；`start_directory` 让选择器从当前已配置路径（文件则取其父目录）打开，路径不存在时回落系统默认。
- `apps/desktop/src/lib/api.ts`：新增 `pickDirectory`、`pickFile`（桌面专用 `invoke`）。
- `apps/desktop/src/App.tsx`、`apps/desktop/src/styles.css`：`SettingsField` 支持可选「浏览…」按钮，仅在 `isDesktopRuntime()` 为真时渲染；选择只填入输入框，保存仍是显式操作；CC Switch / Cockpit 使用文件选择器并带 SQLite 扩展名过滤。

### Codex 官方账号与官方额度

- `crates/domain/src/lib.rs`：新增 `AccountRecord`、`AccountSummary`；`ImportBatch` 新增 `accounts`、`quota_snapshots`。
- `crates/adapters/codex-session/src/account.rs`（新增）：只读解析 `<CODEX_HOME>/auth.json`，支持 ChatGPT OAuth 与 `OPENAI_API_KEY` 两种模式；不校验签名地解码 id_token 载荷读取 `chatgpt_account_id`、`chatgpt_plan_type`、`email`；自带 base64url 解码器，不引入新依赖。
- `crates/adapters/codex-session/src/lib.rs`：新增 `with_fingerprint_salt` / `official_account`；导入时产出 OpenAI provider 与官方账号；解析 `rate_limits`（支持根、`payload`、`info`、`payload.info` 四种位置）生成额度快照；同一文件内同一窗口百分比未变化时不重复产出。
- `crates/storage/migrations/0005_local_fingerprint_salt.sql`、`crates/storage/src/migrations.rs`：新增每安装随机盐列。
- `crates/storage/src/lib.rs`：新增 `local_salt()`（首次调用用 SQLite CSPRNG 生成并持久化）、`upsert_account_record`、`insert_quota_snapshot`、`list_accounts()`；`apply_import_batch` 按 provider → account → 事件 → 额度顺序落库；`ImportStats` 增加 `upserted_accounts`、`inserted_quota_snapshots`；新增 `StorageError::MissingLocalSalt`。
- `crates/core/src/lib.rs`：导入 Codex 前从数据库取盐并注入适配器；新增 `list_accounts()`；`ImportReport` 透出账号与额度计数。
- `apps/desktop/src-tauri/src/lib.rs`、`src/web.rs`：新增 `list_accounts` 命令与 `GET /api/accounts` 路由（选择器无网页对应物，浏览器面板保留文本输入）。
- `apps/desktop/src/App.tsx`：额度页新增「已识别账号」区块，展示账号、Provider、登录方式、订阅方案、指纹前 12 位与最近额度窗口，全部缺失态显式写 Unavailable。
- `fixtures/codex/rate_limits.jsonl`、`fixtures/codex/auth/chatgpt_auth.json`、`fixtures/codex/auth/api_key_auth.json`（新增脱敏 fixture，令牌与 Key 均为占位串）。

## 行为变化

- 桌面端设置页四个路径旁出现「浏览…」；取消选择不修改任何已配置路径；浏览器面板不显示该按钮。
- 检测到 Codex 官方账号后：`accounts` 表出现真实账号行（ChatGPT 登录显示邮箱与订阅方案，API Key 模式只显示盐化指纹前 8 位）；该 Codex Home 导入的 usage 事件带上该账号，`precision_account` 为 `Correlated`。
- rollout 日志中的 `rate_limits` 写入 `quota_snapshots`，窗口名按上报的窗口长度标注（如 `primary_5h`、`secondary_7d`）；`remaining_percent` 缺失时由上报的 `used_percent` 取补数；`reset_at` 由 `resets_in_seconds` 折算。托盘 `QuickSummary` 因此首次能显示官方额度窗口。
- 未检测到账号（无 `auth.json` / 无法解析 / 未配置盐）时，账号与额度都保持 Unavailable，usage 事件的 `account_id` 仍为 `None`——不会退化成占位值。

## 关键取舍

- **额度精度记为 `Correlated` 而非 `Verified`。** 百分比本身来自上游速率限制响应（官方数据），但「属于哪个账号」是通过 Codex Home 关联出来的，日志本身不写账号。按 §14「不得把弱关联显示成 Verified」，以最弱环节定级。
- **账号归因是关联，不是事实。** `auth.json` 只描述当前登录的账号，历史事件同样会关联到它；若用户中途换过账号，历史归因会偏向当前账号。这是 `Correlated` 的含义，UI 有徽标；日志无法提供更强的证据。
- **指纹带每安装随机盐**（§20.2），盐存在 `app_settings.local_salt`，不进入 `AppSettings`、IPC、loopback API 或导出，因此复制走数据库也无法反查账号 id 或 API Key。原始 token / key 全程不落库。
- **额度百分比绝不反推 Token**（§8.4）：额度是独立表与独立类型，集成测试断言额度行不产生任何 usage 事件。

## 验证

均在本机 macOS 执行：

- `cargo fmt --all -- --check`：通过。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`：通过。
- `cargo test --workspace --all-targets`：通过（新增 `tokenbuddy-codex-session` 5 项账号解析 + 3 项导入用例、`tokenbuddy-storage` 2 项、`tokenbuddy-core` 集成用例 `phase6_codex_account`）。
- `apps/desktop` 前端：`prettier --check .`、`eslint . --max-warnings 0`、`vitest run`（12 项，含选择器填充/不保存、浏览器面板无按钮、账号区块渲染）、`tsc -b && vite build`：全部通过。
- `cargo build -p tokenbuddy-desktop`：通过（确认 dialog 插件链接）。

关键断言：重复导入同一 fixture 后 `inserted_events`、`inserted_quota_snapshots` 均为 0，额度行数不变；同一窗口百分比不变的连续 `token_count` 行只保留一行。

## 遗留限制

- 原生选择器只能人工点击验证，本批次未在 macOS / Windows 真机做交互验收；Windows 上的文件过滤器表现同样未验证。
- Codex 之外的账号（Claude Code、CC Switch、Cockpit）仍是占位账号；Claude 官方额度未接入。
- `rate_limits` 的字段名依据现有 Codex rollout 结构实现，遇到未知形态会整体跳过并保持 Unavailable，不会误写；若后续发现新形态需新增 fixture 而不是改现有 fixture。
- 换账号后的历史归因见上文取舍，未实现按时间窗切分多账号。
