// Small presentational pieces shared across views.

import type { CSSProperties, ReactNode } from "react";

import type {
  PrecisionLevel,
  SessionDetail,
  SessionSummary,
  UsageEvent,
  UsageTotals,
} from "../lib/api";
import {
  appLabel,
  formatDate,
  formatTokens,
  precisionLabel,
} from "../lib/format";
import { useCountUp } from "../lib/motion";

export type MetricTone = "accent" | "violet" | "amber" | "plain";

export function MetricCard({
  label,
  value,
  numeric,
  format,
  tone = "plain",
  loading = false,
  hint,
  index = 0,
  children,
}: {
  children?: ReactNode;
  label: string;
  /** Static text, used when the value is not a single number (e.g. cost). */
  value?: string;
  /** A number to count up to; rendered through `format`. */
  numeric?: number | null;
  format?: (value: number | null) => string;
  tone?: MetricTone;
  loading?: boolean;
  hint?: string;
  index?: number;
}) {
  return (
    <article
      className={`metric-card metric-${tone}`}
      data-glow
      style={{ "--i": index } as CSSProperties}
    >
      <p className="metric-label">{label}</p>
      {loading ? (
        <span className="skeleton skeleton-value" aria-label="加载中" />
      ) : format ? (
        <CountUpValue value={numeric ?? null} format={format} />
      ) : (
        <strong className="metric-value">{value}</strong>
      )}
      {hint ? <span className="metric-hint">{hint}</span> : null}
      {children}
    </article>
  );
}

function CountUpValue({
  value,
  format,
}: {
  value: number | null;
  format: (value: number | null) => string;
}) {
  const shown = useCountUp(value);
  // Integers stay integers mid-flight so the digits do not jitter.
  const display =
    shown == null || value == null || Number.isInteger(value)
      ? shown == null
        ? shown
        : Math.round(shown)
      : shown;
  return (
    <strong className="metric-value" data-unavailable={value == null}>
      {format(display)}
    </strong>
  );
}

/**
 * A horizontal gauge. `null` renders an empty hatched track — an unknown share
 * is never drawn as an empty (0%) bar.
 */
export function Meter({
  percent,
  label,
  tone = "auto",
}: {
  percent: number | null;
  label: string;
  tone?: "auto" | "accent" | "violet";
}) {
  const clamped = percent == null ? null : Math.max(0, Math.min(100, percent));
  const resolved =
    tone !== "auto"
      ? tone
      : clamped == null
        ? "accent"
        : clamped >= 90
          ? "danger"
          : clamped >= 70
            ? "warn"
            : "accent";
  return (
    <span
      className={`meter meter-${resolved}`}
      data-unavailable={clamped == null || undefined}
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={clamped ?? undefined}
    >
      <span
        className="meter-fill"
        style={{ "--value": `${clamped ?? 0}%` } as CSSProperties}
      />
    </span>
  );
}

export function SessionRow({
  summary,
  selected,
  onSelect,
  index = 0,
}: {
  summary: SessionSummary;
  selected: boolean;
  onSelect: () => void;
  index?: number;
}) {
  const title =
    summary.session.title ||
    summary.session.external_session_id ||
    "未命名会话";
  return (
    <button
      className={`session-row ${selected ? "selected" : ""}`}
      type="button"
      aria-pressed={selected}
      onClick={onSelect}
      style={{ "--i": Math.min(index, 12) } as CSSProperties}
    >
      <span className={`app-dot app-${summary.session.app}`} aria-hidden />
      <span className="session-row-main">
        <span className="session-title">{title}</span>
        <span className="session-meta">
          {appLabel(summary.session.app)} ·{" "}
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

export function SessionDetailView({ detail }: { detail: SessionDetail }) {
  const { session } = detail.summary;
  return (
    <div className="detail-content" key={session.id}>
      <div className="detail-heading">
        <div className="detail-heading-text">
          <p className="section-kicker">
            {appLabel(session.app)} · Session detail
          </p>
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
      <TokenComposition totals={detail.summary.totals} />
      <div className="timeline-heading">
        <span>请求时间线</span>
        <span className="count-label">{detail.usage_events.length} 次</span>
      </div>
      <div className="timeline" aria-label="请求时间线">
        {detail.usage_events.length ? (
          detail.usage_events.map((event, index) => (
            <EventRow event={event} key={event.id} index={index} />
          ))
        ) : (
          <EmptyState
            compact
            title="没有请求级事件"
            description="该会话只有元数据，usage 仍为 Unavailable。"
          />
        )}
      </div>
    </div>
  );
}

/**
 * Where the session's tokens went: fresh input, cache reads, and output. Drawn
 * only when all three parts are known — a partial bar would misstate shares.
 */
export function TokenComposition({ totals }: { totals: UsageTotals }) {
  const parts = [
    ["fresh", "新输入", totals.input_tokens_uncached],
    ["cache", "缓存读取", totals.cache_read_tokens],
    ["output", "输出", totals.output_tokens_total],
  ] as const;
  if (parts.some(([, , value]) => value == null)) return null;
  const sum = parts.reduce((acc, [, , value]) => acc + (value ?? 0), 0);
  if (sum <= 0) return null;
  return (
    <div className="composition">
      <div className="composition-bar" aria-hidden="true">
        {parts.map(([key, , value]) => (
          <span
            key={key}
            className={`composition-${key}`}
            style={{ flexGrow: value ?? 0 }}
          />
        ))}
      </div>
      <div className="composition-legend">
        {parts.map(([key, label, value]) => (
          <span key={key}>
            <i className={`composition-${key}`} aria-hidden="true" />
            {label} {(((value ?? 0) / sum) * 100).toFixed(1)}%
          </span>
        ))}
      </div>
    </div>
  );
}

export function EventRow({
  event,
  index = 0,
}: {
  event: UsageEvent;
  index?: number;
}) {
  return (
    <article
      className="event-row"
      style={{ "--i": Math.min(index, 14) } as CSSProperties}
    >
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

export function DetailStat({ label, value }: { label: string; value: string }) {
  return (
    <div data-glow>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

export function PrecisionBadge({ level }: { level: PrecisionLevel }) {
  return (
    <span className={`precision-badge precision-${level}`}>
      {precisionLabel(level)}
    </span>
  );
}

export function EmptyState({
  title,
  description,
  compact = false,
  action,
}: {
  title: string;
  description: string;
  compact?: boolean;
  action?: ReactNode;
}) {
  return (
    <div className={`empty-state ${compact ? "compact" : ""}`}>
      <span className="empty-radar" aria-hidden="true">
        <span />
      </span>
      <h3>{title}</h3>
      {description ? <p>{description}</p> : null}
      {action ? <div className="empty-action">{action}</div> : null}
    </div>
  );
}

export function SummaryItem({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  return (
    <div>
      <dt>{label}</dt>
      <dd title={value}>{value}</dd>
    </div>
  );
}

export function Notice({
  children,
  onDismiss,
}: {
  children: ReactNode;
  onDismiss?: () => void;
}) {
  return (
    <div className="notice notice-warning" role="alert">
      <span className="notice-icon" aria-hidden="true">
        !
      </span>
      <p className="notice-text">{children}</p>
      {onDismiss ? (
        <button
          className="notice-close"
          type="button"
          aria-label="关闭"
          onClick={onDismiss}
        >
          ×
        </button>
      ) : null}
    </div>
  );
}
