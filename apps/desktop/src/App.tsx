import { useEffect, useMemo, useState, type ReactNode } from "react";

import {
  getAppSettings,
  detectClaudePath,
  detectCodexPath,
  getDashboardSummary,
  getQuickSummary,
  getSessionDetail,
  exportUsage,
  listProviders,
  listQuotaSnapshots,
  listSessions,
  listSources,
  detectCcSwitchPath,
  detectCockpitPath,
  openLocalWebApi,
  rescanClaude,
  rescanCodex,
  rescanCcSwitch,
  rescanCockpit,
  saveExport,
  showMainWindow,
  isDesktopRuntime,
  updateAppSettings,
  type AppSettings,
  type QuickSummary,
  type DashboardSummary,
  type DetectionResult,
  type PrecisionLevel,
  type ProviderSummary,
  type QuotaSnapshot,
  type SessionDetail,
  type SessionSummary,
  type SourceRecord,
  type UsageEvent,
  type UsageFilters,
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

type DashboardFilterForm = {
  period_start: string;
  period_end: string;
  app: "" | "codex" | "claude_code" | "unknown";
  provider_id: string;
  account_id: string;
  model: string;
  project_path: string;
  precision: "" | PrecisionLevel;
  search: string;
};

// The date picker and its filters speak the user's *local* calendar day. A
// date-only string parsed without a timezone is interpreted as local midnight,
// so converting through Date yields the correct UTC instant to query — matching
// the tray/dashboard "今日" boundary the core now computes in local time.
function localDateInput(date = new Date()): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function localDayStartIso(dateString: string): string | null {
  const start = new Date(`${dateString}T00:00:00`);
  return Number.isNaN(start.valueOf()) ? null : start.toISOString();
}

// Turn an unknown thrown value into a message worth showing a user. The backend
// returns command errors as strings; native/network failures arrive as Error.
function describeError(cause: unknown): string {
  if (cause instanceof Error) return cause.message;
  if (typeof cause === "string" && cause.trim()) return cause;
  return "未知错误";
}

function initialDashboardFilterForm(): DashboardFilterForm {
  const today = localDateInput();
  return {
    period_start: today,
    period_end: today,
    app: "",
    provider_id: "",
    account_id: "",
    model: "",
    project_path: "",
    precision: "",
    search: "",
  };
}

function dashboardFilters(form: DashboardFilterForm): UsageFilters {
  const periodStart = form.period_start
    ? localDayStartIso(form.period_start)
    : null;
  let periodEnd: string | null = null;
  if (form.period_end) {
    const end = new Date(`${form.period_end}T00:00:00`);
    if (!Number.isNaN(end.valueOf())) {
      end.setDate(end.getDate() + 1);
      periodEnd = end.toISOString();
    }
  }
  return {
    period_start: periodStart,
    period_end: periodEnd,
    app: form.app || null,
    provider_id: form.provider_id.trim() || null,
    account_id: form.account_id.trim() || null,
    model: form.model.trim() || null,
    project_path: form.project_path.trim() || null,
    precision: form.precision || null,
    search: form.search.trim() || null,
  };
}

function App() {
  const pathname = usePathname();
  if (pathname === "/quick") return <QuickSummaryView />;
  if (pathname === "/providers") return <ProvidersView />;
  if (pathname === "/quotas") return <QuotasView />;
  if (pathname === "/settings") return <SettingsView />;
  if (pathname === "/sources") return <SourcesView />;
  if (pathname === "/sessions") return <SessionsView />;
  if (pathname.startsWith("/sessions/")) {
    return (
      <SessionRouteView
        sessionId={decodeURIComponent(pathname.slice("/sessions/".length))}
      />
    );
  }
  return <DashboardView />;
}

function usePathname(): string {
  const [pathname, setPathname] = useState(() => window.location.pathname);
  useEffect(() => {
    const handlePopState = () => setPathname(window.location.pathname);
    window.addEventListener("popstate", handlePopState);
    return () => window.removeEventListener("popstate", handlePopState);
  }, []);
  return pathname;
}

function navigate(path: string) {
  window.history.pushState({}, "", path);
  window.dispatchEvent(new PopStateEvent("popstate"));
}

function AppNavigation() {
  return (
    <nav className="route-nav" aria-label="主要导航">
      <RouteLink to="/dashboard">总览</RouteLink>
      <RouteLink to="/sessions">会话</RouteLink>
      <RouteLink to="/providers">Providers</RouteLink>
      <RouteLink to="/quotas">额度</RouteLink>
      <RouteLink to="/sources">数据源</RouteLink>
      <RouteLink to="/settings">设置</RouteLink>
    </nav>
  );
}

function RouteLink({ to, children }: { to: string; children: string }) {
  return (
    <a
      href={to}
      onClick={(event) => {
        if (event.button !== 0 || event.metaKey || event.ctrlKey) return;
        event.preventDefault();
        navigate(to);
      }}
    >
      {children}
    </a>
  );
}

function PageFrame({
  eyebrow,
  title,
  subtitle,
  children,
}: {
  eyebrow: string;
  title: string;
  subtitle: string;
  children: ReactNode;
}) {
  return (
    <main className="app-shell">
      <header className="page-header">
        <div>
          <p className="eyebrow">{eyebrow}</p>
          <h1>{title}</h1>
          <p className="subtitle">{subtitle}</p>
        </div>
        <AppNavigation />
      </header>
      {children}
    </main>
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
  const [claudeHome, setClaudeHome] = useState("");
  const [ccSwitchDb, setCcSwitchDb] = useState("");
  const [cockpitDb, setCockpitDb] = useState("");
  const [codexDetection, setCodexDetection] = useState<DetectionResult | null>(
    null,
  );
  const [claudeDetection, setClaudeDetection] =
    useState<DetectionResult | null>(null);
  const [ccSwitchDetection, setCcSwitchDetection] =
    useState<DetectionResult | null>(null);
  const [cockpitDetection, setCockpitDetection] =
    useState<DetectionResult | null>(null);
  const [status, setStatus] = useState("正在连接本地数据层…");
  const [error, setError] = useState<string | null>(null);
  const [isScanning, setIsScanning] = useState(false);
  const [refreshVersion, setRefreshVersion] = useState(0);
  const [filterForm, setFilterForm] = useState<DashboardFilterForm>(
    initialDashboardFilterForm,
  );
  const [exportingFormat, setExportingFormat] = useState<"csv" | "json" | null>(
    null,
  );
  const filters = useMemo(() => dashboardFilters(filterForm), [filterForm]);

  useEffect(() => {
    let active = true;

    async function loadOverview() {
      try {
        const [nextDashboard, nextSessions, nextSources] = await Promise.all([
          getDashboardSummary(filters),
          // The session list honors the same filters as the metric cards so the
          // two halves of the screen always tell the same story.
          listSessions(filters),
          listSources(),
        ]);
        if (!active) return;
        setDashboard(nextDashboard);
        setSessions(nextSessions.sessions);
        setSources(nextSources);
        setStatus("数据已从本地 SQLite 加载");
        setError(null);
      } catch (cause) {
        if (!active) return;
        console.error("加载总览失败", cause);
        if (isDesktopRuntime()) {
          setStatus("无法读取本地数据层");
          setError(`读取本地数据层失败：${describeError(cause)}`);
        } else {
          setStatus("请通过 Tauri 启动以连接本地数据层");
          setError("浏览器预览没有 Tauri IPC；桌面应用启动后会显示真实数据。");
        }
      }
    }

    void loadOverview();
    return () => {
      active = false;
    };
  }, [filters, refreshVersion]);

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
      } catch (cause) {
        console.error("读取会话详情失败", cause);
        if (active) setError(`无法读取会话详情：${describeError(cause)}`);
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
      setCodexDetection(nextDetection);
      setStatus(
        nextDetection.detected
          ? "已检测到 Codex Session 目录"
          : "未检测到 Codex Session 目录",
      );
      setError(null);
    } catch (cause) {
      console.error("检测 Codex Home 失败", cause);
      setError(`无法检测 Codex Home：${describeError(cause)}`);
    }
  }

  async function handleDetectClaude() {
    try {
      const nextDetection = await detectClaudePath(claudeHome.trim() || null);
      setClaudeDetection(nextDetection);
      setStatus(
        nextDetection.detected
          ? "已检测到 Claude Code projects 目录"
          : "未检测到 Claude Code projects 目录",
      );
      setError(null);
    } catch (cause) {
      console.error("检测 Claude Home 失败", cause);
      setError(`无法检测 Claude Home：${describeError(cause)}`);
    }
  }

  async function handleDetectCcSwitch() {
    try {
      const nextDetection = await detectCcSwitchPath(ccSwitchDb.trim() || null);
      setCcSwitchDetection(nextDetection);
      setStatus(
        nextDetection.detected
          ? "已检测到 CC-Switch 数据库"
          : "未检测到 CC-Switch 数据库",
      );
      setError(null);
    } catch (cause) {
      console.error("检测 CC-Switch 失败", cause);
      setError(`无法检测 CC-Switch：${describeError(cause)}`);
    }
  }

  async function handleDetectCockpit() {
    try {
      const nextDetection = await detectCockpitPath(cockpitDb.trim() || null);
      setCockpitDetection(nextDetection);
      setStatus(
        nextDetection.detected
          ? "已检测到 Cockpit 数据库"
          : "未检测到 Cockpit 数据库",
      );
      setError(null);
    } catch (cause) {
      console.error("检测 Cockpit 失败", cause);
      setError(`无法检测 Cockpit：${describeError(cause)}`);
    }
  }

  async function handleScan() {
    setIsScanning(true);
    // Scan each source independently so one source failing does not discard the
    // others' results or get misreported as the wrong source's failure.
    const [codexOutcome, claudeOutcome, ccSwitchOutcome, cockpitOutcome] =
      await Promise.allSettled([
        rescanCodex(codexHome.trim() || null),
        rescanClaude(claudeHome.trim() || null),
        rescanCcSwitch(ccSwitchDb.trim() || null),
        rescanCockpit(cockpitDb.trim() || null),
      ]);
    let inserted = 0;
    let skipped = 0;
    const problems: string[] = [];
    for (const [label, outcome] of [
      ["Codex", codexOutcome],
      ["Claude", claudeOutcome],
      ["CC-Switch", ccSwitchOutcome],
      ["Cockpit", cockpitOutcome],
    ] as const) {
      if (outcome.status === "fulfilled") {
        inserted += outcome.value.inserted_events;
        skipped += outcome.value.skipped_records;
        if (outcome.value.warning) {
          problems.push(`${label}：${outcome.value.warning}`);
        }
      } else {
        console.error(`${label} 扫描失败`, outcome.reason);
        problems.push(`${label} 扫描失败：${describeError(outcome.reason)}`);
      }
    }
    setStatus(`扫描完成：新增 ${inserted} 条事件，跳过 ${skipped} 条记录`);
    setError(problems.length ? problems.join("；") : null);
    setRefreshVersion((value) => value + 1);
    setIsScanning(false);
  }

  async function handleOpenWeb() {
    try {
      const result = await openLocalWebApi();
      setStatus(
        result.url ? `本地网页面板已启动：${result.url}` : "本地网页面板已启动",
      );
      setError(null);
    } catch (cause) {
      console.error("启动本地网页面板失败", cause);
      setError(`无法启动本地网页面板：${describeError(cause)}`);
    }
  }

  async function handleExport(format: "csv" | "json") {
    setExportingFormat(format);
    try {
      if (isDesktopRuntime()) {
        // WKWebView cannot trigger a blob download, so the desktop app writes
        // the file itself and tells the user where it landed.
        const savedPath = await saveExport(format, filters);
        setStatus(`已导出到 ${savedPath}`);
      } else {
        const result = await exportUsage(format, filters);
        const blob = new Blob([result.content], { type: result.mime_type });
        const url = URL.createObjectURL(blob);
        const link = document.createElement("a");
        link.href = url;
        link.download = result.filename;
        link.click();
        URL.revokeObjectURL(url);
        setStatus(`已导出 ${result.filename}`);
      }
      setError(null);
    } catch (cause) {
      console.error(`导出 ${format} 失败`, cause);
      setError(`无法导出 ${format.toUpperCase()}：${describeError(cause)}`);
    } finally {
      setExportingFormat(null);
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
            {isScanning ? "扫描中…" : "扫描全部来源"}
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
      <AppNavigation />

      {error ? <p className="notice notice-warning">{error}</p> : null}

      <section className="source-bar" aria-labelledby="source-heading">
        <div>
          <p className="section-kicker" id="source-heading">
            数据源
          </p>
          <p className="source-description">
            当前只读导入 Codex 与 Claude Session JSONL；未知值保持
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
            检测 Codex
          </button>
          <label htmlFor="claude-home">Claude Home</label>
          <input
            id="claude-home"
            value={claudeHome}
            onChange={(event) => setClaudeHome(event.target.value)}
            placeholder="留空使用系统默认路径"
          />
          <button
            className="quiet-button"
            type="button"
            onClick={handleDetectClaude}
          >
            检测 Claude
          </button>
          <label htmlFor="cc-switch-db">CC-Switch DB</label>
          <input
            id="cc-switch-db"
            value={ccSwitchDb}
            onChange={(event) => setCcSwitchDb(event.target.value)}
            placeholder="留空使用 ~/.cc-switch/cc-switch.db"
          />
          <button
            className="quiet-button"
            type="button"
            onClick={handleDetectCcSwitch}
          >
            检测 CC-Switch
          </button>
          <label htmlFor="cockpit-db">Cockpit DB</label>
          <input
            id="cockpit-db"
            value={cockpitDb}
            onChange={(event) => setCockpitDb(event.target.value)}
            placeholder="留空使用 ~/.antigravity_cockpit"
          />
          <button
            className="quiet-button"
            type="button"
            onClick={handleDetectCockpit}
          >
            检测 Cockpit
          </button>
        </div>
        <div className="source-detections">
          {codexDetection ? (
            <span
              className={codexDetection.detected ? "detection ok" : "detection"}
            >
              Codex {codexDetection.detected ? "Detected" : "Not found"}
            </span>
          ) : null}
          {claudeDetection ? (
            <span
              className={
                claudeDetection.detected ? "detection ok" : "detection"
              }
            >
              Claude {claudeDetection.detected ? "Detected" : "Not found"}
            </span>
          ) : null}
          {ccSwitchDetection ? (
            <span
              className={
                ccSwitchDetection.detected ? "detection ok" : "detection"
              }
            >
              CC-Switch {ccSwitchDetection.detected ? "Detected" : "Not found"}
            </span>
          ) : null}
          {cockpitDetection ? (
            <span
              className={
                cockpitDetection.detected ? "detection ok" : "detection"
              }
            >
              Cockpit {cockpitDetection.detected ? "Detected" : "Not found"}
            </span>
          ) : null}
        </div>
      </section>

      <section
        className="panel filters-panel"
        aria-labelledby="filters-heading"
      >
        <div className="panel-heading filters-heading">
          <div>
            <p className="section-kicker" id="filters-heading">
              Filter & export
            </p>
            <h2>统计筛选</h2>
          </div>
          <div className="filter-actions">
            <button
              className="quiet-button"
              type="button"
              onClick={() => setFilterForm(initialDashboardFilterForm())}
            >
              清除筛选
            </button>
            <button
              className="quiet-button"
              type="button"
              onClick={() => void handleExport("csv")}
              disabled={exportingFormat !== null}
            >
              {exportingFormat === "csv" ? "导出中…" : "导出 CSV"}
            </button>
            <button
              className="primary-button"
              type="button"
              onClick={() => void handleExport("json")}
              disabled={exportingFormat !== null}
            >
              {exportingFormat === "json" ? "导出中…" : "导出 JSON"}
            </button>
          </div>
        </div>
        <div className="filters-grid">
          <label>
            <span>开始日期</span>
            <input
              type="date"
              value={filterForm.period_start}
              onChange={(event) =>
                setFilterForm({
                  ...filterForm,
                  period_start: event.target.value,
                })
              }
            />
          </label>
          <label>
            <span>结束日期</span>
            <input
              type="date"
              value={filterForm.period_end}
              onChange={(event) =>
                setFilterForm({ ...filterForm, period_end: event.target.value })
              }
            />
          </label>
          <label>
            <span>应用</span>
            <select
              value={filterForm.app}
              onChange={(event) =>
                setFilterForm({
                  ...filterForm,
                  app: event.target.value as DashboardFilterForm["app"],
                })
              }
            >
              <option value="">全部</option>
              <option value="codex">Codex</option>
              <option value="claude_code">Claude Code</option>
              <option value="unknown">Unknown</option>
            </select>
          </label>
          <label>
            <span>精度</span>
            <select
              value={filterForm.precision}
              onChange={(event) =>
                setFilterForm({
                  ...filterForm,
                  precision: event.target
                    .value as DashboardFilterForm["precision"],
                })
              }
            >
              <option value="">全部</option>
              <option value="verified">Verified</option>
              <option value="exact_session">Exact session</option>
              <option value="correlated">Correlated</option>
              <option value="estimated">Estimated</option>
              <option value="unavailable">Unavailable</option>
            </select>
          </label>
          <label>
            <span>Provider ID</span>
            <input
              value={filterForm.provider_id}
              onChange={(event) =>
                setFilterForm({
                  ...filterForm,
                  provider_id: event.target.value,
                })
              }
              placeholder="精确匹配"
            />
          </label>
          <label>
            <span>Account ID</span>
            <input
              value={filterForm.account_id}
              onChange={(event) =>
                setFilterForm({ ...filterForm, account_id: event.target.value })
              }
              placeholder="精确匹配"
            />
          </label>
          <label>
            <span>Model</span>
            <input
              value={filterForm.model}
              onChange={(event) =>
                setFilterForm({ ...filterForm, model: event.target.value })
              }
              placeholder="包含匹配"
            />
          </label>
          <label>
            <span>项目路径</span>
            <input
              value={filterForm.project_path}
              onChange={(event) =>
                setFilterForm({
                  ...filterForm,
                  project_path: event.target.value,
                })
              }
              placeholder="包含匹配"
            />
          </label>
          <label className="filter-search">
            <span>搜索</span>
            <input
              value={filterForm.search}
              onChange={(event) =>
                setFilterForm({ ...filterForm, search: event.target.value })
              }
              placeholder="标题、项目、会话 ID、模型或请求 ID"
            />
          </label>
        </div>
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
              description="确认 Codex 或 Claude Home 后点击“扫描 Codex + Claude”，TokenBuddy 会从 JSONL 增量导入。"
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

function ProvidersView() {
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void listProviders()
      .then((nextProviders) => {
        if (active) {
          setProviders(nextProviders);
          setError(null);
        }
      })
      .catch(() => {
        if (active) setError("无法读取 Provider 统计。");
      });
    return () => {
      active = false;
    };
  }, []);

  return (
    <PageFrame
      eyebrow="Provider observatory"
      title="Providers"
      subtitle="只展示已被数据源明确识别的 Provider；无法归属时保持 Unavailable。"
    >
      {error ? <p className="notice notice-warning">{error}</p> : null}
      {providers.length ? (
        <section className="route-grid" aria-label="Provider 统计">
          {providers.map((provider) => (
            <article className="panel route-card" key={provider.id}>
              <div className="panel-heading route-card-heading">
                <div>
                  <p className="section-kicker">{provider.provider_family}</p>
                  <h2>{provider.display_name}</h2>
                </div>
                <span className="count-label">
                  {formatTokens(provider.request_count)} 请求
                </span>
              </div>
              <dl className="summary-list">
                <SummaryItem
                  label="上游 URL"
                  value={provider.upstream_url || "Unavailable"}
                />
                <SummaryItem
                  label="账号数"
                  value={formatTokens(provider.account_count)}
                />
                <SummaryItem
                  label="成功率"
                  value={formatPercent(provider.success_rate_percent)}
                />
                <SummaryItem
                  label="平均延迟"
                  value={
                    provider.average_latency_ms == null
                      ? "Unavailable"
                      : `${provider.average_latency_ms.toFixed(0)} ms`
                  }
                />
                <SummaryItem
                  label="输入 / 输出"
                  value={`${formatTokens(provider.totals.input_tokens_total)} / ${formatTokens(provider.totals.output_tokens_total)}`}
                />
                <SummaryItem label="费用" value={formatCost(provider.totals)} />
              </dl>
            </article>
          ))}
        </section>
      ) : (
        <section className="panel route-panel">
          <EmptyState
            title="Provider 数据 Unavailable"
            description="当前已导入 Codex 与 Claude Code Session；Provider Adapter 尚未提供可验证归属。"
          />
        </section>
      )}
    </PageFrame>
  );
}

function QuotasView() {
  const [quotas, setQuotas] = useState<QuotaSnapshot[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void listQuotaSnapshots()
      .then((nextQuotas) => {
        if (active) {
          setQuotas(nextQuotas);
          setError(null);
        }
      })
      .catch(() => {
        if (active) setError("无法读取官方额度快照。");
      });
    return () => {
      active = false;
    };
  }, []);

  return (
    <PageFrame
      eyebrow="Official quota windows"
      title="官方额度"
      subtitle="额度窗口与原始 Token 分开保存；不会从百分比反推准确 Token。"
    >
      {error ? <p className="notice notice-warning">{error}</p> : null}
      {quotas.length ? (
        <section className="panel route-panel" aria-label="官方额度快照">
          <div className="quota-list">
            {quotas.map((quota) => (
              <article className="quota-row" key={quota.id}>
                <div>
                  <strong>{quota.window_type}</strong>
                  <span>
                    {quota.provider_name || "Provider Unavailable"} ·{" "}
                    {quota.account_name || "账号 Unavailable"}
                  </span>
                </div>
                <div>
                  <strong>{formatPercent(quota.used_percent)} 已用</strong>
                  <span>
                    剩余 {formatPercent(quota.remaining_percent)} ·{" "}
                    {precisionLabel(quota.precision)}
                  </span>
                </div>
                <div>
                  <strong>{formatDate(quota.captured_at)}</strong>
                  <span>
                    重置{" "}
                    {quota.reset_at
                      ? formatDate(quota.reset_at)
                      : "Unavailable"}
                  </span>
                </div>
              </article>
            ))}
          </div>
        </section>
      ) : (
        <section className="panel route-panel">
          <EmptyState
            title="官方额度 Unavailable"
            description="尚未连接官方额度数据源；此处不会使用 Session Token 估算订阅额度。"
          />
        </section>
      )}
    </PageFrame>
  );
}

