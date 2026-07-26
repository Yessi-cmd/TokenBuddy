# 2026-07-26-24 Cockpit 多账号归因

## 目的

用户在 Windows 上用 Cockpit Tools 轮换三个账号（含一个 ChatGPT Plus）驱动 Codex。此前 Cockpit 适配器只把 `gateway_mode` 映射成 Provider，`request_logs` 里的 `account_id` / `email` 完全没读，三个账号既不会出现在账号页，也无法回答"每个号各用了多少 Token"。上一批次的保护措施让 Codex 用量在检测到轮换时保持 `Unavailable`，本批次补上真正的归因来源。

## 影响文件

- `crates/domain/src/lib.rs`：新增 `AccountActivityWindow`（账号、来源、适用 App、起止时间）；`ImportBatch` 新增 `account_windows`。
- `crates/storage/migrations/0006_account_activity_windows.sql`、`crates/storage/src/migrations.rs`：新增窗口表与 `(app, started_at, ended_at)` 索引。
- `crates/storage/src/lib.rs`：
  - `upsert_account_window` 持久化窗口；
  - `account_at` 按时间点查账号，命中两个及以上账号时返回 `None`；
  - `backfill_account_windows` 回填启动器扫描之前就已入库的事件；
  - 事件插入时的账号优先级改为：启动器会话归因 > 适配器已解析账号 > **时间窗匹配** > 从模型名推断的占位账号；窗口命中时精度写 `Correlated`；
  - `ImportStats` 新增 `attributed_account_events`。
- `crates/adapters/cockpit/src/lib.rs`：新增 `with_fingerprint_salt`；扫描 `request_logs` 时按 `account_id`（回退 `email`）聚合每个账号的请求时刻，产出账号行与活动窗口；账号 `provider_id` 记为 `openai`（Cockpit 是启动器，额度与方案属于上游）。
- `crates/core/src/lib.rs`：给 Cockpit 适配器注入指纹盐。

## 行为变化

- Cockpit 的每个账号成为独立账号行，显示邮箱（缺失时显示盐化指纹前 8 位），登录方式 `cockpit`，归到 OpenAI 之下。三个账号因此在账号页分开显示。
- Codex 用量事件按发生时刻落到当时实际服务它的那个 Cockpit 账号上，精度 `Correlated`；启动器扫描之前导入的历史事件会被回填，包括之前落在"会话日志占位账号"里的那些。
- 请求间隔超过 30 分钟即断开窗口（Cockpit 可能在这段静默里换了号），窗口前后各留 60 秒余量以吸收代理与 rollout 日志的时间戳偏差。
- 两个账号的窗口覆盖同一时刻时，事件保持在占位账号上而不是二选一。

## 关键取舍

- **宁可不归因，不可猜错。** 窗口重叠、时刻落在所有窗口之外、或没有指纹盐时，一律不写真实账号。
- **占位账号可被覆盖，真实账号不可。** 回填只改写 `NULL` 与 `auth_mode = 'session_log'` 的占位账号；其他来源已解析出的真实账号不动，与 Provider 归因的优先级规则一致。
- **Cockpit 仍然不产出 usage 事件**：这些请求 Codex 自己的 rollout 日志已经计过（§6.1）。Cockpit 只回答"谁服务了它"。
- 窗口只标注 `AppKind::Codex`：Cockpit 不代理 Claude Code。

## 验证

- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`cargo test --workspace --all-targets`：通过。
- 新增用例：`rotating_accounts_become_separate_accounts_with_their_own_activity_windows`、`accounts_stay_unavailable_without_a_fingerprint_salt`（Cockpit），`a_launcher_activity_window_attributes_events_by_time_and_refuses_when_ambiguous`（Storage，覆盖回填、插入时命中、重叠拒绝、窗口外拒绝）。

## 遗留限制

- **官方额度在轮换环境下仍不入库。** `QuotaSnapshot.account_id` 目前是必填的 `String`，适配器无法产出"待定归属"的额度行。要让每个账号显示自己的额度窗口，需要把该字段改为可选并在 storage 里走同一套时间窗解析——下一批次。
- Cockpit 的 `request_logs` 只有在 Codex 真正经由 Cockpit 本地代理时才有数据；直连的请求没有窗口，相关事件保持占位账号。
- 窗口断开阈值（30 分钟）与余量（60 秒）是基于代理日志与 rollout 日志时间戳关系的判断值，尚未用真实多账号数据标定。
- CC-Switch 也可能轮换 Codex 账号，但其 `proxy_request_logs` 的账号列尚未接入，目前只贡献 Provider 与会话级归因。
