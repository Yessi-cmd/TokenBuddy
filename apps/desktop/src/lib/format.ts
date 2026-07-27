// Presentation helpers shared by every view.
//
// The rule these encode: a value the Core could not determine renders as
// "Unavailable", never as 0 or an empty cell. Formatting is where that promise
// is most easily broken, so it lives in one place.

import type { PrecisionLevel, QuickSummary, UsageTotals } from "./api";

// Turn an unknown thrown value into a message worth showing a user. The backend
// returns command errors as strings; native/network failures arrive as Error.
export function describeError(cause: unknown): string {
  if (cause instanceof Error) return cause.message;
  if (typeof cause === "string" && cause.trim()) return cause;
  return "未知错误";
}

export function formatTokens(value: number | null | undefined): string {
  return value == null
    ? "Unavailable"
    : new Intl.NumberFormat("en-US").format(value);
}

export function formatPercent(value: number | null): string {
  return value == null ? "Unavailable" : `${value.toFixed(1)}%`;
}

export function formatCost(totals: UsageTotals): string {
  if (totals.provider_reported_cost != null)
    return totals.provider_reported_cost.toFixed(4);
  if (totals.estimated_cost != null)
    return `~${totals.estimated_cost.toFixed(4)}`;
  return "N/A";
}

export function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.valueOf())
    ? "时间 Unavailable"
    : date.toLocaleString();
}

export function precisionLabel(level: PrecisionLevel): string {
  const labels: Record<PrecisionLevel, string> = {
    verified: "Verified",
    exact_session: "Exact session",
    correlated: "Correlated",
    estimated: "Estimated",
    unavailable: "Unavailable",
  };
  return labels[level];
}

export function authModeLabel(authMode: string): string {
  const labels: Record<string, string> = {
    chatgpt: "ChatGPT 官方登录",
    api_key: "API Key",
    session_log: "会话日志推断",
  };
  return labels[authMode] ?? authMode;
}

export function collectionStatusLabel(
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