function SourcesView() {
  const [sources, setSources] = useState<SourceRecord[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void listSources()
      .then((nextSources) => {
        if (active) {
          setSources(nextSources);
          setError(null);
        }
      })
      .catch(() => {
        if (active) setError("无法读取数据源状态。");
      });
    return () => {
      active = false;
    };
  }, []);

  return (
    <PageFrame
      eyebrow="Read-only adapters"
      title="数据源"
      subtitle="每个 Adapter 独立报告路径、健康状态和最近错误。"
    >
      {error ? <p className="notice notice-warning">{error}</p> : null}
      {sources.length ? (
        <section className="route-grid" aria-label="数据源状态">
          {sources.map((source) => (
            <article className="panel route-card" key={source.id}>
              <div className="panel-heading route-card-heading">
                <div>
                  <p className="section-kicker">{source.adapter_type}</p>
                  <h2>{source.display_name}</h2>
                </div>
                <span className="detection ok">
                  {source.health_status || "Unavailable"}
                </span>
              </div>
              <dl className="summary-list">
                <SummaryItem
                  label="检测路径"
                  value={source.path_or_endpoint || "Unavailable"}
                />
                <SummaryItem
                  label="版本"
                  value={source.detected_version || "Unavailable"}
                />
                <SummaryItem
                  label="最近导入"
                  value={
                    source.last_success_at
                      ? formatDate(source.last_success_at)
                      : "Unavailable"
                  }
                />
                <SummaryItem
                  label="最近错误"
                  value={source.last_error || "Unavailable"}
                />
              </dl>
            </article>
          ))}
        </section>
      ) : (
        <section className="panel route-panel">
          <EmptyState
            title="尚未登记数据源"
            description="启动 Core 后会在此展示 Adapter 状态。"
          />
        </section>
      )}
    </PageFrame>
  );
}

