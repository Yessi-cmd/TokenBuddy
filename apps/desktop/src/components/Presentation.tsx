// Small presentational pieces shared across views.

import type {
  PrecisionLevel,
  SessionDetail,
  SessionSummary,
  UsageEvent,
} from "../lib/api";
import {
  appLabel,
  formatDate,
  formatTokens,
  precisionLabel,
} from "../lib/format";

export function MetricCard({
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

export function SessionRow({
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

export function EventRow({ event }: { event: UsageEvent }) {
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

export function DetailStat({ label, value }: { label: string; value: string }) {
  return (
    <div>
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
      <dd>{value}</dd>
    </div>
  );
}
