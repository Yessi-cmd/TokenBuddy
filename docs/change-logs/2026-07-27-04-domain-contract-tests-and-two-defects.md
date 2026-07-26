# 2026-07-27-04 domain 契约测试，并修两处由此暴露的缺陷

## 目的

`crates/domain` 行覆盖率 68%，未覆盖的全是枚举投影与纯函数。补测试时把它们当作**契约**来钉
（而不是只为覆盖率跑一遍），随即暴露出两个真实缺陷。

## 缺陷一：`LauncherKind::CCSwitch` 的 serde 表示与存储表示不一致

- 存储写入与读回用的是 `as_str()` / `launcher_from_str()`，即 `cc_switch`。
- 前端 `LauncherKind` 联合类型声明的也是 `"cc_switch"`。
- 但 `#[serde(rename_all = "snake_case")]` 把 `CCSwitch` 转成 **`c_c_switch`**（连续大写各自
  成词）。于是任何经 CC-Switch 归因的 `UsageEvent` / `SessionRecord` / `ProviderRecord` 通过
  Tauri IPC 或 loopback API 发到前端时，`launcher` 的值都不在前端类型覆盖范围内。

修复：给该变体加 `#[serde(rename = "cc_switch")]`。存储层不经过 serde，因此**没有历史数据
需要迁移**，前端类型也无需改动——这是把实现修正到已声明的契约上。

新增测试遍历五个枚举的每个变体，断言 `as_str()`、`Display` 与 serde 三者一致，防止同类问题
再次出现（`ObserverProxy`、`ImportedDatabase` 等多词变体同样被覆盖）。

## 缺陷二：`delta_from` 对重复快照返回 `Some(全 0)`

函数名与文档都表示"没有增量就返回 `None`"，实际 `is_empty()` 只判断字段是否**缺失**，而重复
快照的差值是 `Some(0)`，于是返回了一个零 Token 的增量。调用方若照单记录，就会多出一条不代表
任何用量的事件、虚增请求数。

现有适配器没有踩到，是因为它们各自在调用前额外做了快照相等判断——共享工具本身不安全，防线
建在每个调用点上。

修复：`delta_from` 在差值全为零时也返回 `None`，并新增 `is_zero()` 与 `is_empty()` 区分
"报了但全是 0"和"根本没报"。生产行为不变（适配器本就拦住了），但未来的调用方不再需要自己
重建这道防线。

## 影响文件

- `crates/domain/src/lib.rs`：两处修复 + 新增 `is_zero()` + 8 个契约测试。

## 验证

- `cargo test -p tokenbuddy-domain`：11 项通过。
- `cargo fmt --check`、`cargo clippy --workspace --all-targets --all-features -D warnings`、
  `cargo test --workspace --all-targets`：通过（13 个套件）。
- 覆盖率：`crates/domain` 68.00% → 92.73%，workspace 82.94% → 83.70%。

## 遗留限制

- `delta_from` 目前在生产代码中无调用点（适配器用 `checked_delta` 加自有判断）。保留并修正
  它是因为它是公开 API 且语义已明确；后续可评估让适配器改用它，以消除重复的相等判断。