function SessionsView() {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void listSessions({}, 100, 0)
      .then((page) => {
        if (active) {
          setSessions(page.sessions);
          setError(null);
        }
      })
      .catch((cause: unknown) => {
        console.error("读取会话列表失败", cause);
        if (active) setError(`无法读取会话列表：${describeError(cause)}`);
      });
    return () => {
      active = false;
    };
  }, []);

  return (
    <PageFrame
      eyebrow="Session history"
      title="会话"
      subtitle="从会话追踪到请求级 Token，精度和缺失值始终可见。"
    >
      {error ? <p className="notice notice-warning">{error}</p> : null}
      <section
        className="panel sessions-panel route-panel"
        aria-label="会话列表"
      >
        {sessions.length ? (
          <div className="session-list">
            {sessions.map((session) => (
              <SessionRow
                key={session.session.id}
                summary={session}
                selected={false}
                onSelect={() =>
                  navigate(
                    `/sessions/${encodeURIComponent(session.session.id)}`,
                  )
                }
              />
            ))}
          </div>
        ) : (
          <EmptyState
            title="还没有导入会话"
            description="确认 Codex Home 后开始增量导入。"
          />
        )}
      </section>
    </PageFrame>
  );
}

function SessionRouteView({ sessionId }: { sessionId: string }) {
  const [detail, setDetail] = useState<SessionDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void getSessionDetail(sessionId)
      .then((nextDetail) => {
        if (active) {
          setDetail(nextDetail);
          setError(null);
        }
      })
      .catch(() => {
        if (active) setError("无法读取会话详情。");
      });
    return () => {
      active = false;
    };
  }, [sessionId]);

  return (
    <PageFrame
      eyebrow="Session detail"
      title="会话详情"
      subtitle="请求时间线只来自 Core 查询服务，不重新扫描原始日志。"
    >
      <p className="route-back">
        <RouteLink to="/sessions">← 返回会话列表</RouteLink>
      </p>
      {error ? <p className="notice notice-warning">{error}</p> : null}
      {detail ? (
        <section className="panel detail-panel route-panel">
          <SessionDetailView detail={detail} />
        </section>
      ) : (
        <section className="panel route-panel">
          <EmptyState
            title="会话 Unavailable"
            description="Core 没有返回该会话。"
          />
        </section>
      )}
    </PageFrame>
  );
}

