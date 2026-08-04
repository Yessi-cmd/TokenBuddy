import { useEffect, useMemo, useState } from "react";

import {
  detectCcSwitchPath,
  detectClaudePath,
  detectCockpitPath,
  detectCodexPath,
  exportUsage,
  getDashboardSummary,
  getModelBreakdown,
  getSessionDetail,
  isDesktopRuntime,
  listSessions,
  listSources,
  openLocalWebApi,
  rescanCcSwitch,
  rescanClaude,
  rescanCockpit,
  rescanCodex,
  saveExport,
  type DashboardSummary,
  type DetectionResult,
  type ModelUsage,
  type SessionDetail,
  type SessionSummary,
  type SourceRecord,
} from "../../lib/api";
import { AppNavigation } from "../../components/Navigation";
import {
  EmptyState,
  MetricCard,
  SessionDetailView,
  SessionRow,
} from "../../components/Presentation";
import {
  dashboardFilters,
  emptyTotals,
  initialDashboardFilterForm,
  type DashboardFilterForm,
} from "../../lib/filters";
import {
  describeError,
  formatCost,
  formatPercent,
  formatTokens,
} from "../../lib/format";

export function DashboardView() {
  const [dashboard, setDashboard] = useState<DashboardSummary | null>(null);
  const [breakdown, setBreakdown] = useState<ModelUsage[]>([]);
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
  // Two error slots on purpose. The overview reloads every few seconds and on
  // every scan, and its success path used to clear whatever was on screen —
  // which erased the result of the action that triggered the reload before the
  // user could read it. Loading owns one slot, user actions own the other.
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const error = actionError ?? loadError;
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
        const [nextDashboard, nextBreakdown, nextSessions, nextSources] =
          await Promise.all([
            getDashboardSummary(filters),
            getModelBreakdown(filters),
            // The session list honors the same filters as the metric cards so
            // the two halves of the screen always tell the same story.
            listSessions(filters),
            listSources(),
          ]);
        if (!active) return;
        setDashboard(nextDashboard);
        setBreakdown(nextBreakdown);
        setSessions(nextSessions.sessions);
        setSources(nextSources);
        setStatus("数据已从本地 SQLite 加载");
        setLoadError(null);
      } catch (cause) {
        if (!active) return;
        console.error("加载总览失败", cause);
        if (isDesktopRuntime()) {
          setStatus("无法读取本地数据层");
          setLoadError(`读取本地数据层失败：${describeError(cause)}`);
        } else {
          setStatus("请通过 Tauri 启动以连接本地数据层");
          setLoadError(
            "浏览器预览没有 Tauri IPC；桌面应用启动后会显示真实数据。",
          );
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
        if (active) setActionError(`无法读取会话详情：${describeError(cause)}`);
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
      setActionError(null);
    } catch (cause) {
      console.error("检测 Codex Home 失败", cause);
      setActionError(`无法检测 Codex Home：${describeError(cause)}`);
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
      setActionError(null);
    } catch (cause) {
      console.error("检测 Claude Home 失败", cause);
      setActionError(`无法检测 Claude Home：${describeError(cause)}`);
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
      setActionError(null);
    } catch (cause) {
      console.error("检测 CC-Switch 失败", cause);
      setActionError(`无法检测 CC-Switch：${describeError(cause)}`);
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
      setActionError(null);
    } catch (cause) {
      console.error("检测 Cockpit 失败", cause);
      setActionError(`无法检测 Cockpit：${describeError(cause)}`);
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
    let reconciled = 0;
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
        reconciled += outcome.value.reconciled_events ?? 0;
        skipped += outcome.value.skipped_records;
        if (outcome.value.warning) {
          problems.push(`${label}：${outcome.value.warning}`);
        }
      } else {
        console.error(`${label} 扫描失败`, outcome.reason);
        problems.push(`${label} 扫描失败：${describeError(outcome.reason)}`);
      }
    }
    setStatus(
      `扫描完成：新增 ${inserted} 条事件，校正 ${reconciled} 条，跳过 ${skipped} 条记录`,
    );
    setActionError(problems.length ? problems.join("；") : null);
    setRefreshVersion((value) => value + 1);
    setIsScanning(false);
  }

  async function handleOpenWeb() {
    try {
      const result = await openLocalWebApi();
      setStatus(
        result.url ? `本地网页面板已启动：${result.url}` : "本地网页面板已启动",
      );
      setActionError(null);
    } catch (cause) {
      console.error("启动本地网页面板失败", cause);
      setActionError(`无法启动本地网页面板：${describeError(cause)}`);
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
      setActionError(null);
    } catch (cause) {
      console.error(`导出 ${format} 失败`, cause);
      setActionError(
        `无法导出 ${format.toUpperCase()}：${describeError(cause)}`,
      );
    } finally {
      setExportingFormat(null);
    }
  }

  const hasSourceDetections = Boolean(
    codexDetection || claudeDetection || ccSwitchDetection || cockpitDetection,
  );

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
        <div className="source-description-block">
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
        {hasSourceDetections ? (
          <div className="source-detections">
            {codexDetection ? (
              <span
                className={
                  codexDetection.detected ? "detection ok" : "detection"
                }
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
                CC-Switch{" "}
                {ccSwitchDetection.detected ? "Detected" : "Not found"}
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
        ) : null}
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

      <section className="panel" aria-labelledby="breakdown-heading">
        <div className="panel-heading">
          <div>
            <p className="section-kicker" id="breakdown-heading">
              Model & provider
            </p>
            <h2>按模型 / 供应商</h2>
          </div>
        </div>
        {breakdown.length ? (
          <div className="table-scroll">
            <table className="breakdown-table">
              <thead>
                <tr>
                  <th scope="col">模型</th>
                  <th scope="col">供应商</th>
                  <th scope="col">应用</th>
                  <th scope="col">输入</th>
                  <th scope="col">输出</th>
                  <th scope="col">缓存命中率</th>
                  <th scope="col">事件</th>
                  <th scope="col">费用</th>
                </tr>
              </thead>
              <tbody>
                {breakdown.map((row) => (
                  <tr
                    key={`${row.model ?? "-"}|${row.provider_id ?? "-"}|${row.app}`}
                  >
                    <td>{row.model || "模型 Unavailable"}</td>
                    <td>
                      {row.provider_name ||
                        row.provider_id ||
                        "供应商 Unavailable"}
                    </td>
                    <td>{row.app}</td>
                    <td>{formatTokens(row.totals.input_tokens_total)}</td>
                    <td>{formatTokens(row.totals.output_tokens_total)}</td>
                    <td>{formatPercent(row.totals.cache_hit_rate_percent)}</td>
                    <td>{formatTokens(row.totals.event_count)}</td>
                    <td>{formatCost(row.totals)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <p className="breakdown-empty">当前筛选下没有用量记录。</p>
        )}
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
