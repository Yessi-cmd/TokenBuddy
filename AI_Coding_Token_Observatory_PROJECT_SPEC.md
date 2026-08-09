# TokenBuddy

> 本文件面向 Codex App / Codex CLI / Claude Code 等编程智能体使用。
> 目标是作为项目根目录中的长期技术规格、实施约束和验收依据。
> 智能体在修改代码前应先完整阅读本文件，不得把本文当作产品宣传材料。

---

## 0. 给 Codex 的执行指令

你正在实现一个跨平台桌面应用：**TokenBuddy**。

该应用用于统一统计和展示以下工具的 Token 使用情况：

- Codex App
- Codex CLI
- Claude Code
- CC Switch
- Cockpit Tools
- 第三方 OpenAI-compatible API
- 第三方 Anthropic-compatible API

必须遵守以下执行原则：

1. 优先实现只读观测，不要在第一版强制引入本地代理。
2. 不要修改 CC Switch 或 Cockpit Tools 的数据库和配置文件。
3. 对所有外部数据源使用 Adapter 模式隔离。
4. 无法确认的数据必须标记为 `Unavailable` 或 `Estimated`，禁止用 `0` 冒充缺失值。
5. 服务端返回的 usage 优先级高于本地 Tokenizer 估算。
6. 不保存完整 API Key、OAuth Token、Refresh Token。
7. macOS 和 Windows 必须共用同一套核心代码。
8. 代理模式必须是可选功能，不能成为应用启动和统计的前置条件。
9. 任何可能阻断 Codex 或 Claude Code 正常工作的改动，都必须有旁路和降级方案。
10. 每完成一个里程碑，更新本文件末尾的实施状态。

---

# 1. 项目目标

## 1.1 核心目标

构建一个本地优先、跨平台的 AI 编程 Token 观测工具，统一回答以下问题：

1. 官方账号今天、当前窗口、本周用了多少额度。
2. 第三方 API Key 实际消耗了多少输入、输出、缓存和费用。
3. 每个 Codex / Claude Code 会话消耗了多少 Token。
4. 哪些 Token 来自主 Agent、子 Agent、辅助请求或工具调用。
5. 缓存命中率是多少。
6. 哪个 Provider、账号、模型或项目消耗异常。
7. 数据来自哪里，精度是多少。

## 1.2 支持平台

第一优先级：

- macOS 13+
- Windows 10/11

后续可选：

- Linux

## 1.3 支持对象

### 必须支持

- Codex App
- Codex CLI
- Claude Code

### 兼容支持

- CC Switch
- Cockpit Tools
- OpenCode

### 后续支持

- Gemini CLI
- Cursor Agent
- 其他 AI Coding 工具

---

# 2. 非目标

第一版明确不做以下功能：

1. 不重新实现完整的 CC Switch。
2. 不重新实现 Cockpit Tools 的多账号 OAuth 管理。
3. 不尝试破解或逆向官方订阅计费算法。
4. 不通过 HTTPS MITM 抓取所有系统流量。
5. 不强制用户安装根证书。
6. 不保存用户提示词或源代码正文作为默认行为。
7. 不声称能在上游不返回 usage 时精确计算服务端缓存命中。
8. 不在第一版实现复杂的模型故障转移和负载均衡。
9. 不依赖云端后端。
10. 不默认上传任何遥测数据。

---

# 3. 核心产品定位

本项目不是账号切换器，而是统一观测层。

```text
CC Switch / Cockpit Tools
    负责：账号、Provider、路由、代理、切换

TokenBuddy
    负责：Token、额度、会话、请求、缓存、费用、精度
```

应用必须能够：

- 独立运行，不依赖 CC Switch 或 Cockpit 常驻。
- 在检测到 CC Switch / Cockpit 时读取其公开或只读数据。
- 在用户未安装上述工具时仍可统计 Codex 与 Claude Code。

---

# 4. 总体架构

```text
┌──────────────────────────────────────────────────────┐
│                    Desktop UI                        │
│ React + TypeScript + Tauri 2                         │
├──────────────────────────────────────────────────────┤
│                    Application Core                  │
│ Rust                                                  │
│                                                      │
│  Adapter Manager                                     │
│  Event Normalizer                                    │
│  Correlation Engine                                  │
│  Precision Evaluator                                 │
│  Cost Calculator                                     │
│  Import Scheduler                                    │
│  File Watcher                                        │
│  Optional Local Proxy                                │
├──────────────────────────────────────────────────────┤
│                    Local Storage                     │
│ SQLite + OS Keychain                                 │
├──────────────────────────────────────────────────────┤
│                    Data Sources                      │
│ Codex JSONL / Codex OTel / Claude JSONL / Claude OTel│
│ CC Switch DB / Cockpit API or DB / Proxy usage       │
└──────────────────────────────────────────────────────┘
```

## 4.1 Tray-first 展示架构（MVP 硬性要求）

TokenBuddy 默认采用 Tray-first（托盘优先）运行方式。应用启动后只启动一个后台采集 Core 并注册系统入口，不自动弹出完整桌面面板；完整桌面面板和本地网页面板均按需打开。

```text
macOS 菜单栏轻量弹窗 ──┐
Windows 系统托盘轻量弹窗 ─┤
完整桌面面板 ───────────┤──> 单实例 Rust Core ──> SQLite
本地网页面板 ───────────┘
```

四个入口必须共享：

- 同一个后台采集 Core。
- 同一个 SQLite 数据库。
- 同一套 Token、额度、精度和缺失值统计语义。
- 同一套只读查询服务和 `QuickSummary` 快速摘要。

禁止每个面板自行扫描 Codex 或 Claude Code 日志。这样会造成重复统计、文件锁竞争和不必要的资源消耗。所有文件监听、增量导入、归一化、聚合和数据库写入都由 Core 负责，面板只通过 Tauri IPC 或本地 HTTP API 查询数据。

默认启动流程：

```text
启动 TokenBuddy
    ↓
启动单实例后台采集 Core
    ↓
注册 macOS 菜单栏图标 / Windows 系统托盘图标
    ↓
开始监听和增量导入 Token 数据
    ↓
不弹出完整窗口
```

---

# 5. 技术栈

## 5.1 桌面框架

使用：

- Tauri 2
- React
- TypeScript
- Vite
- Rust

理由：

- 同一代码库支持 macOS 和 Windows。
- Rust 适合文件监听、SQLite、OTLP Receiver、本地代理。
- 资源占用低于 Electron。
- 系统托盘、macOS 菜单栏、自动启动和原生路径访问较成熟。
- 同一个 React SPA 同时服务完整桌面面板和本地网页面板；两者只区分数据访问通道，不复制业务逻辑或统计实现。

本地网页面板通过 Rust Core 提供的 loopback HTTP API 访问数据，不直接打开 SQLite，也不建立第二套采集或聚合管线。

## 5.2 数据库

使用 SQLite。

Rust 推荐库：

- `sqlx`，或
- `rusqlite`

优先选择 `sqlx`，因为：

- 支持迁移。
- 异步接口清晰。
- 类型约束较好。

## 5.3 前端状态

推荐：

- TanStack Query：后端数据查询与缓存
- Zustand：少量本地 UI 状态
- ECharts 或 Recharts：图表

## 5.4 文件监听

Rust：

- `notify`

需要兼容：

- 文件追加
- 文件轮转
- 文件重命名
- 应用休眠恢复
- Windows 文件锁

## 5.5 本地安全存储

API Key 如必须由本项目托管，使用：

- macOS Keychain
- Windows Credential Manager

通过 Tauri 插件或 Rust keyring 库访问。

SQLite 中只保存：

- `credential_id`
- Provider 名称
- Key 指纹
- 非敏感配置

---

# 6. 数据采集策略

## 6.1 数据源优先级

从高到低：

1. 上游 API 最终响应中的真实 usage
2. OpenTelemetry 原生事件
3. Codex / Claude Code Session 日志
4. CC Switch / Cockpit 的代理日志
5. 本地 Tokenizer 估算

## 6.2 第一版数据流

```text
Codex JSONL ──────────────┐
Codex OTel ───────────────┤
Claude JSONL ─────────────┤
Claude OTel ──────────────┤
CC Switch DB（只读）──────┤──> Normalizer -> SQLite -> UI
Cockpit 数据（只读）──────┤
官方额度数据 ─────────────┘
```

## 6.3 第二版数据流

```text
Codex / Claude Code
        │
        ▼
Optional Local Metering Proxy
        │
        ▼
第三方 API

Proxy usage + OTel + Session JSONL
        │
        ▼
Correlation Engine
```

---

# 7. Adapter 设计

所有数据源必须实现统一接口。

```rust
pub trait UsageAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;

    async fn detect(&self) -> Result<DetectionResult>;
    async fn import_history(&self, cursor: Option<ImportCursor>)
        -> Result<ImportBatch>;
    async fn start_watch(&self, sink: EventSink) -> Result<WatcherHandle>;
    async fn health(&self) -> Result<SourceHealth>;
}
```

必须实现以下 Adapter：

