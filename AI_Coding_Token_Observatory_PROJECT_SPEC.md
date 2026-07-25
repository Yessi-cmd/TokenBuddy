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

### 后续支持

- Gemini CLI
- Cursor Agent
- OpenCode
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
- 系统托盘、自动启动和原生路径访问较成熟。

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
- CC Switch DB 路径
- Cockpit 接口或数据路径
- OTel 端口
- 是否自动启动
- 是否允许代理模式
- 是否保存请求元数据
- 数据保留周期

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
└── cockpit/
    └── sanitized_usage.json
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
2. SQLite migrations。
3. Codex Session 历史导入。
4. Claude Code Session 历史导入。
5. 文件增量监听。
6. 统一 Token 语义。
7. 精度分级。
8. 会话列表。
9. 会话详情。
10. 总览统计。
11. CSV / JSON 导出。
12. macOS 构建。
13. Windows 构建。

## 24.2 MVP 可延后

- Codex OTel
- Claude OTel
- CC Switch Adapter
- Cockpit Adapter
- 官方额度
- 菜单栏 / 系统托盘
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

## Phase 4：桌面面板

任务：

- 总览页。
- 会话列表。
- 会话详情。
- 筛选。
- 导出。

验收：

- 可以从会话追踪到请求级 Token。
- 精度可见。

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
│   │   └── cockpit/
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
5. 编写 fixture，不要先连接真实用户目录。
6. 实现 Codex Session Adapter。
7. 完成单元测试和幂等测试。
8. 再实现 Claude Session Adapter。
9. 完成最小 UI。
10. 最后接入文件监听。
11. OTel、CC Switch、Cockpit、代理按后续 Phase 实现。

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

完成 T001 至 T014 后，再评估 OTel 和其他 Adapter。

---

# 34. 实施状态

```text
[ ] Phase 0：仓库初始化
[ ] Phase 1：数据核心
[ ] Phase 2：Codex Session
[ ] Phase 3：Claude Session
[ ] Phase 4：桌面面板
[ ] Phase 5：OTel
[ ] Phase 6：CC Switch / Cockpit
[ ] Phase 7：可选本地代理
```

最近更新：2026-07-25
