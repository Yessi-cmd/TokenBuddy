# TODO

自主维护待办。每项标注优先级与影响范围。完成后移入 `DONE.md`。

## 基线（2026-07-27 首次扫描）

| 指标 | 当前 | 目标 |
|---|---|---|
| 行覆盖率（cargo llvm-cov, workspace） | 77.63% → **82.83%** | ≥ 85% |
| clippy 警告（`-D warnings`） | 0 | 0 |
| `missing_docs` 待补项 | 436 | 0 |
| README 与代码一致 | 否 | 是 |

各文件行覆盖率：`apps/desktop/src-tauri/src/lib.rs` 10.93%、`crates/core` 67.77%、
`crates/domain` 66.84%、`web.rs` 73.44%、`cc-switch` 84.67%、`cockpit` 85.62%、
`storage` 89.53%、`claude-session` 88.62%、`codex-session` 92.95%。

代码中没有 TODO / FIXME / HACK 注释。

---

## 高优先级

- [x] **H1 · 桌面壳覆盖率 10.93% → 49.34%** — 见 DONE.md。剩余未覆盖集中在 `run()` 与
      托盘/窗口生命周期回调，需真实运行时，只能靠真机验收。

- [x] **H2 · 抽取只读 SQLite 读取机制** — 见 DONE.md。新 crate `tokenbuddy-sqlite-source`，
      +55/−140 行。`resolve_db_path` 两边语义不同，按设计保留在各自适配器。

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

- [x] **M6 · 合并重复的 `fingerprint`** — 见 DONE.md。收敛为 `domain::account_fingerprint`。

## 低优先级

- [ ] **L1 · 前端覆盖率测量与补测**（影响：`apps/desktop`）
      1900 行的 `App.tsx` 只有 12 个用例，且没有覆盖率工具。需要新增
      `@vitest/coverage-v8` dev 依赖（新增而非升级）。

- [x] **L2 · loopback 偶发失败** — 见 DONE.md。是真实缺陷而非测试问题，已修复并加回归测试。

- [ ] **L3 · `now()` 在 5 处重复定义**（影响：5 个 crate）
      各 3 行，抽取收益低于引入耦合的代价，暂列观察。

- [ ] **L4 · 第二轮更严标准**（在以上完成后）
      开启 `clippy::pedantic` 逐条评估；覆盖率目标提到 90%。
