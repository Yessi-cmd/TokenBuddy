import { useEffect, useRef, useState } from "react";

import {
  fitQuickWindowToContent,
  getQuickSummary,
  isDesktopRuntime,
  openLocalWebApi,
  showMainWindow,
  type QuickSummary,
} from "../../lib/api";
import {
  MenuGroupTitle,
  MenuRow,
  MenuSeparator,
  MenuValueRow,
} from "../../components/Menu";
import {
  collectionStatusLabel,
  describeError,
  formatPercent,
  formatTokens,
} from "../../lib/format";

export function QuickSummaryView() {
  const [summary, setSummary] = useState<QuickSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const shellRef = useRef<HTMLDivElement | null>(null);

  // Keep the popover window exactly as tall as what is rendered. The window is
  // created at a fixed height because Rust cannot predict the content — a
  // missing quota window or an added warning changes it — and anything the
  // content does not fill shows up as dead space under the last row.
  useEffect(() => {
    const shell = shellRef.current;
    if (!shell) return;

    const fit = () => {
      void fitQuickWindowToContent(shell.getBoundingClientRect().height).catch(
        (cause: unknown) => console.error("调整快速面板高度失败", cause),
      );
    };
    fit();

    const observer = new ResizeObserver(fit);
    observer.observe(shell);
    return () => observer.disconnect();
  }, [summary, error]);

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
    <div className="menu-shell" ref={shellRef}>
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