const defaultAppSettings: AppSettings = {
  codex_home: null,
  claude_home: null,
  cc_switch_db_path: null,
  cockpit_path: null,
  otel_port: null,
  auto_start: false,
  proxy_enabled: false,
  save_request_metadata: false,
  data_retention_days: null,
};

function SettingsView() {
  const [settings, setSettings] = useState<AppSettings>(defaultAppSettings);
  const [status, setStatus] = useState("正在读取设置…");
  const [error, setError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    let active = true;
    void getAppSettings()
      .then((nextSettings) => {
        if (active) {
          setSettings(nextSettings);
          setStatus("设置已加载");
          setError(null);
        }
      })
      .catch(() => {
        if (active) {
          setStatus("设置不可用");
          setError("无法读取 Core 设置。");
        }
      });
    return () => {
      active = false;
    };
  }, []);

  async function handleSave() {
    setIsSaving(true);
    try {
      const nextSettings = await updateAppSettings({
        ...settings,
        codex_home: settings.codex_home?.trim() || null,
        claude_home: settings.claude_home?.trim() || null,
        cc_switch_db_path: settings.cc_switch_db_path?.trim() || null,
        cockpit_path: settings.cockpit_path?.trim() || null,
      });
      setSettings(nextSettings);
      setStatus("设置已保存");
      setError(null);
    } catch (cause) {
      console.error("保存设置失败", cause);
      setError(`设置保存失败：${describeError(cause)}`);
    } finally {
      setIsSaving(false);
    }
  }

  return (
    <PageFrame
      eyebrow="Local configuration"
      title="设置"
      subtitle="Codex 与 Claude Code Session 路径由 Core 持久化并自动增量导入；其他 Adapter 保持 Unavailable。"
    >
      {error ? <p className="notice notice-warning">{error}</p> : null}
      <section className="panel settings-panel" aria-label="应用设置">
        <div className="settings-heading">
          <div>
            <p className="section-kicker">采集路径</p>
            <h2>数据源路径</h2>
          </div>
          <span
            className="status-pill"
            data-state={error ? "warning" : "ready"}
          >
            {status}
          </span>
        </div>
        <div className="settings-grid">
          <SettingsField
            id="settings-codex-home"
            label="Codex Home"
            value={settings.codex_home}
            onChange={(value) =>
              setSettings({ ...settings, codex_home: value })
            }
            placeholder="留空使用系统默认路径"
          />
          <SettingsField
            id="settings-claude-home"
            label="Claude Home"
            value={settings.claude_home}
            onChange={(value) =>
              setSettings({ ...settings, claude_home: value })
            }
            placeholder="留空使用系统默认路径"
          />
          <SettingsField
            id="settings-cc-switch"
            label="CC Switch DB"
            value={settings.cc_switch_db_path}
            onChange={(value) =>
              setSettings({ ...settings, cc_switch_db_path: value })
            }
            placeholder="Unavailable（只读 Adapter 尚未启用）"
          />
          <SettingsField
            id="settings-cockpit"
            label="Cockpit 数据路径"
            value={settings.cockpit_path}
            onChange={(value) =>
              setSettings({ ...settings, cockpit_path: value })
            }
            placeholder="Unavailable（只读 Adapter 尚未启用）"
          />
        </div>
        <div className="settings-flags">
          <label>
            <input
              type="checkbox"
              checked={settings.auto_start}
              onChange={(event) =>
                setSettings({ ...settings, auto_start: event.target.checked })
              }
            />
            开机自动启动（修改后立即生效）
          </label>
          <label>
            <input
              type="checkbox"
              checked={settings.proxy_enabled}
              disabled
              readOnly
            />
            允许本地代理（Phase 7，当前关闭）
          </label>
        </div>
        <button
          className="primary-button"
          type="button"
          onClick={handleSave}
          disabled={isSaving}
        >
          {isSaving ? "保存中…" : "保存设置"}
        </button>
      </section>
    </PageFrame>
  );
}

