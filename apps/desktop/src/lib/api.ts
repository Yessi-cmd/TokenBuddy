import { invoke } from "@tauri-apps/api/core";

export type AppKind = "codex" | "claude_code" | "unknown";
export type LauncherKind =
  "direct" | "cc_switch" | "cockpit" | "observer_proxy" | "unknown";
export type IngestSource =
  | "session_log"
  | "otel"
  | "proxy"
  | "quota_api"
  | "imported_database"
  | "estimated";
export type PrecisionLevel =
  "verified" | "exact_session" | "correlated" | "estimated" | "unavailable";

export interface NormalizedUsage {
  input_tokens_total: number | null;
  input_tokens_uncached: number | null;
  cache_read_tokens: number | null;
  cache_write_tokens: number | null;
  output_tokens_total: number | null;
  reasoning_tokens: number | null;
  visible_output_tokens: number | null;
}

export interface UsageTotals extends NormalizedUsage {
  event_count: number;
  provider_reported_cost: number | null;
  estimated_cost: number | null;
  cache_hit_rate_percent: number | null;
}

export interface SessionRecord {
  id: string;
  external_session_id: string | null;
  parent_session_id: string | null;
  app: AppKind;
  launcher: LauncherKind | null;
  project_path: string | null;
  title: string | null;
  started_at: string | null;
  ended_at: string | null;
  source_id: string | null;
  created_at: string;
  updated_at: string;
}

export interface UsageEvent {
  id: string;
  occurred_at: string;
  app: AppKind;
  launcher: LauncherKind;
  ingest_source: IngestSource;
  source_id: string;
  provider_id: string | null;
  account_id: string | null;
  session_id: string | null;
  parent_session_id: string | null;
  request_id: string | null;
  response_id: string | null;
  model: string | null;
  query_source: string | null;
  usage: NormalizedUsage;
  provider_reported_cost: number | null;
  estimated_cost: number | null;
  currency: string | null;
  http_status: number | null;
  latency_ms: number | null;
  success: boolean | null;
  precision_token: PrecisionLevel;
  precision_session: PrecisionLevel;
  precision_provider: PrecisionLevel;
  precision_account: PrecisionLevel;
  raw_event_hash: string;
  raw_usage_json: Record<string, unknown> | null;
}

export interface SessionSummary {
  session: SessionRecord;
  totals: UsageTotals;
}

export interface SessionPage {
  sessions: SessionSummary[];
  total: number;
}

export interface SessionDetail {
  summary: SessionSummary;
  usage_events: UsageEvent[];
}

export interface DashboardSummary {
  period_start: string;
  period_end: string;
  totals: UsageTotals;
}

export interface SourceRecord {
  id: string;
  adapter_type: string;
  display_name: string;
  path_or_endpoint: string | null;
  enabled: boolean;
  detected_version: string | null;
  health_status: string | null;
  last_success_at: string | null;
  last_error: string | null;
  created_at: string;
  updated_at: string;
}

export interface RescanResult {
  inserted_events: number;
  duplicate_events: number;
  upserted_sessions: number;
  updated_cursors: number;
  skipped_records: number;
}

export interface DetectionResult {
  source_id: string;
  detected: boolean;
  path_or_endpoint: string | null;
  detected_version: string | null;
  message: string | null;
}

export function getDashboardSummary(): Promise<DashboardSummary> {
  return invoke<DashboardSummary>("get_dashboard_summary");
}

export function listSessions(
  search: string | null = null,
  limit = 50,
  offset = 0,
): Promise<SessionPage> {
  return invoke<SessionPage>("list_sessions", { search, limit, offset });
}

export function getSessionDetail(
  sessionId: string,
): Promise<SessionDetail | null> {
  return invoke<SessionDetail | null>("get_session_detail", {
    sessionId,
  });
}

export function listSources(): Promise<SourceRecord[]> {
  return invoke<SourceRecord[]>("list_sources");
}

export function detectCodexPath(
  codexHome: string | null,
): Promise<DetectionResult> {
  return invoke<DetectionResult>("detect_codex_path", { codexHome });
}

export function rescanCodex(codexHome: string | null): Promise<RescanResult> {
  return invoke<RescanResult>("rescan_codex", { codexHome });
}
