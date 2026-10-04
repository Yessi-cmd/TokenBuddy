import { useEffect, useMemo, useState, type CSSProperties } from "react";
import { startPolling } from "../../lib/polling";

import {
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
  rescanDsh,
  saveExport,
  type DashboardSummary,
  type ModelUsage,
  type SessionDetail,
  type SessionSummary,
  type SourceRecord,
} from "../../lib/api";
import { PageFrame, RouteLink } from "../../components/Navigation";
import {
  EmptyState,
  Meter,
  MetricCard,
  Notice,
  SessionDetailView,
  SessionRow,
} from "../../components/Presentation";
import {
  activePreset,
  advancedFilterKeys,
  dashboardFilters,
  datePresets,
  emptyTotals,
  initialDashboardFilterForm,
  presetRange,
  type DashboardFilterForm,
} from "../../lib/filters";
import {
  appLabel,
  describeError,
  formatCost,
  formatPercent,
  formatTokens,
} from "../../lib/format";
import { toast } from "../../lib/toast";

export function DashboardView() {
  const [dashboard, setDashboard] = useState<DashboardSummary | null>(null);
  const [breakdown, setBreakdown] = useState<ModelUsage[]>([]);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [sessionTotal, setSessionTotal] = useState<number | null>(null);
  const [sources, setSources] = useState<SourceRecord[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(
    null,
  );
  const [detail, setDetail] = useState<SessionDetail | null>(null);
  // The pill only describes the data layer. Results of clicks are toasts, so a
  // finished scan can no longer hide (or be truncated by) the connection state.
  const [status, setStatus] = useState("正在连接本地数据层…");
  // Two error slots on purpose. The overview reloads every few seconds and on
  // every scan, and its success path used to clear whatever was on screen —
  // which erased the result of the action that triggered the reload before the
  // user could read it. Loading owns one slot, user actions own the other.
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [isScanning, setIsScanning] = useState(false);
  const [refreshVersion, setRefreshVersion] = useState(0);
  const [filterForm, setFilterForm] = useState<DashboardFilterForm>(
    initialDashboardFilterForm,
  );
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [exportingFormat, setExportingFormat] = useState<"csv" | "json" | null>(
    null,
  );
  const filters = useMemo(() => dashboardFilters(filterForm), [filterForm]);
  const desktop = isDesktopRuntime();

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
        setSessionTotal(nextSessions.total);
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

    const stop = startPolling(loadOverview, 5000);
    return () => {
      active = false;
      stop();
    };
  }, [filters, refreshVersion]);

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

  const loading = dashboard === null && loadError === null;
  const totals = dashboard?.totals ?? emptyTotals;
  const selectedSession = useMemo(
    () =>
      sessions.find((item) => item.session.id === selectedSessionId) ?? null,
    [selectedSessionId, sessions],
  );
  const visibleDetail =
    detail?.summary.session.id === selectedSessionId ? detail : null;

  const advancedCount = advancedFilterKeys.filter(
    (key) => filterForm[key].trim() !== "",
  ).length;
  const initialForm = initialDashboardFilterForm();
  const hasActiveFilters = (
    Object.keys(initialForm) as (keyof DashboardFilterForm)[]
  ).some((key) => filterForm[key] !== initialForm[key]);
  const currentPreset = activePreset(filterForm);
  const advancedOpen = showAdvanced || advancedCount > 0;

  function updateFilter<K extends keyof DashboardFilterForm>(
    key: K,
    value: DashboardFilterForm[K],
  ) {
    setFilterForm((current) => ({ ...current, [key]: value }));
  }

  async function handleScan() {
    setIsScanning(true);
    // Every source scans with the paths saved in Settings. Scan each source
    // independently so one source failing does not discard the others' results
    // or get misreported as the wrong source's failure.
    const [
      codexOutcome,
      claudeOutcome,
      ccSwitchOutcome,
      cockpitOutcome,
      dshOutcome,
    ] = await Promise.allSettled([
      rescanCodex(null),
      rescanClaude(null),
      rescanCcSwitch(null),
      rescanCockpit(null),
      rescanDsh(null),
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
      ["DeepSeek Harness", dshOutcome],
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
    toast(
      `扫描完成：新增 ${inserted} 条事件，校正 ${reconciled} 条，跳过 ${skipped} 条记录`,
      problems.length ? "warning" : "success",
    );
    setActionError(problems.length ? problems.join("；") : null);
    setRefreshVersion((value) => value + 1);
    setIsScanning(false);
  }

  async function handleOpenWeb() {
    try {
      const result = await openLocalWebApi();
      toast(
        result.url ? `本地网页面板已启动：${result.url}` : "本地网页面板已启动",
        "info",
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
        toast(`已导出到 ${savedPath}`);
      } else {
        const result = await exportUsage(format, filters);
        const blob = new Blob([result.content], { type: result.mime_type });
        const url = URL.createObjectURL(blob);
        const link = document.createElement("a");
        link.href = url;
        link.download = result.filename;
        link.click();
        URL.revokeObjectURL(url);
        toast(`已导出 ${result.filename}`);
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

  // Share bars in the breakdown are relative to the busiest row; a row whose
  // tokens are unknown gets no bar rather than an empty one.
  const rowTokens = (row: ModelUsage) =>
    row.totals.input_tokens_total == null ||
    row.totals.output_tokens_total == null
      ? null
      : row.totals.input_tokens_total + row.totals.output_tokens_total;
  const maxRowTokens = Math.max(
    0,
    ...breakdown.map((row) => rowTokens(row) ?? 0),
  );

  return (
    <PageFrame
      label="总览工具栏"
      busy={isScanning}
      actions={
        <>
          <span
            className="status-pill"
            data-state={loadError ? "warning" : loading ? "loading" : "ready"}
            title={status}
          >
            <span className="status-dot" aria-hidden="true" />
            {status}
          </span>
          {desktop ? (
            <button
              className="quiet-button"
              type="button"
              onClick={handleOpenWeb}
            >
              本地网页
            </button>
          ) : null}
          <button
            className="primary-button scan-button"
            type="button"
            onClick={handleScan}
            disabled={isScanning}
            data-busy={isScanning || undefined}
          >
            <ScanGlyph />
            {isScanning ? "扫描中…" : "扫描全部来源"}
          </button>
        </>
      }
    >
      {loadError ? <Notice>{loadError}</Notice> : null}
      {actionError ? (
        <Notice onDismiss={() => setActionError(null)}>{actionError}</Notice>
      ) : null}

      <section className="panel scope-bar" aria-label="统计筛选">
        <div className="scope-row">
          <div className="segmented" role="group" aria-label="快捷时间范围">
            {datePresets.map((preset) => (
              <button
                key={preset.label}
                type="button"
                aria-pressed={currentPreset === preset.days}
                onClick={() =>
                  setFilterForm((current) => ({
                    ...current,
                    ...presetRange(preset.days),
                  }))
                }
              >
                {preset.label}
              </button>
            ))}
          </div>
          <div className="date-range">
            <label>
              <span className="sr-only">开始日期</span>
              <input
                type="date"
                value={filterForm.period_start}
                max={filterForm.period_end || undefined}
                onChange={(event) =>
                  updateFilter("period_start", event.target.value)
                }
              />
            </label>
            <span className="date-range-sep" aria-hidden="true">
              →
            </span>
            <label>
              <span className="sr-only">结束日期</span>
              <input
                type="date"
                value={filterForm.period_end}
                min={filterForm.period_start || undefined}
                onChange={(event) =>
                  updateFilter("period_end", event.target.value)
                }
              />
            </label>
          </div>
          <label className="scope-select">
            <span className="sr-only">应用</span>
            <select
              value={filterForm.app}
              onChange={(event) =>
                updateFilter(
                  "app",
                  event.target.value as DashboardFilterForm["app"],
                )
              }
            >
              <option value="">全部应用</option>
              <option value="codex">Codex</option>
              <option value="claude_code">Claude Code</option>
              <option value="open_code">OpenCode</option>
              <option value="deepseek_harness">DeepSeek Harness</option>
              <option value="unknown">Unknown</option>
            </select>
          </label>
          <label className="scope-search">
            <span className="sr-only">搜索</span>
            <SearchGlyph />
            <input
              type="search"
              value={filterForm.search}
              onChange={(event) => updateFilter("search", event.target.value)}
              placeholder="搜索标题、项目、会话 ID、模型或请求 ID"
            />
          </label>
          <button
            className="quiet-button filter-toggle"
            type="button"
            aria-expanded={advancedOpen}
            aria-controls="advanced-filters"
            onClick={() => setShowAdvanced(!advancedOpen)}
          >
            更多筛选
            {advancedCount ? (
              <span className="filter-count">{advancedCount}</span>
            ) : null}
          </button>
          <div className="scope-actions">
            {hasActiveFilters ? (
              <button
                className="ghost-button"
                type="button"
                onClick={() => {
                  setFilterForm(initialDashboardFilterForm());
                  setShowAdvanced(false);
                }}
              >
                清除筛选
              </button>
            ) : null}
            <div className="segmented" role="group" aria-label="导出">
              <button
                type="button"
                onClick={() => void handleExport("csv")}
                disabled={exportingFormat !== null}
              >
                {exportingFormat === "csv" ? "导出中…" : "导出 CSV"}
              </button>
              <button
                type="button"
                onClick={() => void handleExport("json")}
                disabled={exportingFormat !== null}
              >
                {exportingFormat === "json" ? "导出中…" : "导出 JSON"}
              </button>
            </div>
          </div>
        </div>
        <div
          className="scope-advanced"
          id="advanced-filters"
          data-open={advancedOpen || undefined}
          inert={!advancedOpen}
        >
          <div className="scope-advanced-inner">
            <label>
              <span>精度</span>
              <select
                value={filterForm.precision}
                onChange={(event) =>
                  updateFilter(
                    "precision",
                    event.target.value as DashboardFilterForm["precision"],
                  )
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
                  updateFilter("provider_id", event.target.value)
                }
                placeholder="精确匹配"
              />
            </label>
            <label>
              <span>Account ID</span>
              <input
                value={filterForm.account_id}
                onChange={(event) =>
                  updateFilter("account_id", event.target.value)
                }
                placeholder="精确匹配"
              />
            </label>
            <label>
              <span>Model</span>
              <input
                value={filterForm.model}
                onChange={(event) => updateFilter("model", event.target.value)}
                placeholder="包含匹配"
              />
            </label>
            <label>
              <span>项目路径</span>
              <input
                value={filterForm.project_path}
                onChange={(event) =>
                  updateFilter("project_path", event.target.value)
                }
                placeholder="包含匹配"
              />
            </label>
          </div>
        </div>
      </section>

      <section className="metrics" aria-label="今日统计">
        <div className="metrics-primary">
          <MetricCard
            index={0}
            label="输入 Token"
            numeric={totals.input_tokens_total}
            format={formatTokens}
            tone="accent"
            loading={loading}
          />
          <MetricCard
            index={1}
            label="输出 Token"
            numeric={totals.output_tokens_total}
            format={formatTokens}
            tone="violet"
            loading={loading}
          />
          <MetricCard
            index={2}
            label="缓存命中率"
            numeric={totals.cache_hit_rate_percent}
            format={formatPercent}
            tone="accent"
            loading={loading}
          >
            {loading ? null : (
              <Meter
                percent={totals.cache_hit_rate_percent}
                label="缓存命中率"
                tone="accent"
              />
            )}
          </MetricCard>
          <MetricCard
            index={3}
            label="费用（USD）"
            value={formatCost(totals)}
            tone="amber"
            loading={loading}
            hint={
              totals.provider_reported_cost != null
                ? "供应商实报"
                : totals.estimated_cost != null
                  ? "按价格表估算"
                  : undefined
            }
          />
        </div>
        <div className="metrics-secondary">
          <MetricCard
            index={4}
            label="缓存读取"
            numeric={totals.cache_read_tokens}
            format={formatTokens}
            loading={loading}
          />
          <MetricCard
            index={5}
            label="缓存写入"
            numeric={totals.cache_write_tokens}
            format={formatTokens}
            loading={loading}
          />
          <MetricCard
            index={6}
            label="推理 Token"
            numeric={totals.reasoning_tokens}
            format={formatTokens}
            loading={loading}
          />
          <MetricCard
            index={7}
            label="事件数"
            numeric={loading ? null : totals.event_count}
            format={formatTokens}
            loading={loading}
          />
        </div>
      </section>

      <section
        className="panel breakdown-panel"
        aria-labelledby="breakdown-heading"
      >
        <div className="panel-heading">
          <div>
            <p className="section-kicker">Model & provider</p>
            <h2 id="breakdown-heading">按模型 / 供应商</h2>
          </div>
          <p className="panel-note">
            费用以 USD 展示，优先使用供应商实报；带 “~”
            的数值按模型价格表估算。部分会话日志未拆分缓存写入时，仅按已记录的输入、缓存命中和输出估算。
          </p>
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
                  <th scope="col">费用（USD）</th>
                </tr>
              </thead>
              <tbody>
                {breakdown.map((row, index) => {
                  const tokens = rowTokens(row);
                  return (
                    <tr
                      key={`${row.model ?? "-"}|${row.provider_id ?? "-"}|${row.app}`}
                      style={{ "--i": Math.min(index, 12) } as CSSProperties}
                    >
                      <td className="model-cell">
                        <span>{row.model || "模型 Unavailable"}</span>
                        {tokens != null && maxRowTokens > 0 ? (
                          <span
                            className="share-bar"
                            aria-hidden="true"
                            style={
                              {
                                "--value": `${(tokens / maxRowTokens) * 100}%`,
                              } as CSSProperties
                            }
                          />
                        ) : null}
                      </td>
                      <td>
                        {row.provider_name ||
                          row.provider_id ||
                          "供应商 Unavailable"}
                      </td>
                      <td>{appLabel(row.app)}</td>
                      <td>{formatTokens(row.totals.input_tokens_total)}</td>
                      <td>{formatTokens(row.totals.output_tokens_total)}</td>
                      <td>
                        {formatPercent(row.totals.cache_hit_rate_percent)}
                      </td>
                      <td>{formatTokens(row.totals.event_count)}</td>
                      <td>{formatCost(row.totals)}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        ) : (
          <p className="breakdown-empty">
            {loading ? "正在读取用量…" : "当前筛选下没有用量记录。"}
          </p>
        )}
      </section>

      <section className="workspace-grid">
        <div className="panel sessions-panel">
          <div className="panel-heading">
            <div>
              <p className="section-kicker">Sessions</p>
              <h2>会话</h2>
            </div>
            <span className="count-label">
              {sessions.length} 条
              {sessionTotal != null && sessionTotal > sessions.length
                ? ` / 共 ${sessionTotal}`
                : ""}
            </span>
          </div>
          {sessions.length ? (
            <div className="session-list scroll-area" role="list">
              {sessions.map((item, index) => (
                <SessionRow
                  key={item.session.id}
                  index={index}
                  summary={item}
                  selected={item.session.id === selectedSessionId}
                  onSelect={() =>
                    setSelectedSessionId((current) =>
                      current === item.session.id ? null : item.session.id,
                    )
                  }
                />
              ))}
            </div>
          ) : (
            <EmptyState
              compact
              title={loading ? "正在读取会话…" : "当前筛选下没有会话"}
              description={
                loading
                  ? ""
                  : "调整时间范围或筛选条件；如果刚安装，点击“扫描全部来源”导入各来源的用量记录。"
              }
            />
          )}
          {sessionTotal != null && sessionTotal > sessions.length ? (
            <div className="panel-foot">
              <RouteLink to="/sessions">查看全部会话 →</RouteLink>
            </div>
          ) : null}
        </div>

        <div className="panel detail-panel">
          {visibleDetail ? (
            <SessionDetailView detail={visibleDetail} />
          ) : selectedSession ? (
            <div className="detail-loading" aria-label="正在读取会话详情">
              <span className="skeleton skeleton-title" />
              <span className="skeleton skeleton-line" />
              <span className="skeleton skeleton-block" />
            </div>
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
          {sources.length ? (
            <>
              {sources.length} 个数据源已登记 ·{" "}
              <RouteLink to="/sources">查看健康状态</RouteLink>
            </>
          ) : (
            "尚未登记数据源"
          )}
        </span>
        <span>代理模式未启用 · 数据留在本机</span>
      </footer>
    </PageFrame>
  );
}

function ScanGlyph() {
  return (
    <svg
      className="scan-glyph"
      viewBox="0 0 16 16"
      width="14"
      height="14"
      aria-hidden="true"
    >
      <path
        d="M13.5 8A5.5 5.5 0 1 1 11.9 4.1M13.5 2.5v3h-3"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function SearchGlyph() {
  return (
    <svg
      className="search-glyph"
      viewBox="0 0 16 16"
      width="14"
      height="14"
      aria-hidden="true"
    >
      <circle
        cx="7"
        cy="7"
        r="4.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
      />
      <path
        d="m10.5 10.5 3 3"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
    </svg>
  );
}