function SettingsField({
  id,
  label,
  value,
  onChange,
  placeholder,
}: {
  id: string;
  label: string;
  value: string | null;
  onChange: (value: string) => void;
  placeholder: string;
}) {
  return (
    <label className="settings-field" htmlFor={id}>
      <span>{label}</span>
      <input
        id={id}
        value={value ?? ""}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
      />
    </label>
  );
}

function SummaryItem({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}

function QuickSummaryView() {
  const [summary, setSummary] = useState<QuickSummary | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    document.documentElement.classList.add("quick-window");
    document.body.classList.add("quick-window");
    let active = true;

    async function loadSummary() {
      try {
        const nextSummary = await getQuickSummary();
        if (!active) return;
        setSummary(nextSummary);
        setError(null);
      } catch (cause) {
        console.error("读取 Core 摘要失败", cause);
        if (active) setError(describeError(cause));
      }
    }

    void loadSummary();
    const timer = window.setInterval(() => {
      void loadSummary();
    }, 2000);
    return () => {
      active = false;
      window.clearInterval(timer);
      document.documentElement.classList.remove("quick-window");
      document.body.classList.remove("quick-window");
    };
  }, []);

  const quota = summary?.quota_summary;
  const status = summary?.collection_status ?? "starting";
  const sessionSubtitle =
    [summary?.active_app, summary?.provider_name, summary?.model]
      .filter(Boolean)
      .join(" · ") || "Unavailable";
  const desktop = isDesktopRuntime();

  return (
    <div className="menu-shell">
      <div
        className="menu-surface"
        role="menu"
        aria-label="TokenBuddy 快速摘要"
      >
        <header className="menu-header">
          <span className="menu-title">TokenBuddy</span>
          <span className={`menu-status menu-status-${status}`}>
            <span className="menu-status-dot" aria-hidden="true" />
            {collectionStatusLabel(summary?.collection_status)}
          </span>
        </header>

        {error ? (
          <div className="menu-note menu-note-warning">
            无法读取后台 Core：{error}
          </div>
        ) : null}

        <MenuSeparator />
        <MenuGroupTitle>今日</MenuGroupTitle>
        <MenuRow
          glyph="chart"
          tint="blue"
          label="今日 Token"
          sublabel="输入 + 输出 · 未知不折算成 0"
          trailing={
            <span className="menu-hero-value">
              {formatTokens(summary?.today_total_tokens)}
            </span>
          }
        />

        <MenuSeparator />
        <MenuGroupTitle>最近活动会话</MenuGroupTitle>
        <MenuRow
          glyph="session"
          tint="mint"
          label={summary?.active_session_title || "Unavailable"}
          sublabel={sessionSubtitle}
        />
        <MenuValueRow
          label="输入"
          value={formatTokens(summary?.session_input_tokens)}
        />
        <MenuValueRow
          label="缓存读取"
          value={formatTokens(summary?.session_cache_read_tokens)}
        />
        <MenuValueRow
          label="输出"
          value={formatTokens(summary?.session_output_tokens)}
        />
        <MenuValueRow
          label="缓存命中率"
          value={formatPercent(summary?.session_cache_hit_rate ?? null)}
        />
        {summary?.active_project_path ? (
          <p className="menu-caption">项目：{summary.active_project_path}</p>
        ) : null}

        <MenuSeparator />
        <MenuGroupTitle>官方额度</MenuGroupTitle>
        <MenuRow
          glyph="gauge"
          tint="amber"
          label={
            quota ? `${formatPercent(quota.used_percent)} 已用` : "Unavailable"
          }
          sublabel={
            quota
              ? `${quota.window_type} · ${quota.precision}`
              : "未接入额度 API"
          }
        />

        {summary?.latest_warning ? (
          <div className="menu-note menu-note-warning">
            {summary.latest_warning}
          </div>
        ) : null}

        {desktop ? (
          <>
            <MenuSeparator />
            <button
              className="menu-action"
              type="button"
              onClick={() => {
                void showMainWindow().catch((cause: unknown) =>
                  console.error("打开完整面板失败", cause),
                );
              }}
            >
              打开完整面板…
            </button>
            <button
              className="menu-action"
              type="button"
              onClick={() => {
                void openLocalWebApi().catch((cause: unknown) =>
                  console.error("打开本地网页面板失败", cause),
                );
              }}
            >
              打开本地网页面板…
            </button>
          </>
        ) : null}
      </div>
    </div>
  );
}

