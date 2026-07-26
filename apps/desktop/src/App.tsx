import { useEffect, useMemo, useState } from "react";

import {
  detectCodexPath,
  getDashboardSummary,
  getQuickSummary,
  getSessionDetail,
  listSessions,
  listSources,
  openLocalWebApi,
  rescanCodex,
  type QuickSummary,
  type DashboardSummary,
  type DetectionResult,
  type PrecisionLevel,
  type SessionDetail,
  type SessionSummary,
  type SourceRecord,
  type UsageEvent,
  type UsageTotals,
} from "./lib/api";

const emptyTotals: UsageTotals = {
  event_count: 0,
  input_tokens_total: null,
  input_tokens_uncached: null,
  cache_read_tokens: null,
  cache_write_tokens: null,
  output_tokens_total: null,
  reasoning_tokens: null,
  visible_output_tokens: null,
  provider_reported_cost: null,
  estimated_cost: null,
  cache_hit_rate_percent: null,
};

function App() {
  return window.location.pathname === "/quick" ? (
    <QuickSummaryView />
  ) : (
    <DashboardView />
  );
}

function DashboardView() {
  const [dashboard, setDashboard] = useState<DashboardSummary | null>(null);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [sources, setSources] = useState<SourceRecord[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(
    null,
  );
  const [detail, setDetail] = useState<SessionDetail | null>(null);
  const [codexHome, setCodexHome] = useState("");
  const [detection, setDetection] = useState<DetectionResult | null>(null);
  const [status, setStatus] = useState("正在连接本地数据层…");
  const [error, setError] = useState<string | null>(null);
  const [isScanning, setIsScanning] = useState(false);
  const [refreshVersion, setRefreshVersion] = useState(0);

  useEffect(() => {
    let active = true;

    async function loadOverview() {
      try {
        const [nextDashboard, nextSessions, nextSources] = await Promise.all([
          getDashboardSummary(),
          listSessions(),
          listSources(),
        ]);
        if (!active) return;
        setDashboard(nextDashboard);
        setSessions(nextSessions.sessions);
        setSources(nextSources);
        setStatus("数据已从本地 SQLite 加载");
        setError(null);
      } catch {
        if (!active) return;
        setStatus("请通过 Tauri 启动以连接本地数据层");
        setError("浏览器预览没有 Tauri IPC；桌面应用启动后会显示真实数据。");
      }
    }

    void loadOverview();
    return () => {
      active = false;
    };
  }, [refreshVersion]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      setRefreshVersion((value) => value + 1);
    }, 5000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    let active = true;
    const sessionId = selectedSessionId;
    if (!sessionId) {
      return () => {
        active = false;
      };
    }

    async function loadDetail(sessionId: string) {
      try {
        const nextDetail = await getSessionDetail(sessionId);
        if (active) setDetail(nextDetail);
      } catch {
        if (active) setError("无法读取会话详情。");
      }
    }

    void loadDetail(sessionId);
    return () => {
      active = false;
    };
  }, [selectedSessionId, refreshVersion]);

  const totals = dashboard?.totals ?? emptyTotals;
  const selectedSession = useMemo(
    () =>
      sessions.find((item) => item.session.id === selectedSessionId) ?? null,
    [selectedSessionId, sessions],
  );
  const visibleDetail =
    detail?.summary.session.id === selectedSessionId ? detail : null;

  async function handleDetect() {
    try {
      const nextDetection = await detectCodexPath(codexHome.trim() || null);
      setDetection(nextDetection);
      setStatus(
        nextDetection.detected
          ? "已检测到 Codex Session 目录"
          : "未检测到 Codex Session 目录",
      );
      setError(null);
    } catch {
      setError("无法检测 Codex Home，请确认桌面应用已启动。");
    }
  }

  async function handleScan() {
    setIsScanning(true);
    try {
      const result = await rescanCodex(codexHome.trim() || null);
      setStatus(
        `扫描完成：新增 ${result.inserted_events} 条事件，跳过 ${result.skipped_records} 条记录`,
      );
      setError(null);
      setRefreshVersion((value) => value + 1);
    } catch {
      setError("Codex 扫描失败；请检查路径、权限或日志格式。");
    } finally {
      setIsScanning(false);
    }
  }

  async function handleOpenWeb() {
    try {
      const result = await openLocalWebApi();
      setStatus(
        result.url ? `本地网页面板已启动：${result.url}` : "本地网页面板已启动",
      );
      setError(null);
    } catch {
      setError("无法启动本地网页面板；请先通过 Tauri 桌面应用运行。");
    }
  }

  return (
    <main className="app-shell">
      <header className="topbar">
        <div>
          <p className="eyebrow">AI coding token observatory</p>
          <h1>TokenBuddy</h1>
          <p className="subtitle">本地优先，先把每一次模型调用看清楚。</p>
        </div>
        <div className="topbar-actions">
          <span
            className="status-pill"
            data-state={error ? "warning" : "ready"}
          >
            <span className="status-dot" aria-hidden="true" />
            {status}
          </span>
          <button
            className="primary-button"
            type="button"
            onClick={handleScan}
            disabled={isScanning}
          >
            {isScanning ? "扫描中…" : "扫描 Codex"}
          </button>
          <button
            className="quiet-button"
            type="button"
            onClick={handleOpenWeb}
          >
            本地网页
          </button>
        </div>
      </header>

      {error ? <p className="notice notice-warning">{error}</p> : null}

      <section className="source-bar" aria-labelledby="source-heading">
        <div>
          <p className="section-kicker" id="source-heading">
            数据源
          </p>
          <p className="source-description">
            当前只读导入 Codex Session JSONL；未知值保持
            Unavailable，不会被折算成 0。
          </p>
        </div>
        <div className="source-controls">
          <label htmlFor="codex-home">Codex Home</label>
          <input
            id="codex-home"
            value={codexHome}
            onChange={(event) => setCodexHome(event.target.value)}
            placeholder="留空使用系统默认路径"
          />
          <button className="quiet-button" type="button" onClick={handleDetect}>
            检测路径
          </button>
        </div>
        {detection ? (
          <span className={detection.detected ? "detection ok" : "detection"}>
            {detection.detected ? "Detected" : "Not found"}
          </span>
        ) : null}
      </section>

      <section className="dashboard-grid" aria-label="今日统计">
        <MetricCard
          label="输入 Token"
          value={formatTokens(totals.input_tokens_total)}
          tone="mint"
        />
        <MetricCard
          label="缓存读取"
          value={formatTokens(totals.cache_read_tokens)}
        />
        <MetricCard
          label="缓存写入"
          value={formatTokens(totals.cache_write_tokens)}
        />
        <MetricCard
          label="输出 Token"
          value={formatTokens(totals.output_tokens_total)}
          tone="ink"
        />
        <MetricCard
          label="推理 Token"
          value={formatTokens(totals.reasoning_tokens)}
        />
        <MetricCard
          label="缓存命中率"
          value={formatPercent(totals.cache_hit_rate_percent)}
        />
        <MetricCard label="事件数" value={formatTokens(totals.event_count)} />
        <MetricCard label="费用" value={formatCost(totals)} tone="ink" />
      </section>

      <section className="workspace-grid">
        <div className="panel sessions-panel">
          <div className="panel-heading">
            <div>
              <p className="section-kicker">Sessions</p>
              <h2>会话</h2>
            </div>
            <span className="count-label">{sessions.length} 条</span>
          </div>
          {sessions.length ? (
            <div className="session-list" role="list">
              {sessions.map((item) => (
                <SessionRow
                  key={item.session.id}
                  summary={item}
                  selected={item.session.id === selectedSessionId}
                  onSelect={() => setSelectedSessionId(item.session.id)}
                />
              ))}
            </div>
          ) : (
            <EmptyState
              title="还没有导入会话"
              description="确认 Codex Home 后点击“扫描 Codex”，TokenBuddy 会从 JSONL 增量导入。"
            />
          )}
        </div>

        <div className="panel detail-panel">
          {visibleDetail ? (
            <SessionDetailView detail={visibleDetail} />
          ) : selectedSession ? (
            <EmptyState title="正在读取会话详情…" description="" />
          ) : (
            <EmptyState
              title="选择一个会话"
              description="从左侧会话列表查看每一轮请求、Token 语义和精度。"
            />
          )}
        </div>
      </section>

      <footer className="footer-note">
        <span>
          {sources.length
            ? `${sources.length} 个数据源已登记`
            : "尚未登记数据源"}
        </span>
        <span>代理模式未启用 · 数据留在本机</span>
      </footer>
    </main>
  );
}

