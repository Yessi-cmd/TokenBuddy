# DONE

已完成的自主维护项，倒序排列。

## 2026-07-27

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
