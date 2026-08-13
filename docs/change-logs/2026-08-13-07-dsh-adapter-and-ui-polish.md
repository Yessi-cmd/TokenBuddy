# 2026-08-13-07 DeepSeek Harness 数据源适配与面板 UI 优化

## 目的

1. **新增 DeepSeek Harness（DSH）数据源**：读取本机 DeepSeek Harness 的会话日志（`<DSH_HOME>/sessions/**/session.jsonl[.zstd]`），把每次模型调用的真实 token 用量（输入、缓存读/写、输出、推理）导入 TokenBuddy，使面板能统计运行在本机 DSH 中的智能体会话。
2. **UI 布局与合理性优化**：数据源区补齐 DSH 入口、设置页补上缺失的“数据保留周期”与 DSH 路径字段、会话/数据源/Providers 页的信息组织与状态可视化改进、应用名称统一本地化显示。

## 影响文件

- 新增 `crates/adapters/dsh-session`：只读解析 DSH JSONL 会话工件（明码与 Zstandard 压缩两种编码），头部行解析（session id/cwd/parentSession/origin/delegationDepth），`assistant/message` 的 `usage` 映射为统一 Token 语义，`request/context`/`request/header` 携带的 provider/model 作为路由上下文；稳定事件哈希、按文件 size+mtime 跳过、截断/轮转重置、未完成尾行重试、坏行跳过；`resource_id` 去除编码后缀使两种物理编码共享同一逻辑资源（编码切换后新旧文件去重而非重复计数）。默认路径 `~/.dsh`（或 `$DSH_HOME`），允许用户自定义。
- `crates/domain`：`AppKind` 新增 `DeepseekHarness`（持久化/序列化名 `deepseek_harness`）；`AppSettings` 新增 `dsh_home`。
- `crates/storage`：迁移 `0011_dsh_home.sql`；settings 读写入 `dsh_home`；`app_from_str` 与 Provider 推导匹配新 AppKind。
- `crates/core`：注册 DSH 适配器（descriptor/catalog、导入、检测、重扫、路径设置、文件监听、源错误降级）；`CoreConfig.with_dsh_home`。
- `apps/desktop/src-tauri`：`detect_dsh_path`/`rescan_dsh` 命令与 loopback `/api/detect-dsh`、`/api/rescan-dsh`；`web.rs` 筛选支持 `deepseek_harness`；启动配置注入 `default_dsh_home()`。
- `apps/desktop/src`：`api.ts`（AppKind、`dsh_home`、detect/rescan DSH）、`SettingsView`（DSH Home 目录选择 + 数据保留周期字段）、`DashboardView`（数据源区 DSH 行与检测、筛选新增 DeepSeek Harness、说明文案更新、按模型表格应用列本地化）、`SessionsView`（顶部计数行、总条数、空态文案）、`SourcesView`（健康状态彩色徽章）、`ProvidersView`（缓存命中率）、`Presentation`/`QuickSummaryView`（`appLabel` 本地化显示）、`styles.css`（健康徽章、会话计数行）。
- 新增 fixtures：`fixtures/dsh/simple_session.jsonl`、`fixtures/dsh/subagent.jsonl`（手工构造的脱敏样例，非真实数据）。
- 新增测试：DSH 适配器 11 项（映射、zstd 等价、幂等、增量、截断/轮转、子代理、坏行/尾行重试、正文不落库、无关文件忽略、缺失根降级、usage 语义）、Core 集成 `tests/phase9_dsh.rs`（导入、重扫幂等、跨重启幂等、源失败不阻断其他源）；前端新增 DSH 检测、设置保存（含保留周期回退）、应用标签 3 项回归测试。

## 行为变化

- 安装/升级后，Core 默认检测 `~/.dsh/sessions`（或 `DSH_HOME` 环境变量）并增量导入其中的 DSH 会话；用量精度为 `ExactSession`，Provider 由 DSH 自身的 `request/context` 路由给出（如 `deepseek-official`）并生成对应 Provider 记录。
- Prompt、completion、推理正文与工具参数从不进入领域模型；`raw_usage_json` 仅存数值字段（受既有 `save_request_metadata` 开关约束）。
- 设置页新增“DeepSeek Harness Home”与“数据保留周期（天）”字段；总览数据源区新增 DSH 检测行。
- 会话行与按模型表格中的 `deepseek_harness` 显示为“DeepSeek Harness”而非原始 id。

## 验证

- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`：通过。
- `cargo test --workspace --exclude tokenbuddy-desktop`：全部通过（含 DSH 适配器 11 项与 phase9 Core 集成 3 项；desktop 链接因本机无 MSVC 由 CI 覆盖）。
- `pnpm --filter @tokenbuddy/desktop format:check`/`lint`/`test`（61 项）/`build`：全部通过。
- 真实数据冒烟（只读、未入库）：解压本机 DSH 会话的首帧与全量扫描，头部、`assistant/message` 的 usage 字段名与 `request/context` 路由均与解析器预期一致（会话内 321 条 usage 记录可被导入）。

## 剩余限制

- 仅解析当前 DSH JSONL 格式（`SESSION_FORMAT_VERSION = 0`）；未来格式版本变化时按头部 `version` 直接跳过该文件并计入坏行。
- DSH 会话文件由后台轮询 + 文件监听驱动导入，压缩文件的增量以“整文件重解压 + 明文字节偏移”实现；超大单文件（>256 MiB 解压后）会拒绝并记为该源错误。
- Windows 托盘/自启动等真机验收仍归入未完成的 `Phase 4b 跨平台真机交互补验`。