function QuickSummaryView() {
  const [summary, setSummary] = useState<QuickSummary | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;

    async function loadSummary() {
      try {
        const nextSummary = await getQuickSummary();
        if (!active) return;
        setSummary(nextSummary);
        setError(null);
      } catch {
        if (active) setError("无法读取后台 Core 摘要。");
      }
    }

    void loadSummary();
    const timer = window.setInterval(() => {
      void loadSummary();
    }, 2000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, []);

  return (
    <main className="quick-shell">
      <div className="quick-heading">
        <div>
          <p className="eyebrow">后台 Core</p>
          <h1>TokenBuddy</h1>
        </div>
        <span
          className={`quick-status status-${summary?.collection_status ?? "starting"}`}
        >
          {collectionStatusLabel(summary?.collection_status)}
        </span>
      </div>
      {error ? <p className="notice notice-warning">{error}</p> : null}
      <section className="quick-primary" aria-label="今日 Token">
        <span>今日 Token</span>
        <strong>{formatTokens(summary?.today_total_tokens)}</strong>
      </section>
      <section className="quick-session" aria-label="当前会话摘要">
        <div className="quick-session-title">
          <span>当前会话</span>
          <strong>{summary?.active_session_title || "Unavailable"}</strong>
        </div>
        <div className="quick-model">
          {summary?.active_app || "Unavailable"} ·{" "}
          {summary?.model || "Unavailable"}
        </div>
        <div className="quick-metrics">
          <QuickMetric
            label="输入"
            value={formatTokens(summary?.session_input_tokens)}
          />
          <QuickMetric
            label="缓存读取"
            value={formatTokens(summary?.session_cache_read_tokens)}
          />
          <QuickMetric
            label="输出"
            value={formatTokens(summary?.session_output_tokens)}
          />
          <QuickMetric
            label="缓存命中率"
            value={formatPercent(summary?.session_cache_hit_rate ?? null)}
          />
        </div>
      </section>
      {summary?.quota_summary ? (
        <p className="quick-note">
          官方额度：{formatPercent(summary.quota_summary.used_percent)} 已用 ·{" "}
          {summary.quota_summary.precision}
        </p>
      ) : (
        <p className="quick-note">官方额度 Unavailable</p>
      )}
      {summary?.latest_warning ? (
        <p className="notice notice-warning">{summary.latest_warning}</p>
      ) : null}
    </main>
  );
}

function QuickMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="quick-metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function MetricCard({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: "mint" | "ink";
}) {
  return (
    <article className={`metric-card ${tone ?? ""}`}>
      <p>{label}</p>
      <strong>{value}</strong>
    </article>
  );
}

function SessionRow({
  summary,
  selected,
  onSelect,
}: {
  summary: SessionSummary;
  selected: boolean;
  onSelect: () => void;
}) {
  const title =
    summary.session.title ||
    summary.session.external_session_id ||
    "未命名会话";
  return (
    <button
      className={`session-row ${selected ? "selected" : ""}`}
      type="button"
      onClick={onSelect}
    >
      <span className="session-row-main">
        <span className="session-title">{title}</span>
        <span className="session-meta">
          {summary.session.app} ·{" "}
          {summary.session.project_path || "项目路径 Unavailable"}
        </span>
      </span>
      <span className="session-row-tokens">
        <strong>{formatTokens(summary.totals.input_tokens_total)}</strong>
        <span>in</span>
        <strong>{formatTokens(summary.totals.output_tokens_total)}</strong>
        <span>out</span>
      </span>
    </button>
  );
}

function SessionDetailView({ detail }: { detail: SessionDetail }) {
  const { session } = detail.summary;
  return (
    <div className="detail-content">
      <div className="panel-heading detail-heading">
        <div>
          <p className="section-kicker">Session detail</p>
          <h2>
            {session.title || session.external_session_id || "未命名会话"}
          </h2>
          <p className="detail-subtitle">
            {session.project_path || "项目路径 Unavailable"}
          </p>
        </div>
        <PrecisionBadge
          level={detail.usage_events[0]?.precision_token ?? "unavailable"}
        />
      </div>
      <div className="detail-stats">
        <DetailStat
          label="输入"
          value={formatTokens(detail.summary.totals.input_tokens_total)}
        />
        <DetailStat
          label="缓存读取"
          value={formatTokens(detail.summary.totals.cache_read_tokens)}
        />
        <DetailStat
          label="输出"
          value={formatTokens(detail.summary.totals.output_tokens_total)}
        />
        <DetailStat
          label="推理"
          value={formatTokens(detail.summary.totals.reasoning_tokens)}
        />
      </div>
      <div className="timeline" aria-label="请求时间线">
        {detail.usage_events.length ? (
          detail.usage_events.map((event) => (
            <EventRow event={event} key={event.id} />
          ))
        ) : (
          <EmptyState
            title="没有请求级事件"
            description="该会话只有元数据，usage 仍为 Unavailable。"
          />
        )}
      </div>
    </div>
  );
}

