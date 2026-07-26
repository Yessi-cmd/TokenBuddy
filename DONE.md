# DONE

已完成的自主维护项，倒序排列。

## 2026-07-27

- **L1 · 前端覆盖率测量与补测，并修错误提示被抹掉的缺陷**（41.92% → 88.81%）
  接入 `@vitest/coverage-v8`。补 16 项传输契约测试（`api.ts` 原本因被整体 mock 而 0% 覆盖，
  它承载的正是"两条通道问同一件事"这个不变量）与 22 项面板行为测试。
  过程中发现：`DashboardView` 只有一个 error 槽位，总览加载成功时会 `setError(null)`，
  而扫描结束会立刻触发一次加载、另有 5 秒定时器——**扫描的部分失败提示在用户看到之前就被
  清空**，独立扫描以指名失败来源的设计因此形同虚设。拆成 `loadError` 与 `actionError`
  两个槽位后，加载只清自己的。补测筛选联动（指标卡与会话列表必须被同一组筛选收窄）、
  来源路径输入、会话选中、浏览器预览提示与导航链接（带修饰键的点击留给浏览器）。

- **M4 · 其余 crate 与桌面壳补全 162 项文档注释**（436 → 0）
  core 65、storage 49、四个适配器 59、桌面壳 4。文档写的是"为什么"而非重述签名：落库顺序
  的理由、拒绝更新版库文件的理由、首次导入为何同步、未出现在新设置中的来源会被清空、
  重复导入为何不产生事件。每个 crate 根开启 `#![warn(missing_docs)]`，配合 CI 的
  `clippy -D warnings` 使回退直接构建失败。

- **M1 · Core 配置面与查询面测试**（76.07% → 91.21%，workspace 达 85.92%）
  新增 `crates/core/tests/core_surface.rs`：设置改动无需重启即生效、四个来源路径可设可清、
  `update_app_settings` 一次性重配所有来源（未设的会被清空而非沿用）、启动器数据库只加归因
  不加 Token（CC-Switch 提供真实 Provider、Cockpit 提供账号，事件数仍为会话日志的 2 条）、
  dashboard/filtered/breakdown 三条查询在同一窗口下结果一致、摘要监听器在变化时被通知。
  core 的 dev-dependency 增加 rusqlite 用于构造两个启动器的只读 fixture。

- **M2 · domain 契约测试，并修两处缺陷**（68% → 92.73%）
  把枚举投影与纯函数当契约钉住，随即发现：(1) `LauncherKind::CCSwitch` 经 serde 发到前端是
  `c_c_switch`，与存储层和前端类型声明的 `cc_switch` 不一致，任何 CC-Switch 归因的事件在
  前端都落在类型之外——加显式 `rename` 修正，存储不走 serde 故无需迁移；(2) `delta_from`
  对重复快照返回 `Some(全 0)` 而非文档承诺的 `None`，会让调用方记录零 Token 事件、虚增请求数，
  现有适配器只是各自额外加了相等判断才没踩到。新增 `is_zero()` 区分"报了但为 0"与"根本没报"。

- **H4 · README 重写**
  原文只有先决条件与六条命令，读完不知道这个工具做什么。补上：安装入口与未签名/未真机验证
  的实情、四个适配器各读什么、为何 CC-Switch 与 Cockpit 不产出 Token 事件、Tray-first 的
  三入口共享一个 Core、数据库位置与不保存清单、"缺失就是缺失"与"精度随值显示"两条贯穿规则、
  覆盖率命令与调试开关、仓库结构。所有断言都对照代码核实（数据库路径、只读打开方式、
  crate 列表）。

- **H3 · domain 文档注释补全**（274 → 0）
  为共享词汇表的每个公开项写了文档：五个枚举的每个变体（含 §6.1 来源优先级与 §14 精度
  分级的语义）、`UsageEvent` 的四个分维度精度字段、`NormalizedUsage` 为何用 `Option`
  而非 0、`checked_delta` 为何拒绝下溢、`UsageTotals` 为何在任一事件缺字段时返回 `None`。
  末尾开启 `#![warn(missing_docs)]`，配合 `clippy -D warnings` 使新增未文档化的公开项
  直接构建失败。

- **H2 + M6 · 抽取只读 SQLite 读取机制与账号指纹**（commit 见下）
  六个逐字重复的 helper 收进新 crate `tokenbuddy-sqlite-source`，`fingerprint` 收进
  `domain::account_fingerprint`（它承载 §20.2 的隐私约定，与 `AccountRecord` 同处才只有一处
  定义）。只抽取"如何安全读陌生 SQLite"的机制，表名/列名/语义留在各适配器内，以满足
  AGENTS.md 的适配器隔离要求；`resolve_db_path` 因两边语义不同而保留。净 +55/−140 行，
  新 crate 自带 4 项测试（只读拒绝写入、缺失文件不创建、缺列/NULL 不退化成默认值、
  epoch 秒/毫秒同解且 0 不映射到 1970）。

- **L2 · 修复 loopback 服务丢弃迟到请求**（commit `8b1341e`）
  排查 macOS 偶发的 `Connection reset by peer`，发现不是测试不稳定而是真实缺陷：BSD 语义下
  accept 出的连接继承监听套接字的非阻塞标志，令读超时失效、迟到的请求被当作断开丢弃。
  accept 后显式恢复阻塞语义，并补了一个"先连接、150ms 后再发请求"的确定性回归测试
  （已验证移除修复必定失败）。浏览器面板同样受此缺陷影响，不只是测试。

- **H1 · Tauri 命令层测试**（commit `4418f9f`）
  用 Tauri mock runtime 在无窗口环境驱动 `#[tauri::command]` 层，覆盖读取命令、检测、
  重扫、导出格式校验、设置读取与选择器起始路径等前端直接依赖的契约。
  `lib.rs` 行覆盖率 10.93% → 49.34%，workspace 77.63% → 82.83%。
  绑定真实运行时的命令（会注册登录项或结束进程）按原样保留并写明原因。
