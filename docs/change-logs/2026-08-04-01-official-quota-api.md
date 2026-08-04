# 官方额度 API 与无 Cockpit 使用统计

## 目的

接入 OpenAI 官方 ChatGPT/Codex 额度接口，展示官方额度窗口与 Credits，并让桌面应用在没有 Cockpit Tools 或 CC Switch 时仍能管理当前 ChatGPT OAuth 账号的官方额度。官方额度与 Session Token 统计保持独立。

## 影响文件

- `Cargo.toml`、`Cargo.lock`
- `crates/adapters/codex-session/src/account.rs`
- `crates/adapters/official-quota/`
- `crates/core/Cargo.toml`、`crates/core/src/lib.rs`
- `crates/core/tests/phase7_official_quota.rs`
- `crates/storage/src/lib.rs`
- `apps/desktop/src-tauri/src/lib.rs`
- `apps/desktop/src-tauri/src/web.rs`
- `apps/desktop/src/lib/api.ts`
- `apps/desktop/src/features/quotas/QuotasView.tsx`
- `fixtures/codex/official_quota_response.json`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`

## 行为变化

- 新增独立 `OfficialQuotaAdapter`，读取 `<CODEX_HOME>/auth.json` 中的 ChatGPT OAuth access token/account id，并请求官方额度接口。
- access token 只存在于单次 HTTP 请求内存中；不会写入账号、cursor、source error、日志或 SQLite。API Key 登录不会被误判为 ChatGPT 订阅额度。
- 支持 primary/secondary 窗口、additional rate limits、spend control、Credits、重置时间和缺失值；官方接口快照标为 `Verified`。
- 以账号身份加完整官方响应生成 cursor hash，重复响应不会新增 `QuotaSnapshot`。
- Core 正式桌面配置开启官方额度后台刷新；官方网络失败只产生官方数据源告警，不会阻断本地 Session/Token 统计。
- 新增 Tauri 命令和 loopback `POST /api/refresh-official-quota` 手动刷新入口；额度页增加刷新按钮并显示 Credits 余额。
- 官方额度查询不依赖 Cockpit/CC Switch；Cockpit 仍只负责历史请求的账号归因，外部数据库继续只读。
- 额度摘要选择同一时间点的 primary/secondary 窗口优先于 Credits，避免 Credits 行遮蔽百分比窗口。

## 验证

- `cargo test -p tokenbuddy-official-quota`
- `cargo test -p tokenbuddy-core`
- `cargo test -p tokenbuddy-core --test phase7_official_quota`
- `cargo test -p tokenbuddy-desktop`
- `cargo test --workspace --all-targets`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `pnpm --filter @tokenbuddy/desktop build`
- `pnpm --filter @tokenbuddy/desktop lint`
- `pnpm --filter @tokenbuddy/desktop test`
- `pnpm --filter @tokenbuddy/desktop format:check`

## 剩余限制

- 当前依赖 Codex Home 中可读且未过期的文件型 OAuth 登录态；TokenBuddy 不刷新或写回 access/refresh token，过期后需要在 Codex/ChatGPT 中重新登录。
- 官方额度接口属于官方客户端使用的后端契约，响应字段变化时需要更新 parser；API Key 的 OpenAI API 用量不等同于 ChatGPT 订阅额度，因此仍保持 `Unavailable`。
- Windows 真机托盘、隐藏窗口持续采集及官方额度网络场景仍待 Windows CI/真机验收。
