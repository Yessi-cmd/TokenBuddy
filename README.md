# TokenBuddy

本地优先的 AI 编程 Token 观测工具。统一回答"Codex 和 Claude Code 到底用掉了多少 Token、
花在哪个会话、由哪个 Provider 和账号服务、以及这个数字有多可信"。

所有数据留在本机 SQLite，不上传、不依赖云端后端。产品与工程约束以
[`AI_Coding_Token_Observatory_PROJECT_SPEC.md`](AI_Coding_Token_Observatory_PROJECT_SPEC.md)
为准，代码注释按章节引用它（`spec §6.1`、`§14` 等）。

## 安装

从 [Releases](https://github.com/Yessi-cmd/TokenBuddy/releases) 下载：

- **Windows**：`.msi` 或 `-setup.exe`（x64）
- **macOS**：`.dmg`（Apple Silicon）

安装包**未做代码签名**：Windows 会弹 SmartScreen（"更多信息"→"仍要运行"），macOS 需在
"系统设置 → 隐私与安全性"中放行。Windows 上的托盘交互、开机自启与路径选择器尚未在真机
验证，CI 只保证能编译与通过测试。

## 它读什么

四个只读适配器，任何一个失败或缺失都不影响其余部分：

| 来源                | 读取内容                                                | 提供                                    |
| ------------------- | ------------------------------------------------------- | --------------------------------------- |
| Codex Session       | `<CODEX_HOME>/sessions/**/*.jsonl`、`auth.json`         | Token、会话、官方账号身份、官方额度窗口 |
| Claude Code Session | `<CLAUDE_HOME>/projects/**/*.jsonl`                     | Token、会话、父子 Agent 关系            |
| CC-Switch           | `~/.cc-switch/cc-switch.db`                             | 真实 Provider 与会话级归因              |
| Cockpit Tools       | `~/.antigravity_cockpit/codex_local_access_logs.sqlite` | 账号身份与账号活动时间窗                |

默认路径在 macOS / Linux 与 Windows 下各自解析，也可在设置页手工指定或用系统选择器挑选。

**CC-Switch 与 Cockpit 不产出 Token 事件。** 它们代理的正是 Codex / Claude Code 已经自己
记录的那些请求，按来源优先级（spec §6.1）会话日志排在代理日志之上，重复导入会把同一次调用
计两遍。它们的独特贡献是"谁真正服务了这次请求"——这是会话日志永远不写的信息。

第三方数据库一律以 `SQLITE_OPEN_READ_ONLY` 打开，读表前先探测 `sqlite_master`，绝不写入
它们的数据、配置或凭据。

## 运行方式

Tray-first：启动后只常驻一个后台采集 Core 并注册菜单栏 / 托盘图标，**不自动弹出窗口**。

```
菜单栏 / 托盘轻量弹窗 ┐
完整桌面面板         ├─→ 单实例 Core ─→ SQLite
本地网页面板         ┘
```

- 单击托盘图标：轻量弹窗，只读 Core 维护的 `QuickSummary`，不扫描日志、不跑聚合。
- 双击：完整面板。关闭窗口只是隐藏，采集继续；真正退出只能走托盘菜单。
- 本地网页面板按需启动，只绑定 `127.0.0.1` 与 `::1`，局域网访问不到。

三个入口共享同一个 Core、同一个数据库、同一套统计语义——不会因为多开一个面板就重复扫描或
重复计数。

## 数据与隐私

- 数据库：`com.tokenbuddy.desktop` 的应用数据目录下 `tokenbuddy.sqlite3`
  （macOS `~/Library/Application Support/`、Windows `%APPDATA%`）。
- **不保存**：提示词正文、模型回复、源代码、Authorization 头、Cookie、完整 API Key、
  OAuth Token、Refresh Token。
- 账号身份只以"每安装随机盐 + SHA-256"的指纹入库（spec §20.2），盐不进入设置、IPC、
  loopback API 或导出，因此复制走数据库也无法反查账号。
- 导出（CSV / JSON）不含原始 payload。

## 数字的可信度

两条贯穿全仓库的规则：

1. **缺失就是缺失。** Token、费用、额度、归因全部是 `Option`，源里没有的字段保持
   `Unavailable`，绝不写成 0。聚合时只要有一个事件缺该字段，总数就是 `Unavailable`——
   不会把部分和冒充完整总数。
2. **精度随值一起显示。** 每个事件分别记录 Token / 会话 / Provider / 账号四个维度的精度
   （`Verified > ExactSession > Correlated > Estimated > Unavailable`），由最弱环节定级。
   例如额度百分比来自官方响应但归属账号是按时间窗关联的，就记 `Correlated` 而非 `Verified`。

额度窗口与原始 Token 分开存放，任何时候都不会用百分比反推 Token 数（spec §8.4）。

## 开发

先决条件：Node.js 24+、pnpm 11+、Rust 1.93+，以及
[Tauri 2 的平台依赖](https://v2.tauri.app/start/prerequisites/)。

若 `pnpm` 不在 PATH 中（只装了 Node 而未启用 corepack 垫片），先执行一次
`corepack enable`；或改用 `scripts/` 下的脚本，它们会自行处理。

```sh
pnpm install
sh scripts/dev.sh          # 从源码运行（热更新）
sh scripts/build-app.sh    # 构建可安装的应用，并打印产物位置
sh scripts/run-app.sh      # 启动已构建的应用（--window 强制显示窗口）
```

对应的原始命令：`pnpm dev`、`pnpm build`、`pnpm build:web`。

### 入口在哪里

| 你要找的     | 位置                                                                          |
| ------------ | ----------------------------------------------------------------------------- |
| 程序入口     | `apps/desktop/src-tauri/src/main.rs` → `lib.rs` 的 `run()`                    |
| 界面入口     | `apps/desktop/src/main.tsx` → `App.tsx`                                       |
| 可执行文件   | `target/release/bundle/macos/TokenBuddy.app`（`pnpm build` 之后）             |
| 裸二进制     | `target/debug/tokenbuddy-desktop`（`cargo build` 之后，macOS 上不便直接双击） |
| 运行时数据库 | `~/Library/Application Support/com.tokenbuddy.desktop/tokenbuddy.sqlite3`     |

**启动后看不到窗口是正常的。** 应用是托盘优先的：macOS 上以 accessory 模式启动，
既不出现在 Dock 也不出现在应用切换器，只在菜单栏右侧有一个图标。单击它打开轻量摘要，
双击打开完整面板。想让主窗口直接出现，用 `TOKENBUDDY_DEBUG_SHOW_WINDOWS=1`（仅 debug 构建）。

提交前跑完整验证（与 CI 一致）：

```sh
pnpm format:check
pnpm lint
pnpm test
pnpm build:web
pnpm check:rust
```

覆盖率（需 `cargo install cargo-llvm-cov`）：

```sh
cargo llvm-cov --workspace --summary-only
```

调试开关：`TOKENBUDDY_DEBUG_SHOW_WINDOWS=1`（仅 debug 构建）让主窗口启动即可见；
`TOKENBUDDY_WEB_ROOT` 覆盖本地网页服务的静态文件目录。

## 仓库结构

```
crates/domain              共享类型与 UsageAdapter 契约，不依赖 Tauri 或 SQLite
crates/storage             SQLite 迁移、幂等批量导入、全部聚合查询
crates/adapters/*          四个只读来源适配器 + 共享的只读 SQLite 读取机制
crates/core                长驻 Core：持有数据库连接、导入线程、QuickSummary
apps/desktop/src-tauri     Tauri 外壳：托盘、窗口、命令、loopback HTTP 服务
apps/desktop/src           React SPA，桌面面板与网页面板共用
fixtures/                  脱敏解析样本，禁止使用真实数据
docs/change-logs/          每批改动的变更日志
```

依赖方向严格向下。前端通过 Tauri 命令或 loopback API 取数，永不直接打开 SQLite。

面向 AI 编程助手的工作约定见 [`AGENTS.md`](AGENTS.md) 与 [`CLAUDE.md`](CLAUDE.md)。
