// The dashboard's filter form and its translation into the query contract.
//
// The date inputs speak the user's *local* calendar day while the Core speaks
// UTC instants; converting in one place is what keeps the metric cards, the
// session list and an export all describing the same window.

import type { PrecisionLevel, UsageFilters, UsageTotals } from "./api";

export const emptyTotals: UsageTotals = {
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

export type DashboardFilterForm = {
  period_start: string;
  period_end: string;
  app:
    "" | "codex" | "claude_code" | "open_code" | "deepseek_harness" | "unknown";
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

export function localDateInput(date = new Date()): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

export function localDayStartIso(dateString: string): string | null {
  const start = new Date(`${dateString}T00:00:00`);
  return Number.isNaN(start.valueOf()) ? null : start.toISOString();
}

// Turn an unknown thrown value into a message worth showing a user. The backend

export function initialDashboardFilterForm(): DashboardFilterForm {
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

// Quick date ranges for the scope bar. `null` days means "no bound" (all time).
export type DatePreset = { label: string; days: number | null };

export const datePresets: DatePreset[] = [
  { label: "今天", days: 1 },
  { label: "近 7 天", days: 7 },
  { label: "近 30 天", days: 30 },
  { label: "全部", days: null },
];

export function presetRange(days: number | null, now = new Date()) {
  if (days == null) return { period_start: "", period_end: "" };
  const start = new Date(now);
  start.setDate(start.getDate() - (days - 1));
  return {
    period_start: localDateInput(start),
    period_end: localDateInput(now),
  };
}

export function activePreset(
  form: Pick<DashboardFilterForm, "period_start" | "period_end">,
  now = new Date(),
): number | null | undefined {
  const match = datePresets.find((preset) => {
    const range = presetRange(preset.days, now);
    return (
      range.period_start === form.period_start &&
      range.period_end === form.period_end
    );
  });
  return match ? match.days : undefined;
}

export const advancedFilterKeys = [
  "precision",
  "provider_id",
  "account_id",
  "model",
  "project_path",
] as const;

export function dashboardFilters(form: DashboardFilterForm): UsageFilters {
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