```text
CodexSessionAdapter
CodexOtelAdapter
ClaudeSessionAdapter
ClaudeOtelAdapter
OpenCodeAdapter
CCSwitchAdapter
CockpitAdapter
OfficialQuotaAdapter
OpenAIProxyAdapter        // v2
AnthropicProxyAdapter     // v2
```

每个 Adapter 必须：

- 独立处理 Schema 变化。
- 不向 UI 暴露原始 JSON 结构。
- 为原始事件计算稳定哈希。
- 支持重复导入去重。
- 报告自身健康状态。

---

# 8. Codex 适配

## 8.1 默认路径

macOS / Linux：

```text
~/.codex/sessions/
~/.codex/config.toml
~/.codex/auth.json
```

Windows：

```text
%USERPROFILE%\.codex\sessions\
%USERPROFILE%\.codex\config.toml
%USERPROFILE%\.codex\auth.json
```

必须允许用户自定义 Codex Home。

## 8.2 Session JSONL

目标字段：

```text
session_id
conversation_id
timestamp
project_path
model
provider
input_tokens
cached_input_tokens
output_tokens
reasoning_output_tokens
total_tokens
```

解析要求：

1. 支持逐行流式读取，禁止一次性加载大文件。
2. 保存每个文件的 inode/path、size、mtime、last_offset。
3. 从 last_offset 增量读取。
4. 检测文件截断和轮转。
5. 对累计快照做差或去重。
6. 子 Agent 继承父历史时禁止重复统计。
7. 原始事件哈希用于幂等导入。

## 8.3 Codex OTel

用于获取：

- conversation ID
- 请求级 Token
- 模型
- 认证模式
- API 事件

OTel Receiver 第一版只需要支持：

- OTLP HTTP protobuf
- localhost

后续再支持：

- OTLP gRPC

## 8.4 官方额度

额度必须作为独立数据类型保存：

```text
quota_window_type
used_percent
remaining_percent
reset_at
plan
credits
```

禁止用原始 Token 反推官方订阅窗口消耗。

---

# 9. Claude Code 适配

## 9.1 默认路径

macOS / Linux：

```text
~/.claude/projects/
```

Windows：

```text
%USERPROFILE%\.claude\projects\
```

必须允许用户自定义 Claude Home。

## 9.2 Session JSONL

需要导入：

- session ID
- 项目目录
- 消息时间
- 模型
- Token usage
- 子 Agent 标识
- 会话标题或可推导摘要

Claude Code 本地日志 Schema 可能变化，因此必须使用版本化解析器：

```text
ClaudeParserV1
ClaudeParserV2
ClaudeParserFallback
```

Fallback 解析器只提取稳定字段，不得猜测未知字段。

## 9.3 Claude OTel

优先采集：

```text
session.id
model
input_tokens
output_tokens
cache_read_tokens
cache_creation_tokens
query_source
agent_name
skill_name
plugin_name
mcp_server_name
mcp_tool_name
```

OTel 是 Claude Code 请求归属和主/子 Agent 分析的首选来源。

---

# 10. CC Switch 适配

## 10.1 原则

- 只读。
- 不写入数据库。
- 不依赖 CC Switch 正在运行。
- 不把 CC Switch 当作唯一 Token 来源。

## 10.2 默认位置

通常位于：

```text
~/.cc-switch/cc-switch.db
~/.cc-switch/settings.json
```

但用户可能修改数据目录，因此必须：

1. 自动检测常见位置。
2. 读取设置中的自定义目录。
3. 允许用户手动选择。

## 10.3 需要读取的数据

可能包括：

```text
providers
provider_endpoints
proxy_config
proxy_request_logs
provider_health
model_pricing
settings
```

用途：

- Provider 显示名称
- 上游 URL
- 当前激活 Provider
- 请求级 usage
- 模型价格
- 请求状态

## 10.4 数据库访问

必须使用只读模式：

```text
file:path/to/cc-switch.db?mode=ro
```

对 Schema 变化：

- 先读取 `sqlite_master`
- 检测表和列是否存在
- 按版本做兼容映射
- 不允许硬编码后直接崩溃

---

# 11. Cockpit Tools 适配

## 11.1 原则

优先级：

1. 本地公开接口
2. 导出日志
3. 只读数据库

禁止优先逆向内部数据库格式。

## 11.2 目标数据

```text
account_alias
account_fingerprint
plan
hourly_quota
weekly_quota
request_id
model
usage
status
provider
upstream_url
```

## 11.3 降级策略

当 Cockpit 未提供稳定接口时：

- 仍使用 Codex Session / OTel 获取会话 Token。
- Cockpit 只提供 Provider 和账号上下文。
- 关联失败时标记为 `Correlated`，不能标记为 `Verified`。

---

# 12. 可选本地显式代理

## 12.1 是否必要

第一版不必要。

第二版用于解决：

- 第三方 Provider 精确归属
- API Key 精确归属
- 上游真实 usage
- 上游实际响应模型
- 请求状态和延迟
- 服务端返回费用

## 12.2 代理模式

```text
Codex / Claude Code
        │
        ▼
127.0.0.1:<dynamic_port>
        │
        ▼
第三方 API URL
```

只监听：

```text
127.0.0.1
::1
```

禁止默认监听：

```text
0.0.0.0
```

## 12.3 不使用 MITM

禁止：

- 安装根证书
- 解密系统全部 HTTPS 流量
- 修改系统全局代理作为核心方案

只允许客户端显式配置 `base_url` 指向本地代理。

## 12.4 协议支持

必须支持：

### OpenAI-compatible

- `/v1/responses`
- SSE 流
- 非流式 JSON

后续可选：

- `/v1/chat/completions`

### Anthropic-compatible

- `/v1/messages`
- SSE 流
- 非流式 JSON

## 12.5 代理实现约束

1. 不缓冲完整响应后再返回。
2. 流式数据必须边接收边转发。
3. 同时复制并解析最终 usage。
4. 正确处理客户端取消。
5. 正确处理超时。
6. 记录重试，但不能把一次请求重复计费。
7. 不记录请求正文，除非用户显式开启调试模式。
8. Header 日志必须过滤 Authorization、Cookie、API Key。
9. 上游不返回 usage 时标记 `Unavailable`。
10. 代理崩溃不能导致配置永久损坏。

---

# 13. Token 统一语义

统一字段：

```text
input_tokens_total
input_tokens_uncached
cache_read_tokens
cache_write_tokens
output_tokens_total
reasoning_tokens
visible_output_tokens
provider_reported_total
```

## 13.1 OpenAI 语义

常见关系：

```text
input_tokens_total 包含 cache_read_tokens
input_tokens_uncached = input_tokens_total - cache_read_tokens
reasoning_tokens 通常是 output_tokens_total 的子集
visible_output_tokens = output_tokens_total - reasoning_tokens
```

## 13.2 Anthropic 语义

常见字段独立：

```text
input_tokens
cache_creation_input_tokens
cache_read_input_tokens
output_tokens
```

统一映射：

```text
input_tokens_uncached = input_tokens
cache_write_tokens = cache_creation_input_tokens
cache_read_tokens = cache_read_input_tokens
input_tokens_total = input + cache_write + cache_read
```

## 13.3 原始字段保留

数据库必须保留：

```text
raw_usage_json
usage_schema
provider_family
```

原因：不同 Provider 对 input 的定义可能不同，不能只保存统一结果后丢失原始语义。

---

# 14. 精度分级

定义：

```rust
pub enum PrecisionLevel {
    Verified,
    ExactSession,
    Correlated,
    Estimated,
    Unavailable,
}
```

含义：

## Verified

数据直接来自：

- 上游 API usage
- 官方 OTel
- 官方额度接口

## ExactSession

Token 精确且会话归属明确，来源为稳定 Session 日志。

## Correlated

Token 本身精确，但 Provider、账号或会话通过时间窗口和模型关联。

## Estimated

通过 Tokenizer、价格表或其他规则估算。

## Unavailable

数据源没有该字段。

UI 必须显示精度徽标。

禁止：

- 将 Unavailable 显示为 0
- 将 Estimated 显示成 Verified
- 隐藏关联失败

---

# 15. 统一数据模型

## 15.1 sources