function MenuSeparator() {
  return <div className="menu-separator" role="separator" />;
}

function MenuGroupTitle({ children }: { children: string }) {
  return <p className="menu-group-title">{children}</p>;
}

function MenuValueRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="menu-row menu-row-compact">
      <span className="menu-row-label">{label}</span>
      <span className="menu-value">{value}</span>
    </div>
  );
}

type MenuGlyphName = "chart" | "session" | "gauge";

function MenuRow({
  glyph,
  tint,
  label,
  sublabel,
  trailing,
}: {
  glyph: MenuGlyphName;
  tint: "blue" | "mint" | "amber";
  label: string;
  sublabel?: string;
  trailing?: ReactNode;
}) {
  return (
    <div className="menu-row">
      <span className={`menu-glyph menu-glyph-${tint}`} aria-hidden="true">
        <MenuGlyph name={glyph} />
      </span>
      <span className="menu-row-body">
        <span className="menu-row-label">{label}</span>
        {sublabel ? <span className="menu-row-sub">{sublabel}</span> : null}
      </span>
      {trailing ? <span className="menu-row-trailing">{trailing}</span> : null}
    </div>
  );
}

function MenuGlyph({ name }: { name: MenuGlyphName }) {
  if (name === "chart") {
    return (
      <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
        <path
          d="M2.5 13.5h11M4.75 11V7.5M8 11V4.5M11.25 11V8.5"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinecap="round"
        />
      </svg>
    );
  }
  if (name === "session") {
    return (
      <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
        <path
          d="M3 3.5h10a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H7l-3 2.4V10.5H3a1 1 0 0 1-1-1v-5a1 1 0 0 1 1-1Z"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.4"
          strokeLinejoin="round"
        />
      </svg>
    );
  }
  return (
    <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
      <path
        d="M3 12a5 5 0 1 1 10 0M8 8.5l2.6-2.6"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </svg>
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
