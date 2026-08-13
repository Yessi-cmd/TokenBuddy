# 2026-08-13-06 本地 Web API 防 CSRF/DNS-rebinding + 依赖漏洞升级

## 目的

落实本项目安全审计发现中的两项整改：

1. **F-01（中危）**：本地 loopback 网页面板 API 此前不校验 `Host`/`Origin`，任何网页都可用 `no-cors` + `text/plain` 体跨站改写设置（如 `data_retention_days: 1` 导致历史被清），或经 DNS rebinding 读取用量数据。为所有请求增加 Host 校验、为写请求增加 Origin 校验。
2. **F-04（低危）**：升级存在已知漏洞的第三方依赖。

## 影响文件

- `apps/desktop/src-tauri/src/web.rs` — `Request` 增加 `host`/`origin` 字段；`read_request` 解析 Host/Origin 头；`handle_connection` 执行 `request_allowed` 策略（拒绝时返回 403）；`raw_response` 增加 403 reason；新增 3 组测试。
- `pnpm-workspace.yaml` — 新增 `overrides` 与 `minimumReleaseAgeExclude`（pnpm 11 的 supply-chain 策略会拒绝发布未满最短时长的版本，需逐版本豁免）。
- `pnpm-lock.yaml` — `minimatch` 10.2.5→10.2.6、`brace-expansion` 4.0.4→5.0.9、`nanoid` 3.3.16→3.3.18、`postcss` 8.5.22→8.5.26。
- `Cargo.lock` — `event-listener` 5.4.1→5.4.2（RUSTSEC-2026-0221，并移除其 `concurrent-queue` 依赖）、`rand` 0.9.2→0.9.3（RUSTSEC-2026-0097 / GHSA-cq8v-f236-94qc）。

## 行为变化

- 网页面板现在拒绝 Host 非 loopback（127.0.0.1 / localhost / [::1]）的请求 → DNS rebinding 读取失效。
- 拒绝带非 loopback `Origin` 的 POST/PUT/PATCH/DELETE → 浏览器跨站写（含 text/plain 体的 JSON）失效。
- 不带 `Origin` 的本机客户端（curl、脚本、测试）不受影响；浏览器正常打开 `http://127.0.0.1:<port>` 使用面板不受影响。

## 验证

- `pnpm audit`：0 漏洞（此前 3 个：brace-expansion high、nanoid high、postcss moderate，均位于 eslint/vite 开发工具链）。
- `pnpm --filter @tokenbuddy/desktop format:check` / `lint` / `test`（58 个测试）/ `build`：全部通过。
- `cargo fmt --all -- --check`：通过（首轮提交时本机无 Rust 工具链、按仓库风格手工排版，CI 格式检查失败；随后按 `rust-toolchain.toml` 安装 1.97.1 + rustfmt，以其输出修正）。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`：通过（含新增测试代码）。
- `cargo test --workspace --exclude tokenbuddy-desktop`：全部通过（core/storage/adapters/otel-receiver 等 100+ 用例，覆盖 Cargo.lock 升级后的全链路）。
- 新增测试：`loopback_host_policy_accepts_only_loopback_hosts`、`request_policy_blocks_cross_origin_writes_and_rebinding_reads`、`cross_origin_and_rebinding_requests_are_refused_on_the_socket`。
- `Cargo.lock` 的版本号与校验和取自 crates.io 官方索引（`event-listener 5.4.2`、`rand 0.9.3`），依赖列表按索引逐项核对。

## 剩余限制

- 本机无 MSVC 链接器（GNU 工具链在 desktop crate 上触发 mingw 导出符号上限），`tokenbuddy-desktop` 的测试与最终链接未能在本地完成，由 CI（macOS + Windows，MSVC）验证。
- `rand 0.9.3` 仅覆盖 0.9 系列；glib/gtk-rs/unic-*/proc-macro-error 的 unmaintained 公告仅影响 Linux 构建链或构建期，macOS/Windows 目标不涉及，未处理。
- 审计其余发现（F-02 无 CSP、F-03 官方额度 raw_json 全量存储、F-05 OTel 无鉴权）留待后续批次。