```sql
CREATE TABLE sources (
    id TEXT PRIMARY KEY,
    adapter_type TEXT NOT NULL,
    display_name TEXT NOT NULL,
    path_or_endpoint TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    detected_version TEXT,
    health_status TEXT,
    last_success_at TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

## 15.2 providers

```sql
CREATE TABLE providers (
    id TEXT PRIMARY KEY,
    provider_family TEXT NOT NULL,
    display_name TEXT NOT NULL,
    upstream_url TEXT,
    launcher TEXT,
    source_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

## 15.3 accounts

```sql
CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    display_name TEXT,
    account_fingerprint TEXT NOT NULL,
    auth_mode TEXT NOT NULL,
    plan TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

## 15.4 sessions

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    external_session_id TEXT,
    parent_session_id TEXT,
    app TEXT NOT NULL,
    launcher TEXT,
    project_path TEXT,
    title TEXT,
    started_at TEXT,
    ended_at TEXT,
    source_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

## 15.5 usage_events

```sql
CREATE TABLE usage_events (
    id TEXT PRIMARY KEY,
    occurred_at TEXT NOT NULL,
    app TEXT NOT NULL,
    launcher TEXT,
    ingest_source TEXT NOT NULL,
    source_id TEXT NOT NULL,
    provider_id TEXT,
    account_id TEXT,
    session_id TEXT,
    parent_session_id TEXT,
    request_id TEXT,
    response_id TEXT,
    model TEXT,
    query_source TEXT,

    input_tokens_total INTEGER,
    input_tokens_uncached INTEGER,
    cache_read_tokens INTEGER,
    cache_write_tokens INTEGER,
    output_tokens_total INTEGER,
    reasoning_tokens INTEGER,
    visible_output_tokens INTEGER,

    provider_reported_cost REAL,
    estimated_cost REAL,
    currency TEXT,

    http_status INTEGER,
    latency_ms INTEGER,
    success INTEGER,

    precision_token TEXT NOT NULL,
    precision_session TEXT NOT NULL,
    precision_provider TEXT NOT NULL,
    precision_account TEXT NOT NULL,

    raw_event_hash TEXT NOT NULL UNIQUE,
    raw_usage_json TEXT,
    created_at TEXT NOT NULL
);
```

## 15.6 quota_snapshots

```sql
CREATE TABLE quota_snapshots (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    captured_at TEXT NOT NULL,
    window_type TEXT NOT NULL,
    used_percent REAL,
    remaining_percent REAL,
    reset_at TEXT,
    credits_remaining REAL,
    precision TEXT NOT NULL,
    raw_json TEXT
);
```

## 15.7 import_cursors

```sql
CREATE TABLE import_cursors (
    source_id TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    file_size INTEGER,
    modified_at TEXT,
    byte_offset INTEGER,
    content_hash TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (source_id, resource_id)
);
```

---

# 16. 去重策略

每个事件生成 `raw_event_hash`。

优先组成字段：

```text
source_id
external_request_id
external_response_id
external_session_id
timestamp
model
usage values
```

如果没有 Request ID：

```text
SHA256(
    source_id +
    file_path +
    byte_offset +
    normalized_event_json
)
```

不得仅用 timestamp 去重。

## 16.1 累计快照

若日志提供累计 Token：

- 与前一个有效快照做差。
- 累计值不变时忽略。
- 累计值回退时视为新轮次或文件重置。
- 负差值禁止写入数据库。

## 16.2 子 Agent 历史复制

检测方式：

- task_started 前的继承记录不计入子 Agent 新消耗。
- 相同 request/response ID 只保留一次。
- 相同累计快照只保留一次。

---

# 17. 关联引擎

目标：把代理请求、OTel 事件、Session 日志关联为同一次模型调用。

## 17.1 强关联

满足任一条件：

- request_id 相同
- response_id 相同
- conversation_id + sequence_id 相同
- 显式注入 observer_session_id

精度：`Verified`

## 17.2 中关联

组合匹配：

```text
时间差 <= 3 秒
模型相同
输入 Token 接近
输出 Token 相同或接近
应用相同
```

精度：`Correlated`

## 17.3 弱关联

仅按：

```text
时间窗口
进程 PID
工作目录
模型
```

精度不得高于 `Correlated`。

## 17.4 冲突处理

多个候选同时满足时：

- 不自动强行合并。
- 记录 correlation_conflict。
- UI 标记“归属不确定”。

---

# 18. 成本计算

成本分为：

```text
provider_reported_cost
estimated_cost
```

优先显示 `provider_reported_cost`。

## 18.1 价格表

价格表字段：

```text
provider
model
valid_from
valid_to
uncached_input_price
cache_read_price
cache_write_price
output_price
reasoning_price
currency
source
```

不得只按模型名全局定价，因为不同第三方中转价格不同。

## 18.2 价格缺失

价格缺失时：

- Token 仍正常显示。
- 成本显示 N/A。
- 不默认套用 OpenAI 或 Anthropic 官方价格。

---

# 19. UI 设计

## 19.1 总览页

展示：

```text
今日输入
今日缓存读取
今日缓存写入
今日输出
今日推理
缓存命中率
估算/实际费用
官方额度窗口
```

支持筛选：

- 日期
- 应用
- Provider
- 账号
- 模型
- 项目
- 精度

## 19.2 会话页

列表列：

```text
标题
应用
项目
Provider
账号
模型
输入
缓存读取
缓存写入
输出
推理
命中率
费用
精度
开始时间
```

详情页：

- 请求时间线
- 主 Agent / 子 Agent
- 每轮 Token
- Provider 切换点
- Compaction 事件
- 失败请求
- 重试

## 19.3 Provider 页

展示：

- Provider 名称
- 上游 URL
- 账号指纹
- 请求数量
- 成功率
- 平均延迟
- Token
- 费用
- 缓存命中率

## 19.4 数据源页

展示所有 Adapter：

```text
状态
检测路径
版本
最近导入
最近错误
已导入事件数
精度能力
```

## 19.5 设置页

包括：

- Codex Home
- Claude Home
- OpenCode 数据目录或 `opencode.db` 路径
- CC Switch DB 路径
- Cockpit 接口或数据路径
- OTel 端口
- 是否自动启动
- 是否允许代理模式
- 是否保存请求元数据
- 数据保留周期

## 19.6 Tray-first 入口与运行行为

默认运行方式必须是后台常驻、按需展示：

| 功能 | 默认状态 |
|---|---|
| 后台采集 Core | 开启 |
| macOS 菜单栏 / Windows 系统托盘 | 开启 |
| 轻量弹窗 | 点击时打开 |
| 完整桌面面板 | 按需打开 |
| 本地网页服务 | 按需启动 |
| 开机自启 | 安装引导时询问 |
| 本地代理 | 关闭 |

启动 TokenBuddy 后，Core 负责单实例初始化、数据源发现、文件监听、增量导入和摘要维护；应用不得自动弹出完整桌面窗口。关闭完整桌面窗口默认只隐藏窗口，不能停止后台采集；真正退出必须通过菜单栏或托盘菜单执行。

### macOS 菜单栏

点击菜单栏图标打开轻量 Popover。Popover 至少可以读取预聚合的：

- 采集状态、最近告警。
- 当前应用、Provider、模型、项目和会话标题（缺失时显示 `Unavailable`）。
- 当前会话输入、缓存读取、输出和缓存命中率。
- 今日 Token 总量。
- 官方额度摘要（若数据源未提供则保持不可用，不从百分比反推 Token）。

菜单栏支持两种显示形式：仅图标、图标加文字。文字代表可以由用户选择：

- 今日 Token。
- 当前会话 Token。
- 缓存命中率。
- 官方额度已用百分比。
- 不显示文字。

### Windows 系统托盘

行为约定：

- 单击：打开轻量弹窗。
- 双击：打开完整桌面面板。
- 右键：打开功能菜单。
- 鼠标悬停：显示今日 Token、采集状态和当前 Provider。
- 关闭完整桌面窗口：隐藏窗口，后台 Core 继续采集。
- 真正退出：通过右键托盘菜单执行“退出 TokenBuddy”。

### 轻量弹窗边界

轻量弹窗不是完整面板的缩小版，不得：

- 加载全部历史会话。
- 绘制大型趋势图。
- 扫描原始 JSONL。
- 运行复杂数据库聚合。
- 一次展示几十个指标。

Core 必须提前维护 `QuickSummary`，轻量弹窗只读取这一份摘要。摘要中的计数、费用、额度和归因都必须遵守全局缺失值与精度规则；未知值保持 `None` / `Unavailable`，不能为了展示方便写成零。

建议的跨端领域契约如下（具体枚举和额度字段由 `domain` crate 定义）：

```rust
pub struct QuickSummary {
    pub collection_status: CollectionStatus,
    pub active_app: Option<AppKind>,
    pub active_session_title: Option<String>,
    pub provider_name: Option<String>,
    pub model: Option<String>,

    pub session_input_tokens: Option<u64>,
    pub session_cache_read_tokens: Option<u64>,
    pub session_output_tokens: Option<u64>,
    pub session_cache_hit_rate: Option<f64>,

    pub today_total_tokens: Option<u64>,
    pub quota_summary: Option<QuotaSummary>,
    pub latest_warning: Option<String>,
}
```

`today_total_tokens` 只有在聚合结果已知时才返回 `Some`；已知总量确实为零时才允许返回 `Some(0)`。`QuotaSummary` 必须保留官方窗口和适用精度，不能把额度百分比换算成所谓的准确 Token 数。

### 完整桌面面板与本地网页面板

完整桌面面板和本地网页面板必须共用同一个 React SPA，不得开发两套 UI 或两套统计逻辑。建议路由：

```text
/quick       托盘轻量弹窗
/dashboard   总览
/sessions    会话列表
/sessions/:id 会话详情
/providers   Provider 统计
/quotas      官方额度
/sources     数据源状态
/settings    设置
```

两种入口只区分数据访问方式：

```text
桌面端：Tauri IPC -> Rust Core -> SQLite
网页端：loopback HTTP API -> Rust Core -> SQLite
```

本地网页服务默认只监听 `127.0.0.1` 和 `::1`，禁止监听 `0.0.0.0` 或其他局域网地址。网页服务按需启动，适合浏览器长期查看、大屏分析、开发调试、复制表格和导出数据；它同样不得直接访问 SQLite 或原始日志。

---

# 20. 隐私与安全

## 20.1 默认不保存

- Prompt 正文
- Completion 正文
- 源代码正文
- Authorization Header
- Cookie
- 完整 API Key
- OAuth Token
- Refresh Token

## 20.2 API Key 指纹

生成方式：

```text
fingerprint = SHA256(local_random_salt + api_key)
```

只显示前 8 至 12 位。

## 20.3 日志脱敏

必须过滤：

```text
Authorization
Proxy-Authorization
Cookie
Set-Cookie
x-api-key
api-key
anthropic-api-key
openai-api-key
```

## 20.4 本地服务

OTel Receiver 和代理：

- 默认只监听 localhost。
- 使用随机或可配置端口。
- 提供进程级随机访问 Token。
- 不接受局域网请求。

本地网页 Dashboard 是独立于 OTel Receiver 和本地代理的按需服务：

- 只允许绑定 `127.0.0.1` 和 `::1`。
- 明确禁止绑定 `0.0.0.0`，避免意外暴露到局域网。
- 通过 Rust Core 的只读查询服务返回数据，不直接访问 SQLite。
- 未启动本地网页入口时，不影响桌面端和后台采集 Core。

---

# 21. 性能要求

## 21.1 目标

- 空闲内存：尽量低于 150 MB。
- 空闲 CPU：接近 0%。
- 10 GB 历史日志可增量导入。
- UI 查询不扫描原始日志。
- 10 万 usage_events 列表分页流畅。

以上是工程目标，不是绝对保证。

## 21.2 文件导入

- 分块读取。
- 单次事务批量写入。
- 每批建议 500 至 2000 条。
- 避免每条事件单独提交事务。

## 21.3 数据库索引

至少建立：

```sql
CREATE INDEX idx_usage_time ON usage_events(occurred_at);
CREATE INDEX idx_usage_session ON usage_events(session_id);
CREATE INDEX idx_usage_provider ON usage_events(provider_id);
CREATE INDEX idx_usage_model ON usage_events(model);
CREATE INDEX idx_usage_app ON usage_events(app);
CREATE INDEX idx_usage_request ON usage_events(request_id);
CREATE INDEX idx_quota_account_time ON quota_snapshots(account_id, captured_at);
```

## 21.4 Tray-first 资源与响应目标

Tray-first 模式必须优先保证后台采集的低干扰和轻量入口的响应速度：

- `QuickSummary` 查询 P95 < 50 ms。
- 托盘 / 菜单栏轻量弹窗打开 P95 < 200 ms。
- 空闲 CPU 持续平均目标 < 0.5%。
- 托盘模式默认不创建完整 Dashboard WebView；轻量弹窗隐藏较长时间后可以销毁，下一次打开再创建。
- 文件监听采用增量读取；历史日志只做首次导入，之后记录 offset 或等价 cursor。
- 完整 Dashboard 只查询 SQLite 和 Core 查询服务，不碰原始日志。

以上是工程目标，不是当前已经验证的数据，必须在 macOS 和 Windows 真机测试中分别测量并记录结果。

---

# 22. 错误处理

任何单个 Adapter 失败，不得导致整个应用退出。

错误分类：

```text
SourceNotFound
PermissionDenied
SchemaUnsupported
DatabaseLocked
MalformedRecord
NetworkUnavailable
AuthenticationExpired
RateLimited
PortInUse
ProxyUpstreamError
```

每个错误必须包含：

- 用户可读信息
- 技术详情
- 恢复建议
- 是否可重试

---

# 23. 测试策略

## 23.1 单元测试

必须覆盖：

- OpenAI usage 映射
- Anthropic usage 映射
- 累计快照做差
- 重复事件去重
- 缓存命中率
- 缺失字段处理
- API Key 指纹
- 价格计算
- 时间窗口关联

## 23.2 Fixture 测试

仓库中建立脱敏样本：

```text
fixtures/
├── codex/
│   ├── simple_session.jsonl
│   ├── duplicate_snapshot.jsonl
│   ├── subagent_inherited_history.jsonl
│   └── malformed_lines.jsonl
├── claude/
│   ├── simple_session.jsonl
│   ├── subagent.jsonl
│   └── schema_variant.jsonl
├── otel/
│   ├── codex_otlp.bin
│   └── claude_otlp.bin
├── cc_switch/
│   └── sanitized.db
├── cockpit/
│   └── sanitized_usage.json
└── opencode/
    └── sanitized.db（测试内生成：session/message/part 表 + step-finish 快照）
```

## 23.3 集成测试

- macOS 文件监听
- Windows 文件监听
- SQLite 并发读写
- 应用休眠恢复
- OTel Receiver
- 本地代理 SSE 透传
- 客户端取消
- 上游超时

## 23.4 回归测试

每次支持新的 Codex / Claude Code 日志格式时：

- 添加新的 fixture。
- 保留旧格式 fixture。
- 禁止修改旧 fixture 让测试“通过”。

---

# 24. MVP 范围

## 24.1 MVP 必须完成

1. Tauri 2 桌面壳。
2. 单实例后台 Rust Core，以及 Core 生命周期和退出控制。
3. SQLite migrations。
4. Codex Session 历史导入。
5. Claude Code Session 历史导入。
6. 文件增量监听、轮转处理和 cursor 持久化。
7. 统一 Token 语义、缺失值语义和精度分级。
8. `QuickSummary` 维护与查询。
9. macOS 菜单栏入口。
10. Windows 系统托盘入口。
11. 轻量 Popover / 托盘弹窗。
12. 完整桌面 Dashboard。
13. 本地网页 Dashboard；与桌面端共用同一个 React SPA 和 Rust 查询服务。
14. 会话列表。
15. 会话详情。
16. 总览统计。
17. 官方额度摘要的可用态 / 不可用态展示，不从百分比反推 Token。
18. CSV / JSON 导出。
19. 完整桌面窗口关闭后，后台 Core 继续采集。
20. macOS 构建。
21. Windows 构建。
22. 安装引导中询问是否开机自启，默认不得静默修改用户选择。

## 24.2 MVP 可延后

- Codex OTel
- Claude OTel
- CC Switch Adapter
- Cockpit Adapter
- 各 Provider 的官方额度数据源适配器（MVP 先完成统一字段和 `Unavailable` 展示）
- 自动更新

## 24.3 MVP 明确不做

- 本地代理
- API Key 管理
- 多 Provider 路由
- 云同步

---

# 25. 开发阶段

## Phase 0：仓库初始化

任务：

- 初始化 Tauri 2 + React + TypeScript。
- 配置 Rust workspace。
- 配置 lint、format、test。
- 建立 CI。

验收：

- macOS 可启动。
- Windows CI 可编译。
- 前后端 IPC 示例可用。

## Phase 1：数据核心

任务：

- SQLite migrations。
- 统一类型定义。
- Adapter trait。
- Import cursor。
- Event hash。
- PrecisionLevel。

验收：

- fixture 能导入。
- 重复导入事件数不增加。

## Phase 2：Codex Session

任务：

- 自动检测路径。
- 扫描历史 JSONL。
- 增量导入。
- 去重。
- 会话聚合。

验收：

- 输入、缓存、输出、推理统计与原始日志一致。
- 重启应用后不会重复累计。

## Phase 3：Claude Session

任务同上。

验收：

- 支持至少两种日志 Schema。
- 无法解析字段显示 Unavailable。

## Phase 4：Tray-first 入口与展示层

任务：

- 实现单实例后台采集 Core 的启动、停止和退出生命周期。
- 启动时注册 macOS 菜单栏和 Windows 系统托盘，不自动打开完整窗口。
- 实现 Core 维护的 `QuickSummary` 及其 Tauri 查询命令。
- 实现 macOS 菜单栏 Popover、Windows 托盘轻量弹窗及各自的交互约定。
- 实现共享 React SPA 的 `/quick`、`/dashboard`、`/sessions`、`/sessions/:id`、`/providers`、`/quotas`、`/sources` 和 `/settings` 路由。
- 完整桌面面板通过 Tauri IPC 访问 Rust Core；本地网页面板通过 loopback HTTP API 访问同一 Rust Core。
- 本地网页服务只绑定 `127.0.0.1` 和 `::1`，按需启动。
- 完成总览页、会话列表、会话详情、筛选和导出。
- 完整窗口关闭后只隐藏窗口，后台 Core 和文件监听继续运行；托盘菜单提供真正退出。

验收：

- 应用启动后 Core 和系统入口已运行，但完整窗口没有自动弹出。
- 菜单栏 / 托盘轻量弹窗只读取 `QuickSummary`，不扫描原始日志、不加载全部历史、不运行复杂聚合。
- 四个入口的同一查询在 Token、精度、缺失值和额度语义上保持一致。
- 可以从会话追踪到请求级 Token，精度可见。
- 关闭完整窗口后仍能持续采集；退出后所有后台资源被释放。
- 本地网页 API 无法从局域网访问。
- 在 macOS 和 Windows 真机分别验证入口交互，并记录 `QuickSummary` P95、轻量弹窗 P95 和空闲 CPU。

## Phase 5：OTel

任务：

- OTLP HTTP Receiver。
- Codex OTel 映射。
- Claude OTel 映射。
- 与 Session 关联。

验收：

- 新请求实时出现在面板。
- 主 Agent / 子 Agent 可区分。

## Phase 6：CC Switch / Cockpit

任务：

- 只读 Adapter。
- Provider 和账号归因。
- 官方额度或代理 usage 导入。

验收：

- 不修改第三方数据库。
- 数据源失败不影响 Session 统计。

## Phase 7：可选本地代理

任务：

- OpenAI Responses API。
- Anthropic Messages API。
- SSE 转发。
- Usage 捕获。
- Keychain。

验收：

- 不增加明显首 Token 延迟。
- 代理关闭后用户可恢复直连。
- API Key 不进入日志和 SQLite。

---

# 26. 推荐仓库结构

```text
TokenBuddy/
├── PROJECT_SPEC.md
├── README.md
├── package.json
├── pnpm-workspace.yaml
├── apps/
│   └── desktop/
│       ├── src/
│       │   ├── app/
│       │   ├── components/
│       │   ├── features/
│       │   │   ├── dashboard/
│       │   │   ├── sessions/
│       │   │   ├── providers/
│       │   │   ├── quotas/
│       │   │   ├── sources/
│       │   │   └── settings/
│       │   └── lib/
│       └── src-tauri/
│           ├── Cargo.toml
│           └── src/
│               └── main.rs
├── crates/
│   ├── domain/
│   ├── storage/
│   ├── adapters/
│   │   ├── codex-session/
│   │   ├── codex-otel/
│   │   ├── claude-session/
│   │   ├── claude-otel/
│   │   ├── cc-switch/
│   │   ├── cockpit/
│   │   └── opencode/
│   ├── correlation/
│   ├── otlp-receiver/
│   ├── proxy-core/
│   └── security/
├── migrations/
├── fixtures/
├── scripts/
└── docs/
    ├── architecture.md
    ├── token-semantics.md
    ├── adapter-development.md
    └── privacy.md
```

---

# 27. 核心 Rust 类型草案

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AppKind {
    Codex,
    ClaudeCode,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LauncherKind {
    Direct,
    CCSwitch,
    Cockpit,
    ObserverProxy,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IngestSource {
    SessionLog,
    Otel,
    Proxy,
    QuotaApi,
    ImportedDatabase,
    Estimated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PrecisionLevel {
    Verified,
    ExactSession,
    Correlated,
    Estimated,
    Unavailable,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NormalizedUsage {
    pub input_tokens_total: Option<u64>,
    pub input_tokens_uncached: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub output_tokens_total: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub visible_output_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEvent {
    pub id: String,
    pub occurred_at: DateTime<Utc>,
    pub app: AppKind,
    pub launcher: LauncherKind,
    pub ingest_source: IngestSource,
    pub source_id: String,

    pub provider_id: Option<String>,
    pub account_id: Option<String>,
    pub session_id: Option<String>,
    pub parent_session_id: Option<String>,
    pub request_id: Option<String>,
    pub response_id: Option<String>,
    pub model: Option<String>,
    pub query_source: Option<String>,

    pub usage: NormalizedUsage,

    pub provider_reported_cost: Option<f64>,
    pub estimated_cost: Option<f64>,
    pub currency: Option<String>,

    pub precision_token: PrecisionLevel,
    pub precision_session: PrecisionLevel,
    pub precision_provider: PrecisionLevel,
    pub precision_account: PrecisionLevel,

    pub raw_event_hash: String,
    pub raw_usage_json: Option<serde_json::Value>,
}
```

---

# 28. API / IPC 草案

Tauri Commands：

```text
get_dashboard_summary(filters)
list_sessions(filters, pagination)
get_session_detail(session_id)
list_usage_events(session_id, pagination)
list_sources()
rescan_source(source_id)
update_source_path(source_id, path)
list_providers()
list_accounts()
list_quota_snapshots(account_id)
export_usage(format, filters)
get_app_settings()
update_app_settings(settings)
```

前端禁止直接访问 SQLite。

---

# 29. 验收标准

## 29.1 Token 正确性

使用脱敏 fixture：

- 输入误差为 0。
- 缓存读取误差为 0。
- 输出误差为 0。
- 推理误差为 0。
- 不支持字段显示 N/A。

若数据来自 Estimated，则测试允许定义误差，但 UI 必须明确显示 Estimated。

## 29.2 幂等性

同一日志重复扫描 10 次：

```text
usage_events 数量不增加
会话汇总不变化
```

## 29.3 跨平台

macOS 和 Windows 必须完成：

- 自动检测默认路径
- 手动选择目录
- 增量监听
- 数据库迁移
- 导出 CSV / JSON

## 29.4 安全

- SQLite 中搜索不到完整 API Key。
- 日志中搜索不到 Authorization。
- OTel Receiver 不对局域网开放。
- 禁用代理后客户端可恢复原配置。

---

# 30. 关键设计决策

## 决策 1：第一版不做强制本地代理

原因：

- Session + OTel 已覆盖核心会话统计。
- 代理会成为单点故障。
- 代理协议兼容成本高。
- 第一版应先验证数据价值和 UI。

## 决策 2：架构预留代理

数据库和领域模型必须从第一天包含：

```text
request_id
provider_id
account_id
upstream_url
provider_reported_cost
ingest_source
precision_level
```

## 决策 3：官方额度和原始 Token 分开

禁止在 UI 中混成一个指标。

## 决策 4：精度是产品功能

所有关键字段都必须能解释：

- 数据来自哪里
- 是否真实上报
- 是否通过关联
- 是否估算

## 决策 5：兼容 CC Switch / Cockpit，但不依赖它们

本项目可读取它们的数据，但运行时不要求它们常驻。

---

# 31. Codex 实施顺序

Codex 在开始编码时按以下顺序执行：

1. 检查现有仓库结构。
2. 如果仓库为空，初始化 Tauri 2 + React + TypeScript。
3. 建立 Rust workspace 和 `domain` crate。
4. 先实现数据库 migration 和核心类型。
5. 编写脱敏 fixture，不要先连接真实用户目录。
6. 先写解析、累计快照、cursor、轮转和幂等测试，再实现 Codex Session Adapter。
7. 实现 Claude Session Adapter，并为每种 Schema 保留独立 fixture。
8. 完成文件监听、增量导入、会话聚合和只读查询服务。
9. 实现单实例后台采集 Core，并让 Core 维护 `QuickSummary`。
10. 实现 macOS 菜单栏、Windows 系统托盘和轻量弹窗；启动时不自动打开完整窗口。
11. 实现共享 React SPA 的轻量入口、完整桌面 Dashboard 和路由。
12. 实现按需启动的 loopback 本地 Web API；只监听 `127.0.0.1` 和 `::1`。
13. 完成跨入口一致性、窗口隐藏后继续采集、退出释放资源和真机性能测试。
14. OTel、CC Switch、Cockpit、代理按后续 Phase 实现；本地代理不得成为 Core 或统计功能的前置条件。

每个阶段：

- 先写测试。
- 再写实现。
- 运行格式化、lint、单元测试。
- 更新实施状态。

---

# 32. Codex 禁止事项

Codex 不得：

1. 未经说明将技术栈改为 Electron。
2. 为省事把所有 Adapter 写进一个文件。
3. 将缺失 Token 写成 0。
4. 直接修改用户的 Codex、Claude、CC Switch 或 Cockpit 配置。
5. 保存完整 API Key。
6. 默认记录 Prompt 或代码正文。
7. 仅按时间戳去重。
8. 用模型官方价格替代第三方 Provider 价格并标记为真实费用。
9. 在没有测试的情况下实现代理。
10. 因某个数据源损坏导致整个应用启动失败。
11. 把官方额度百分比换算成所谓“准确 Token”。
12. 在无法关联 Provider 时偷偷选取最近的 Provider 并标为 Verified。

---

# 33. 初始实施任务

当前应执行的第一批任务：

```text
T001 初始化 Tauri 2 + React + TypeScript 项目
T002 建立 Rust workspace
T003 建立 domain crate 和核心类型
T004 建立 SQLite migration 系统
T005 实现 source、session、usage_event repository
T006 添加 Codex JSONL fixtures
T007 实现 CodexSessionAdapter 初版
T008 实现累计快照去重
T009 实现 import cursor
T010 实现会话汇总查询
T011 实现最小 Dashboard
T012 实现会话列表和详情页
T013 增加 macOS/Windows 路径检测
T014 添加 CI 编译与测试
```

## 33.1 Tray-first 补充实施要求

T001 至 T014 是本计划最初定义的第一批任务编号，并不是全部产品需求的标题。根据 Tray-first 调整，完成初始任务后还必须按 Phase 4 实现以下要求：

1. 应用只运行一个后台采集 Core；所有面板共享 Core、SQLite、统计语义和查询服务。
2. 启动后默认注册 macOS 菜单栏或 Windows 系统托盘，开始监听 Token 数据，但不自动弹出完整窗口。
3. Core 维护 `QuickSummary`，轻量弹窗只读摘要，不扫描原始 JSONL、不加载历史、不执行复杂聚合。
4. macOS 菜单栏支持 Popover、图标或图标加文字显示；Windows 托盘支持单击轻量弹窗、双击完整面板、右键菜单和悬停摘要。
5. 完整桌面面板与本地网页面板共用一个 React SPA 和一套 Rust 查询服务，只区分 Tauri IPC 与 loopback HTTP 访问通道。
6. 本地网页服务按需启动，只监听 `127.0.0.1` 和 `::1`，禁止监听 `0.0.0.0` 或局域网地址。
7. 关闭完整窗口只隐藏窗口，后台 Core 继续采集；真正退出只能通过菜单栏或托盘菜单执行。
8. 验证重复入口不会重复扫描、重复导入或改变会话聚合；分别在 macOS 和 Windows 真机验证入口行为和资源目标。

完成 T001 至 T014 后，再评估 OTel 和其他 Adapter。

---

# 34. 实施状态

```text
[x] Phase 0：仓库初始化
[x] Phase 1：数据核心
[x] Phase 2：Codex Session
[x] Phase 3：Claude Session
[x] Phase 4a：初始桌面面板（T011-T013）
[x] Phase 4b：Tray-first 最小闭环（后台 Core、QuickSummary、托盘、轻量弹窗与 loopback API）
[x] Phase 4b 展示层与采集可靠性补全：共享 SPA 路由、原生文件通知、轮询兜底、loopback `::1` 与 Core 多入口集成测试
[x] Phase 4b 正确性与 MVP 补全：QuickSummary Core 边界、部分 JSONL 行重试、缺失值聚合、筛选与 CSV/JSON 导出、单实例、自启动和按需 WebView
[x] Phase 4b 路径选择与 Codex 官方账号：原生目录/文件选择器、`auth.json` 账号身份、官方额度窗口入库
[ ] Phase 4b 跨平台真机交互补验：Windows 托盘与 macOS/Windows 菜单栏或托盘交互、隐藏窗口持续采集和 Windows CPU/P95
[x] Phase 5：OTel（可选回环 OTLP/HTTP traces receiver、Core 集成、跨来源关联与优先级去重）
[x] Phase 6：CC Switch / Cockpit（只读 Adapter、Provider/账号归因、Codex 官方额度；代理 usage 导入按防重复计数原则不实现）
[x] Phase 6 官方额度 API 独立适配：直接读取 ChatGPT OAuth 官方额度、无 Cockpit 管理、额度快照幂等与手动刷新
[ ] Phase 7：可选本地代理
```

OpenCode 只读适配已完成（2026-08-07）：新增独立 `tokenbuddy-opencode`，只读打开 `opencode.db`（macOS/Linux 默认 `~/.local/share/opencode/opencode.db`，Windows 默认 `%LOCALAPPDATA%\opencode\opencode.db`，均允许用户自定义路径），把 `session` 表映射为会话（标题、项目目录、模型、父会话），把 `part` 表的 `step-finish` 记录映射为每次模型调用的 usage 事件。已在真实数据库上验证：会话累计计数器恰好等于其 step-finish 请求之和（输入、输出、推理、缓存读逐一相等），且不存在单独的累计快照，因此按请求导入不会重复计数；`tokens.total` 只是各字段之和，不单独存储。归一化遵循 Anthropic 式分离语义（`input` 为未缓存输入，缓存读/写独立上报），缺失字段保持 `Unavailable`；OpenCode 自行按模型价格表计算的 `cost` 记入 `estimated_cost`，绝不冒充 Provider 实报费用；`providerID` 只描述用户配置的 Provider 插件而非真实上游，因此 Provider/账号归因保持 `Unavailable`，不生成 Provider 记录。增量导入以 `part.time_created` 高位水位为 cursor，同毫秒桶按稳定 part-id 哈希重读并靠存储层去重幂等；原始 Prompt、工具输入、推理文本与 completion 从不进入领域模型。Core、Tauri commands、loopback `/api/detect-opencode`、`/api/rescan-opencode` 与设置页均已接入；`AppKind` 新增 `open_code`（含 SQLite 迁移 0009）。新增适配器单测（解析、幂等、cursor、坏行、Schema 降级）与 Core 集成测试（导入、幂等、删库降级、异常 Schema 只报源错误不阻断应用）。当前限制：依赖 OpenCode `session`/`message`/`part` 三表；文件监听（`file_watch`）未启用，更新靠后台轮询兜底；Windows 真机路径仍未实测。

最近更新：2026-08-09

Phase 0 已完成：Tauri 2 + React + TypeScript 桌面壳、Rust workspace、格式化/lint/test/build 命令和 macOS/Windows CI 配置均已建立。Windows 构建仍需在远程 GitHub Actions 环境中执行确认。

Phase 1 已完成：共享 domain 类型、SQLite 初始迁移、sources/sessions/usage_events/import_cursors 持久化、稳定 raw event hash、幂等批量导入、精度分级和缺失值语义已实现。代理、OTel、CC Switch 和 Cockpit 仍未实现。

Phase 2 已完成：Codex 脱敏 JSONL fixture、普通 usage 与累计快照解析、重复快照去重、回退重置、子 Agent 继承历史跳过、坏行降级、文件 cursor 增量导入、文件轮转签名检测、会话聚合和 macOS/Windows 默认路径检测已实现。

Phase 4a 已完成：Tauri commands 已提供 dashboard、session list/detail、usage event 和 source 查询；最小 Dashboard、会话列表/详情时间线、精度徽标、Codex 路径检测和显式扫描入口已完成。T014 CI 已加入 macOS/Windows 的前端、Rust 测试、lint、检查和 Tauri 无 bundle 编译步骤。

Phase 3 已完成：新增独立 `tokenbuddy-claude-session`，支持 Claude Code `projects/**/*.jsonl` 的 V1/V2/保守 fallback 解析、脱敏 usage 保留、子 Agent 父子会话、继承历史跳过、增量 cursor、部分尾行重试、文件轮转、累计快照差分和重复导入幂等；Claude Home 支持 macOS/Linux 默认路径、Windows 默认路径和用户自定义配置。Core 以独立 Adapter 边界导入 Codex 与 Claude，单个来源失败不会阻断另一个来源，并通过 Tauri IPC、loopback API 和共享 React SPA 暴露检测/重扫入口。Claude Session 的缺失字段、Provider/费用归属和未知未来 Schema 保持 `Unavailable`；Claude OTel 仍按 Phase 5 实现。

Phase 4b 最小闭环已完成：新增独立 `tokenbuddy-core`，由单个后台线程持有 SQLite 查询 / 写入边界，启动时导入现有 Codex 与 Claude Session 并持续执行基于 cursor 的增量导入；Core 维护 `QuickSummary`，Tauri commands、macOS 菜单栏 / Windows 托盘、隐藏启动的完整窗口、轻量 `/quick` 窗口和按需 loopback HTTP API 共用同一 Core。关闭完整窗口只隐藏窗口，退出菜单才停止 Core；本地网页服务绑定 `127.0.0.1` 与 `::1`。

Phase 4b 展示层与采集可靠性实现已完成：Core 使用 `notify` 原生文件事件作为正常唤醒路径，并保留低频轮询兜底；新增 Core 生命周期和多入口共享的集成测试；共享 SPA 已覆盖 `/providers`、`/quotas`、`/settings`，对应 Tauri / loopback 查询契约和显式 `Unavailable` 状态；`QuickSummary` 查询 P95、轻量 HTTP 入口 P95 和 macOS 打包应用空闲 CPU 已完成测量。macOS debug 打包验收已实际打开三个新增路由，关闭完整窗口后保持进程和 Core 存活，并在隐藏窗口期间通过原生事件导入脱敏 fixture 新记录。Windows 真实托盘交互、隐藏窗口持续采集和 CPU/P95 仍需在 Windows 真机或 CI 运行环境补验；macOS 状态栏图标的直接点击仍受当前 Computer Use 无法读取 `SystemUIServer` 状态项的限制，但默认 accessory 启动、完整窗口隐藏和 Core 采集链路已确认。Claude OTel、CC Switch、Cockpit、官方额度数据源和本地代理仍按后续 Phase 实现，本地代理继续不是 Core 或统计功能的前置条件。

Phase 4b 正确性与 MVP 补全已完成：Quick 面板只消费 Core 的 `QuickSummary`，展示活动会话的标题、项目、Provider 和模型；Codex/Claude 导入会保留未完成 JSONL 尾行的 cursor 位置并在追加完成后重试；Session、Provider、Dashboard 和 QuickSummary 聚合在字段缺失时返回 `Unavailable` 而不是把已知事件的部分和冒充完整总数。Dashboard 与 loopback API 已支持日期、应用、Provider、账号、模型、项目、精度和搜索筛选，并可导出不含原始 payload 的 CSV/JSON；Tauri 已接入单实例转发、自启动同步和按需创建 `main`/`quick` WebView，托盘优先启动不再预建完整 Dashboard WebView。Windows 真机安装、自启动和托盘行为仍需在 Windows 环境补验，后续 Phase 的 Claude OTel、CC Switch、Cockpit、官方额度 Adapter 和本地代理未在本批次实现。

Phase 6 与路径选择补全已完成：CC Switch 与 Cockpit 只读 Adapter 已在上一批次接入并只提供 Provider / 账号上下文（按 §6.1 与 §10.1，不导入会与 Session 日志重复计数的代理 usage）。本批次补上 Codex 官方账号与官方额度：只读解析 `<CODEX_HOME>/auth.json`，支持 ChatGPT 登录与 API Key 两种模式，账号身份以每安装随机盐指纹化后入 `accounts`，原始 access token / refresh token / id token / API Key 全程不落库（§20.1、§20.2）；rollout 日志中的 `rate_limits` 作为独立数据类型写入 `quota_snapshots`，窗口按上报长度标注、`reset_at` 由倒计时折算、同一窗口百分比不变时不重复记录，托盘 `QuickSummary` 与额度页因此首次出现真实官方额度窗口。额度精度记为 `Correlated`：百分比来自官方响应，但归属账号是通过 Codex Home 关联得出的，按 §14 以最弱环节定级，绝不显示为 Verified，也绝不从百分比反推 Token（§8.4）。设置页四个数据源路径新增原生目录 / 文件选择器（`tauri-plugin-dialog`），仅桌面壳提供，浏览器面板保留文本输入；选择只填入表单，保存仍是显式操作，取消不修改既有配置。仍未实现：Claude / 其他 Provider 的官方额度数据源、OTel、本地代理；选择器与 Windows 文件过滤器的真机交互验收仍待补。

多账号轮换保护与发布流程已加入：检测到 CC Switch 或 Cockpit 数据库时，Codex 适配器仍上报 `auth.json` 中的官方账号身份，但不再把该账号写入 usage 事件、也不产出额度快照——启动器在多个账号间轮换时，`auth.json` 只描述此刻登录的账号，按它归因会把其他账号的历史算到当前账号头上。判定刻意保守：丢失一次归因可以补回，发布一个错误归因不可以。真正的多账号归因需从 Cockpit `request_logs` 的 `account_id` / `email` 按时间窗关联，尚未实现。同时新增 `.github/workflows/release.yml`，由 `windows-latest` 与 `macos-15` 在 `v*` tag 上产出 `.msi` / `-setup.exe` / `.dmg` 并发布为 pre-release；安装包未签名，Windows 托盘、自启动与路径选择器仍未在真机验证。

Cockpit 多账号归因已完成：Cockpit `request_logs` 的 `account_id` / `email` 现在产出独立账号行与「账号活动时间窗」，Codex 用量事件按发生时刻落到当时实际服务它的账号上，精度 `Correlated`（§17.2）；启动器扫描之前导入的历史事件会被回填，包括原本落在会话日志占位账号里的那些。请求间隔超过 30 分钟断开窗口，窗口前后各留 60 秒余量；两个账号的窗口覆盖同一时刻时保持占位账号，不做二选一。回填只改写空账号与 `auth_mode = 'session_log'` 的占位账号，其他来源已解析的真实账号不动。Cockpit 依然不产出 usage 事件（§6.1）。仍未实现：轮换环境下的官方额度归属（`QuotaSnapshot.account_id` 需改为可选并复用同一套时间窗解析）、CC-Switch 的账号列接入、窗口阈值的真实数据标定。

Windows UI 回归修复已完成（2026-07-29）：快速面板在内容高度变化后会重新锚定托盘，根据显示器工作区在顶部/底部任务栏之间选择可见位置，并按目标显示器 DPI 计算物理尺寸；面板高度受工作区约束，内容过长时可纵向滚动。主面板数据源路径控件改为窄窗口友好的网格布局，长 Windows 路径可收缩，空检测结果不再占行，WebView2 不支持磨砂时有实色回退。前端测试、构建、lint、格式检查、Rust workspace 测试和 clippy 已通过；Windows MSVC 构建、托盘交互、高 DPI 与隐藏窗口持续采集仍需 Windows CI/真机补验，因此“Phase 4b 跨平台真机交互补验”保持未完成。

Adapter descriptor 与隐私默认值硬化已完成（2026-08-04）：Codex、Claude Code、CC Switch 和 Cockpit 均声明集中式 `AdapterDescriptor`，Core 维护统一 Adapter catalog，并明确外部 Adapter 的只读边界，描述每个来源是否提供 usage、Provider/account context、quota 和文件监听能力；错误状态也从 descriptor 生成，避免源信息散落在 Core 分支中。`save_request_metadata` 现在是真正的显式 opt-in：默认导入不把脱敏 `raw_usage_json` 写入 SQLite，关闭设置时会清除历史原始 usage 元数据，normalized token、归因和精度事实保持不变；设置页提供明确的开关和删除提示。OTel Receiver 的具体实现与状态在后续 Phase 5 条目中维护。

Phase 5 OTel 与跨来源关联已完成（2026-08-04）：新增独立 `tokenbuddy-otel-receiver`，只监听 `127.0.0.1` 的 OTLP/HTTP `/v1/traces`，支持 protobuf/JSON，提取 `gen_ai.*` 与兼容别名中的数值 usage、request/response/session/model/provider 和延迟状态；原始请求正文、completion、未知属性和凭据不会进入默认持久化路径。OTel 通过 Core 的同一导入锁、SQLite 事务、QuickSummary 和查询服务入库，端口可留空关闭，端口冲突只产生 warning，不阻塞主应用。新增 `correlation_key` 及 source/precision precedence，同一 request/response 的 OTel、Session 等观察只保留更强事实并报告校正数，Otel-only span 也会生成无正文会话元数据。当前限制是仅支持 HTTP traces，不支持 OTLP gRPC/metrics/logs；缺少 app identity 的 span 保持 `unknown`，本地 Proxy 仍未实现；Windows 托盘和运行时验收仍待 Windows CI/真机。

官方额度 API 独立适配已完成（2026-08-04）：新增 `tokenbuddy-official-quota`，从 Codex Home 的 ChatGPT OAuth 登录态读取 access token 与 account id，并按官方 Codex 客户端使用的 `/backend-api/wham/usage` 路径（以及 Codex API 兼容路径）请求额度窗口。token 只在一次请求内存中使用，不进入 domain、cursor、source error 或 SQLite；API Key 登录保持官方 ChatGPT 订阅额度 `Unavailable`。官方响应会拆成独立 `QuotaSnapshot`，保留缺失字段、Credits、重置时间和 `Verified` 精度；响应 hash 作为增量 cursor，重复轮询不增加快照。Core 在桌面正式配置中启动时和后台轮询时刷新该数据源，官方接口失败只记录官方源告警，不阻断本地 Codex/Claude 统计；配置 CC Switch 或 Cockpit 不再阻止官方额度请求，未配置它们也能通过 Tauri/loopback 的手动刷新入口管理官方 ChatGPT 使用情况。新增脱敏响应 fixture、官方适配器请求头/幂等测试及无 Cockpit 的 Core 集成测试。当前限制：依赖 Codex Home 中仍有效的文件型 OAuth 登录态；access token 过期时提示重新登录，不在 TokenBuddy 内刷新或写回凭据；官方额度接口是随官方客户端验证的后端契约，若上游响应 schema 变化需更新 parser；Windows 真机托盘与额度网络场景仍待跨平台验收。

官方额度展示与托盘同步修复已完成（2026-08-04）：QuickSummary 在当前活动会话账号没有官方窗口时，会回退到最新的 ChatGPT 官方额度快照，因此不使用 Cockpit 也能在托盘查看官方账号的已用、剩余和重置时间；托盘额度行新增显式刷新入口，并在刷新完成后立即重新读取 Core 摘要。额度页改为按账号展示窗口、已用、剩余、重置和精度，快照列表保留采集时间与精度；额度页覆盖全局固定面板的最小高度和裁切规则，支持多账号/多窗口自然增长与窄窗口响应式布局。新增 QuickSummary 跨账号回退测试和桌面展示回归测试。当前限制仍是官方接口依赖 Codex Home 中有效的文件型 OAuth 登录态，Windows 真机托盘与网络场景仍待跨平台验收。

完整面板 UI 密度重构已完成（2026-08-04）：主导航改为内容自适应宽度并修复右对齐造成的左侧空白；通用面板改为内容驱动高度，筛选区和按模型/供应商区不再继承 480px 固定最小高度，搜索字段扩展为整行；Dashboard 结构、筛选交互、导出、扫描、会话详情和数据统计逻辑保持不变。浏览器视觉回归、前端格式检查、lint、55 项单测和生产构建均通过。

模型上下文与费用估算补全已完成（2026-08-04）：Codex/Claude Session 解析器现在会跨 session metadata、turn context、响应记录和增量 cursor 传播模型；同一会话只有一个明确模型时，会回填模型晚于 token snapshot 的历史事件，多模型或日志缺失时继续保持 `Unavailable`。SQLite 0007/0008 migration 会重读原生会话来源并按稳定 `raw_event_hash` 原地补齐缺失模型、Provider 与费用，不重复计数。新增 Claude `costUSD` 等供应商实报费用优先级，以及严格的 Provider + Model 价格表，对 OpenAI `gpt-5-codex` 和 Anthropic Claude 3.7 Sonnet 按未缓存输入、缓存读/写、输出计算 API-equivalent 估算；未知模型、第三方 relay 或缺少必要 Token 字段继续显示 `N/A`。中文「数据源」标签改用独立字距和比例，按模型/供应商面板补充估算说明。Rust workspace 测试、clippy、前端格式/lint/55 项单测、生产构建和 macOS `.app`/`.dmg` 构建均通过；本机 `/Applications/TokenBuddy.app` 已更新并运行。实际历史文件不存在或没有模型字段的旧行仍不可回填，Codex 订阅 credits rate card 未与 API USD 估算混用。

当前模型价格覆盖已完成（2026-08-04）：存储价格卡新增 OpenAI GPT-5.6 的 `Sol`/`Terra`/`Luna` 档位，以及 Anthropic Claude Opus 5、Claude Fable 5 的输入、输出和缓存价格；模型变体按精确 Provider + 已知模型族匹配，第三方中转仍不套用官方价格。应用启动时会重新计算已有且没有供应商实报费用的事件，供应商实报费用保持优先；OpenAI 会话日志没有独立缓存写入字段时，估算明确只计入已记录的输入、缓存命中和输出，不把未知写入计为零。新增价格规则、历史重算和 UI 说明测试；API 单价估算不代表订阅额度、Batch/Fast mode、区域加价或第三方实际账单。

费用单位与托盘展示已完成（2026-08-04）：完整面板、Provider 摘要和按模型/供应商表格的费用统一显示 USD 单位；`QuickSummary` 增加当前会话与本地日的实报/估算费用，托盘快速面板和 tooltip 均展示费用。费用优先级和 `N/A` 缺失语义保持不变；无事件的今日 Token 继续显示 0，有事件但必要字段缺失继续显示 `Unavailable`。新增 Tauri tooltip、Storage QuickSummary 和前端托盘渲染覆盖，Rust workspace 与前端验证继续通过。

Windows 自启动与窗口恢复补强已完成（2026-08-04）：Windows 自启动改为直接维护当前用户 `Run` 注册表项，并给带空格的安装路径写入带引号的命令行；启用时同步恢复 `StartupApproved` 状态，关闭时对不存在的旧项保持幂等。托盘双击、单实例转发和快速面板唤回会先解除窗口最小化，再显示并聚焦，避免“已显示但仍不可见”。新增安装路径引号回归测试；`Phase 4b 跨平台真机交互补验` 仍保持未完成，Windows MSVC 构建、真实托盘、高 DPI、隐藏窗口持续采集、CPU/P95 与路径选择器仍需 Windows CI/真机验收。

Windows 发布同步已完成（2026-08-05）：GitHub `agent/overnight` 最近的 OTel、官方额度、模型/费用、Dashboard 和 Windows 修复已合并到 `main`，版本 `v0.1.2` 已由 Release workflow 在 `windows-latest` 产出并发布 MSI 与 NSIS 安装器；CI run `30964052385` 的 Windows/macOS 格式、Lint、测试、前端构建、Rust 检查和 Tauri 无 bundle 构建均通过，Release run `30964628839` 全部成功。由于 Windows runner 上 Tauri MockRuntime 测试二进制仍会在加载阶段返回 `STATUS_ENTRYPOINT_NOT_FOUND`，该命令契约测试继续只在非 Windows 平台执行；Windows 纯桌面测试与生产 Tauri 构建已覆盖。`Phase 4b 跨平台真机交互补验` 仍未完成，安装包未签名，托盘、自启动、高 DPI、隐藏窗口持续采集和路径选择器仍需 Windows 真机确认。

Windows GUI subsystem 修复已完成（2026-08-05）：桌面二进制入口增加 Windows GUI subsystem 链接属性，直接启动 `tokenbuddy-desktop.exe` 不再创建命令行窗口；版本统一升级到 `v0.1.3`，本机格式、Lint、测试、前端构建、Rust 检查和 Tauri 构建均通过，Windows PE subsystem 待下一次 Windows CI 或实机确认。该修复不等同于 Windows 真机托盘、隐藏窗口持续采集和安装器体验验收，`Phase 4b 跨平台真机交互补验` 仍保持未完成。

Windows 分发与自动更新补全已完成（2026-08-07）：loopback Web 服务的 IPv6 绑定改为 best-effort，IPv6 被禁用（常见于企业网/VPN 的 Windows 机器）时面板仍走 `127.0.0.1` 可用；Windows NSIS/MSI 安装包改为内嵌 WebView2 离线安装器（+~127MB），首次安装无需联网，NSIS 界面语言为简体中文、MSI 本地化为 zh-CN；新增 Windows 应用内自动更新（`tauri-plugin-updater` 2.10.1，RSA 签名 + GitHub Releases `latest.json`，启动 10 秒后后台检查、原生对话框确认、`/UPDATE` 静默安装并自动重启，检查失败只记日志）；Release 工作流注入 `TAURI_SIGNING_PRIVATE_KEY` 仓库 secret、上传签名后的 NSIS/MSI 更新负载并生成 `latest.json`，且不再标记 pre-release（`/releases/latest/download` 不解析 pre-release）；`ci.yml` 增加 Cargo 构建缓存；Windows MockRuntime 测试二进制加载期 `STATUS_ENTRYPOINT_NOT_FOUND` 的根因已定位——tauri-winres 只给 `bin` 目标注入 Common Controls v6 manifest，`build.rs` 现在为所有目标补发该 manifest，命令契约测试重新在所有平台运行。本机格式、Lint、测试、前端构建、Rust 检查、Tauri 无 bundle 构建与带签名 release 打包均通过；Windows CI 将确认 MSVC 下测试恢复、NSIS/WiX 中文本地化与 offlineInstaller 内嵌。`Phase 4b 跨平台真机交互补验` 仍保持未完成：托盘、自启动、高 DPI、隐藏窗口持续采集、更新对话框与安装器体验待 Windows 真机验收。

Windows CI Manifest 链接修复已完成（2026-08-09）：Windows 更新检查器不再把后续原生弹窗仍需使用的 `AppHandle` 移入异步块；Tauri build-time 资源生成改为保留图标和版本资源但关闭其内置 application manifest，再由项目的 `windows-app-manifest.xml` 为生产和测试目标各链接唯一一份 Common Controls v6 manifest，消除了 `CVT1100 duplicate resource` / `LNK1123`，同时保留 MockRuntime 命令契约测试。GitHub Actions run `31285870999` 的 `Verify (windows-latest)` 已通过格式、Clippy/Lint、全部前端与 Rust 测试、前端生产构建、Rust workspace check 和 Tauri debug `--no-bundle` 构建。该结果确认 Windows MSVC 编译链路恢复，不等同于托盘、自启动、高 DPI、隐藏窗口采集、更新对话框或安装器的 Windows 真机交互验收，因此 `Phase 4b 跨平台真机交互补验` 仍保持未完成。

Claude 流式 usage 补全与 OpenCode Go 定价修复已完成（2026-08-09）：Claude Code 对同一 message ID 先写入零值临时 usage、再写入完整 usage 时，存储层现在以完整输入、缓存读写和输出原位补全同一稳定事件，不新增事件、不破坏重复导入幂等性；迁移 0010 仅清除 Claude Session cursor，使已有安装在下一次只读扫描时修复历史临时值。新增脱敏流式 Schema fixture、Adapter、Storage 和 Core 回归覆盖。`deepseek-v4-flash` 新增 OpenCode Go 官方端点专属价格卡，按每百万 token USD 0.14 未缓存输入、USD 0.0028 缓存读取、USD 0.28 输出估算，缓存写入不虚构费用；规则严格绑定 `https://opencode.ai/zen/go`，其他同名中转继续保持 `N/A`。Provider 归属变化后会立即重算或清除不再适用的静态价格估算，供应商实报费用与 Adapter 自带估算保持优先。前端格式、lint、55 项测试与生产构建、SQLite 0001-0010 顺序迁移、`git diff --check`，以及受影响 Claude Adapter、Storage、Core 的 Rust 1.97.1 Clippy、67 项测试和 workspace check 已通过；完整 Windows Tauri 桌面链接因本机缺少 MSVC Build Tools 且 GNU DLL 链接器不兼容而待 CI 验证。

DeepSeek 官方 API 定价已完成（2026-08-09）：`deepseek-v4-flash` 在 Provider 明确指向 `https://api.deepseek.com` 官方端点（含 `/anthropic`、`/v1` 子路径）时，按每百万 token USD 0.14 缓存未命中输入、USD 0.0028 缓存命中输入、USD 0.28 输出计算 API-equivalent 估算，不虚构缓存写入费用。规则要求官方 endpoint；仅由模型名推导出的 `deepseek` provider 或其他同名中转继续保持 `N/A`，符合 §18.1 与 §32.8 的 Provider + Model 定价边界。新增官方端点、兼容子路径、无端点拒绝和 Storage Provider 归属集成覆盖；供应商实报费用仍优先于估算。
