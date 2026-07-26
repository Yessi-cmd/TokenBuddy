# TODO

自主维护待办。每项标注优先级与影响范围。完成后移入 `DONE.md`。

## 基线（2026-07-27 首次扫描）

| 指标 | 当前 | 目标 |
|---|---|---|
| 行覆盖率（cargo llvm-cov, workspace） | 77.63% | ≥ 85% |
| clippy 警告（`-D warnings`） | 0 | 0 |
| `missing_docs` 待补项 | 436 | 0 |
| README 与代码一致 | 否 | 是 |

各文件行覆盖率：`apps/desktop/src-tauri/src/lib.rs` 10.93%、`crates/core` 67.77%、
`crates/domain` 66.84%、`web.rs` 73.44%、`cc-switch` 84.67%、`cockpit` 85.62%、
`storage` 89.53%、`claude-session` 88.62%、`codex-session` 92.95%。

代码中没有 TODO / FIXME / HACK 注释。

---

## 高优先级

- [ ] **H1 · 桌面壳覆盖率 10.93% → ≥60%**（影响：`apps/desktop/src-tauri/src/lib.rs`）
      717 行未覆盖，占全仓库未覆盖行的 38%，是总覆盖率的最大拖累。纯函数（路径归一化、
      弹窗定位、托盘文案）已有测试，缺的是 `#[tauri::command]` 包装层与窗口/托盘逻辑。
      方案：为 tauri 加 `test` feature 的 dev-dependency，用 mock runtime 覆盖命令层。

- [ ] **H2 · 抽取 cc-switch / cockpit 重复的只读 SQLite helper**（影响：两个适配器 + 新增内部 crate）
      `table_exists`、`column_set`、`column_names`、`string_col`、`int_col`、`epoch_to_utc`、
      `resolve_db_path` 七个函数在两个适配器里逐字重复。注意：只抽取"如何安全读一个陌生
      SQLite"的机制，各适配器的表名/列名/语义必须留在各自 crate 内，否则违反 AGENTS.md
      「一个适配器的 schema 变化不得影响另一个」。

- [ ] **H3 · `crates/domain` 补全 274 项文档注释**（影响：`crates/domain/src/lib.rs`）
      domain 是全仓库共享词汇表，也是唯一被所有 crate 依赖的公开接口。补完后开启
      `#![warn(missing_docs)]` 防止回退。

- [ ] **H4 · README 与当前代码状态对齐**（影响：`README.md`）
      现有 README 只有先决条件和 6 条命令。缺：产品做什么、四个适配器、Tray-first 运行方式、
      安装包下载入口、精度分级与缺失值语义、数据存放位置与隐私边界。

## 中优先级

- [ ] **M1 · `crates/core` 覆盖率 67.77% → ≥85%**（影响：`crates/core/src/lib.rs`）
      330 行未覆盖，集中在错误分支：适配器失败落库、设置更新、四个 detect 入口、
      watcher 路径推导、保留策略。

- [ ] **M2 · `crates/domain` 覆盖率 66.84% → ≥85%**（影响：`crates/domain/src/lib.rs`）
      64 行未覆盖，全是纯函数（枚举 as_str/Display、UsageTotals、NormalizedUsage 边界），
      成本最低的一块。

- [ ] **M3 · `web.rs` 覆盖率 73.44% → ≥85%**（影响：`apps/desktop/src-tauri/src/web.rs`）
      187 行未覆盖：404/405 分支、静态文件 content-type、请求体超限、畸形请求行。

- [ ] **M4 · storage / core / 适配器补全 162 项文档注释**（影响：5 个 crate）
      H3 之外剩余：storage 49、core 65、codex-session 14+6、claude-session 14、
      cockpit 13、cc-switch 12。

- [ ] **M5 · 拆分超长函数**（影响：4 个文件）
      `codex-session::import_file` 256 行、`claude-session::import_file` 203 行、
      `web::route_request_with_autostart` 186 行、`desktop::run` 131 行。
      按职责切分，不改变对外行为。

- [ ] **M6 · 合并重复的 `fingerprint` 实现**（影响：`codex-session/account.rs`、`cockpit`）
      两处逐字相同的盐化 SHA-256。归入 H2 的内部 crate 或 domain。

## 低优先级

- [ ] **L1 · 前端覆盖率测量与补测**（影响：`apps/desktop`）
      1900 行的 `App.tsx` 只有 12 个用例，且没有覆盖率工具。需要新增
      `@vitest/coverage-v8` dev 依赖（新增而非升级）。

- [ ] **L2 · 修复 loopback 测试偶发失败**（影响：`web.rs` 测试）
      macOS CI 上 `local_api_serves_core_data_over_both_loopback_families` 与
      `quick_summary_http_p95_stays_within_lightweight_entry_budget` 偶发
      `Connection reset by peer`，重跑即过。

- [ ] **L3 · `now()` 在 5 处重复定义**（影响：5 个 crate）
      各 3 行，抽取收益低于引入耦合的代价，暂列观察。

- [ ] **L4 · 第二轮更严标准**（在以上完成后）
      开启 `clippy::pedantic` 逐条评估；覆盖率目标提到 90%。
