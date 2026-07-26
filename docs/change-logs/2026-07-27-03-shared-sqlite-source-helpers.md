# 2026-07-27-03 抽取只读 SQLite 读取机制与账号指纹

## 目的

CC-Switch 与 Cockpit 两个只读适配器里有六个逐字相同的函数（`table_exists`、`column_set`、
`column_names`、`string_col`、`int_col`、`epoch_to_utc`），以及两处相同的只读打开方式；
`fingerprint` 又在 Cockpit 与 Codex 账号模块里各写了一遍。重复实现意味着修一个 bug 要记得
改另一处——例如"epoch 为 0 表示未记录、不能当成 1970"这条判断，就同时躺在四个地方。

## 影响文件

- `crates/adapters/sqlite-source/`（新增 crate `tokenbuddy-sqlite-source`）：
  `open_read_only`、`table_exists`、`column_set`、`column_names`、`string_col`、`int_col`、
  `float_col`、`epoch_to_utc`。
- `Cargo.toml`：注册新成员与 workspace 依赖。
- `crates/adapters/cc-switch/`、`crates/adapters/cockpit/`：删除本地副本，改用共享 crate。
- `crates/domain/src/lib.rs`：新增 `account_fingerprint(salt, secret)`，并移除两处重复实现；
  domain 增加 `sha2` 依赖，Cockpit 去掉不再需要的 `sha2`。

净变化：+55 / −140 行。

## 边界

**只抽取"如何安全地读一个陌生 SQLite"的机制，不抽取任何 schema 知识。** 表名、列名、
以及这些值的含义全部留在各自适配器内——这正是 AGENTS.md「一个适配器的 schema 变化或失败
不得影响另一个」所要求的。`resolve_db_path` 因此**没有**被抽取：CC-Switch 只需在目录后拼
文件名，Cockpit 还要接受 `~/.antigravity_cockpit` 的父目录，两者语义不同。

`account_fingerprint` 放进 domain 而不是 sqlite-source：它承载的是 §20.2 的隐私约定
（原始账号 id / API Key / OAuth token 绝不入库），与 `AccountRecord` 放在一起才能让这条
约定只有一处定义；且 Codex 账号模块并不读 SQLite。

## 验证

- 新 crate 自带 4 项测试：只读打开确实拒绝写入、打开缺失文件不会创建文件、按名取列在列缺失
  / 值为 NULL / 类型不符时一律返回 `None`（不退化成默认值）、epoch 秒与毫秒解析为同一时刻
  且 0 与负数不映射到 1970。
- `cargo fmt --check`、`cargo clippy --workspace --all-targets --all-features -D warnings`、
  `cargo test --workspace --all-targets`：通过（13 个测试套件）。
- 两个适配器的原有测试全部保持通过，未修改任何断言。

## 遗留限制

- `now()` 仍在五个 crate 里各定义一次（各 3 行）。抽取它需要所有 crate 依赖一个时间工具，
  收益低于耦合代价，暂不处理（TODO 中的 L3）。