function EventRow({ event }: { event: UsageEvent }) {
  return (
    <article className="event-row">
      <div className="event-time">{formatDate(event.occurred_at)}</div>
      <div className="event-main">
        <div className="event-title-row">
          <strong>{event.model || "模型 Unavailable"}</strong>
          <PrecisionBadge level={event.precision_token} />
        </div>
        <p>
          {event.request_id || event.response_id || "请求 ID Unavailable"} ·{" "}
          {event.ingest_source}
        </p>
      </div>
      <div className="event-usage">
        <span>{formatTokens(event.usage.input_tokens_total)} in</span>
        <span>{formatTokens(event.usage.output_tokens_total)} out</span>
      </div>
    </article>
  );
}

function DetailStat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function PrecisionBadge({ level }: { level: PrecisionLevel }) {
  return (
    <span className={`precision-badge precision-${level}`}>
      {precisionLabel(level)}
    </span>
  );
}

function EmptyState({
  title,
  description,
}: {
  title: string;
  description: string;
}) {
  return (
    <div className="empty-state">
      <span className="empty-mark" aria-hidden="true">
        ◌
      </span>
      <h3>{title}</h3>
      {description ? <p>{description}</p> : null}
    </div>
  );
}

function formatTokens(value: number | null | undefined): string {
  return value == null
    ? "Unavailable"
    : new Intl.NumberFormat("en-US").format(value);
}

function formatPercent(value: number | null): string {
  return value == null ? "Unavailable" : `${value.toFixed(1)}%`;
}

function formatCost(totals: UsageTotals): string {
  if (totals.provider_reported_cost != null)
    return totals.provider_reported_cost.toFixed(4);
  if (totals.estimated_cost != null)
    return `~${totals.estimated_cost.toFixed(4)}`;
  return "N/A";
}

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.valueOf())
    ? "时间 Unavailable"
    : date.toLocaleString();
}

function precisionLabel(level: PrecisionLevel): string {
  const labels: Record<PrecisionLevel, string> = {
    verified: "Verified",
    exact_session: "Exact session",
    correlated: "Correlated",
    estimated: "Estimated",
    unavailable: "Unavailable",
  };
  return labels[level];
}

function collectionStatusLabel(
  status: QuickSummary["collection_status"] | undefined,
): string {
  const labels: Record<QuickSummary["collection_status"], string> = {
    starting: "启动中",
    collecting: "采集中",
    idle: "待机",
    error: "需注意",
  };
  return status ? labels[status] : labels.starting;
}

export default App;
