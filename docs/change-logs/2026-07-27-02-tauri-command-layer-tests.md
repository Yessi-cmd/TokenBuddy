# 2026-07-27-02 Tauri 命令层测试

## 目的

`apps/desktop/src-tauri/src/lib.rs` 行覆盖率只有 10.93%，717 行未覆盖，占全仓库未覆盖行的
38%，是总覆盖率的最大拖累。此前的单元测试只触及托盘文案、弹窗定位等纯函数，**桌面面板真正
调用的 `#[tauri::command]` 层完全没有测试**——包括分页默认值、缺失来源的降级行为、导出格式
校验这些前端直接依赖的契约。

## 影响文件

- `apps/desktop/src-tauri/Cargo.toml`：dev-dependency 增加 `tauri` 的 `test` feature。这是
  已有依赖的 dev-only feature，不改变发布产物，也不是版本升级。
- `apps/desktop/src-tauri/src/lib.rs`：新增 `command_tests` 模块，用 Tauri mock runtime 在
  无窗口系统的环境下构造真实 `State<AppState>`（内含真实 Core + 脱敏 Codex fixture），
  直接驱动命令函数。

## 覆盖的契约

- 读取类命令返回 Core 拥有的视图：QuickSummary、Dashboard 默认今日窗口、模型分解、
  会话列表/详情、用量事件（全量与按会话）、来源、Provider、账号、额度。
- 未配置的来源返回空结果而非编造行：账号只有会话日志占位账号，额度为空。
- 不存在的会话详情返回 `None` 而不是错误。
- 导出覆盖 csv/json 两种格式，并断言未知格式被拒绝、导出不含原始 payload。
- 四个 detect 命令对"已配置"与"未找到"都给出显式结论，且支持显式路径覆盖。
- 四个 rescan 命令在同一 fixture 上幂等，未配置的来源不会让扫描按钮报错。
- 设置读取命令：未配置项保持 `None`，不伪装成已配置。
- 本地网页服务未启动时状态为停止，重复停止不报错。
- 目录选择器起始位置：目录用自身、文件用父目录、失效路径回落系统默认。

## 关键取舍

- **`update_app_settings`、`save_export`、`show_main_window`、两个选择器、`quit_tokenbuddy`
  未纳入**：它们绑定真实运行时（`AppHandle<Wry>`）。其中 `update_app_settings` 会调用
  `sync_autostart`，在测试机上注册登录项；`quit_tokenbuddy` 会结束测试进程。为可测性把它们
  改成对运行时泛型会牵动 `generate_handler!`，风险高于收益，故按原样保留并在测试模块顶部
  写明原因。

## 验证

- `cargo test -p tokenbuddy-desktop`：25 项通过。
- `cargo fmt --check`、`cargo clippy --workspace --all-targets --all-features -D warnings`、
  `cargo test --workspace --all-targets`：通过。
- 前端 `prettier --check`、`eslint --max-warnings 0`、`vitest run`、`tsc -b`：通过。
- 覆盖率（`cargo llvm-cov --workspace`）：`lib.rs` 10.93% → 49.34%，
  workspace 行覆盖率 77.63% → 82.83%。

## 遗留限制

- `lib.rs` 剩余未覆盖部分集中在 `run()`、托盘与窗口生命周期回调，这些需要真实运行时，
  只能靠真机验收覆盖。
- `main.rs` 覆盖率 0%（3 行入口），无实际意义。
