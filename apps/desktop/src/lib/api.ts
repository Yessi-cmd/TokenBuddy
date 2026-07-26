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
export type CollectionStatus = "starting" | "collecting" | "idle" | "error";

export interface QuotaSummary {
  window_type: string;
  used_percent: number | null;
  remaining_percent: number | null;
  reset_at: string | null;
  credits_remaining: number | null;
  precision: PrecisionLevel;
}

export interface QuickSummary {
  collection_status: CollectionStatus;
  active_app: AppKind | null;
  active_session_id: string | null;
  active_session_title: string | null;
  active_project_path: string | null;
  provider_name: string | null;
  model: string | null;
  session_input_tokens: number | null;
  session_cache_read_tokens: number | null;
  session_output_tokens: number | null;
  session_cache_hit_rate: number | null;
  today_total_tokens: number | null;
  quota_summary: QuotaSummary | null;
  latest_warning: string | null;
}

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

export interface UsageEventPage {
  events: UsageEvent[];
  total: number;
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

export interface ModelUsage {
  model: string | null;
  provider_id: string | null;
  provider_name: string | null;
  app: AppKind;
  totals: UsageTotals;
}

export interface UsageFilters {
  period_start?: string | null;
  period_end?: string | null;
  app?: AppKind | null;
  provider_id?: string | null;
  account_id?: string | null;
  model?: string | null;
  project_path?: string | null;
  precision?: PrecisionLevel | null;
  search?: string | null;
}

export interface ExportResult {
  filename: string;
  mime_type: string;
  content: string;
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
  warning: string | null;
}

export interface DetectionResult {
  source_id: string;
  detected: boolean;
  path_or_endpoint: string | null;
  detected_version: string | null;
  message: string | null;
}

export interface LocalWebApiStatus {
  running: boolean;
  url: string | null;
  loopback_urls?: string[];
}

export interface ProviderSummary {
  id: string;
  provider_family: string;
  display_name: string;
  upstream_url: string | null;
  launcher: LauncherKind | null;
  source_id: string | null;
  account_count: number;
  request_count: number;
  successful_request_count: number | null;
  success_rate_percent: number | null;
  average_latency_ms: number | null;
  totals: UsageTotals;
}

export interface QuotaSnapshot {
  id: string;
  account_id: string;
  account_name: string | null;
  provider_name: string | null;
  captured_at: string;
  window_type: string;
  used_percent: number | null;
  remaining_percent: number | null;
  reset_at: string | null;
  credits_remaining: number | null;
  precision: PrecisionLevel;
  raw_json: Record<string, unknown> | null;
}

export interface AccountRecord {
  id: string;
  provider_id: string;
  display_name: string | null;
  account_fingerprint: string;
  auth_mode: string;
  plan: string | null;
}

export interface AccountSummary {
  account: AccountRecord;
  provider_name: string | null;
  latest_quota: QuotaSummary | null;
}

export interface AppSettings {
  codex_home: string | null;
  claude_home: string | null;
  cc_switch_db_path: string | null;
  cockpit_path: string | null;
  otel_port: number | null;
  auto_start: boolean;
  proxy_enabled: boolean;
  save_request_metadata: boolean;
  data_retention_days: number | null;
}

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function request<T>(
  command: string,
  path: string,
  args?: Record<string, unknown>,
  init?: RequestInit,
): Promise<T> {
  if (inTauri()) {
    return invoke<T>(command, args);
  }
  const response = await fetch(path, init);
  const payload = (await response.json()) as T | { error?: string };
  if (!response.ok) {
    const error = payload as { error?: string };
    throw new Error(error.error || `请求失败（${response.status}）`);
  }
  return payload as T;
}

function queryString(
  values: Record<string, string | number | null | undefined>,
): string {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(values)) {
    if (value != null && value !== "") query.set(key, String(value));
  }
  const encoded = query.toString();
  return encoded ? `?${encoded}` : "";
}

export function getQuickSummary(): Promise<QuickSummary> {
  return request<QuickSummary>("get_quick_summary", "/api/quick-summary");
}

export function getDashboardSummary(
  filters: UsageFilters = {},
): Promise<DashboardSummary> {
  return request<DashboardSummary>(
    "get_dashboard_summary",
    `/api/dashboard-summary${queryString({ ...filters })}`,
    { filters },
  );
}

export function getModelBreakdown(
  filters: UsageFilters = {},
): Promise<ModelUsage[]> {
  return request<ModelUsage[]>(
    "get_model_breakdown",
    `/api/model-breakdown${queryString({ ...filters })}`,
    { filters },
  );
}

export function exportUsage(
  format: "csv" | "json",
  filters: UsageFilters = {},
): Promise<ExportResult> {
  return request<ExportResult>(
    "export_usage",
    "/api/export",
    { format, filters },
    inTauri()
      ? undefined
      : {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ format, filters }),
        },
  );
}

export function listSessions(
  filters: UsageFilters = {},
  limit = 50,
  offset = 0,
): Promise<SessionPage> {
  return request<SessionPage>(
    "list_sessions",
    `/api/sessions${queryString({ ...filters, limit, offset })}`,
    { filters, limit, offset },
  );
}

// Whether the app is running inside the desktop shell (vs. a plain browser tab).
export function isDesktopRuntime(): boolean {
  return inTauri();
}

// Desktop-only: write the export to disk (WKWebView cannot trigger a blob
// download reliably) and return the saved path.
export function saveExport(
  format: "csv" | "json",
  filters: UsageFilters = {},
): Promise<string> {
  return invoke<string>("save_export", { format, filters });
}

// Desktop-only: bring the full dashboard window forward.
export function showMainWindow(): Promise<void> {
  return invoke<void>("show_main_window");
}

export function getSessionDetail(
  sessionId: string,
): Promise<SessionDetail | null> {
  return request<SessionDetail | null>(
    "get_session_detail",
    `/api/sessions/${encodeURIComponent(sessionId)}`,
    { sessionId },
  );
}

export function listUsageEvents(
  sessionId: string | null = null,
  limit = 100,
  offset = 0,
): Promise<UsageEventPage> {
  return request<UsageEventPage>(
    "list_usage_events",
    `/api/usage-events${queryString({
      session_id: sessionId,
      limit,
      offset,
    })}`,
    { sessionId, limit, offset },
  );
}

export function listSources(): Promise<SourceRecord[]> {
  return request<SourceRecord[]>("list_sources", "/api/sources");
}

export function listProviders(): Promise<ProviderSummary[]> {
  return request<ProviderSummary[]>("list_providers", "/api/providers");
}

export function listAccounts(): Promise<AccountSummary[]> {
  return request<AccountSummary[]>("list_accounts", "/api/accounts");
}

// Desktop-only: native directory picker. Resolves to null when the user
// cancels. The browser panel has no system dialog, so it keeps its text field.
export function pickDirectory(
  title: string,
  startAt: string | null,
): Promise<string | null> {
  return invoke<string | null>("pick_directory", { title, startAt });
}

// Desktop-only: native file picker, used for the third-party SQLite paths.
export function pickFile(
  title: string,
  startAt: string | null,
): Promise<string | null> {
  return invoke<string | null>("pick_file", { title, startAt });
}

export function listQuotaSnapshots(
  accountId: string | null = null,
  limit = 100,
): Promise<QuotaSnapshot[]> {
  return request<QuotaSnapshot[]>(
    "list_quota_snapshots",
    `/api/quotas${queryString({ account_id: accountId, limit })}`,
    { accountId, limit },
  );
}

export function getAppSettings(): Promise<AppSettings> {
  return request<AppSettings>("get_app_settings", "/api/settings");
}

export function updateAppSettings(settings: AppSettings): Promise<AppSettings> {
  return request<AppSettings>(
    "update_app_settings",
    "/api/settings",
    { settings },
    inTauri()
      ? undefined
      : {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(settings),
        },
  );
}

export function detectCodexPath(
  codexHome: string | null,
): Promise<DetectionResult> {
  return request<DetectionResult>(
    "detect_codex_path",
    `/api/detect-codex${queryString({ codex_home: codexHome })}`,
    { codexHome },
  );
}

export function rescanCodex(codexHome: string | null): Promise<RescanResult> {
  return request<RescanResult>(
    "rescan_codex",
    "/api/rescan-codex",
    {
      codexHome,
    },
    inTauri()
      ? undefined
      : {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ codex_home: codexHome }),
        },
  );
}

export function detectClaudePath(
  claudeHome: string | null,
): Promise<DetectionResult> {
  return request<DetectionResult>(
    "detect_claude_path",
    `/api/detect-claude${queryString({ claude_home: claudeHome })}`,
    { claudeHome },
  );
}

export function rescanClaude(claudeHome: string | null): Promise<RescanResult> {
  return request<RescanResult>(
    "rescan_claude",
    "/api/rescan-claude",
    {
      claudeHome,
    },
    inTauri()
      ? undefined
      : {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ claude_home: claudeHome }),
        },
  );
}

export function detectCcSwitchPath(
  ccSwitchDb: string | null,
): Promise<DetectionResult> {
  return request<DetectionResult>(
    "detect_cc_switch_path",
    `/api/detect-cc-switch${queryString({ cc_switch_db: ccSwitchDb })}`,
    { ccSwitchDb },
  );
}

export function rescanCcSwitch(
  ccSwitchDb: string | null,
): Promise<RescanResult> {
  return request<RescanResult>(
    "rescan_cc_switch",
    "/api/rescan-cc-switch",
    {
      ccSwitchDb,
    },
    inTauri()
      ? undefined
      : {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ cc_switch_db: ccSwitchDb }),
        },
  );
}

export function detectCockpitPath(
  cockpitDb: string | null,
): Promise<DetectionResult> {
  return request<DetectionResult>(
    "detect_cockpit_path",
    `/api/detect-cockpit${queryString({ cockpit_db: cockpitDb })}`,
    { cockpitDb },
  );
}

export function rescanCockpit(cockpitDb: string | null): Promise<RescanResult> {
  return request<RescanResult>(
    "rescan_cockpit",
    "/api/rescan-cockpit",
    {
      cockpitDb,
    },
    inTauri()
      ? undefined
      : {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ cockpit_db: cockpitDb }),
        },
  );
}

export function startLocalWebApi(): Promise<LocalWebApiStatus> {
  if (!inTauri()) {
    return Promise.resolve({ running: true, url: window.location.origin });
  }
  return request<LocalWebApiStatus>(
    "start_local_web_api",
    "/api/local-web/status",
  );
}

export function openLocalWebApi(): Promise<LocalWebApiStatus> {
  if (!inTauri()) {
    return Promise.resolve({ running: true, url: window.location.origin });
  }
  return request<LocalWebApiStatus>(
    "open_local_web_api",
    "/api/local-web/status",
  );
}

export function getLocalWebApiStatus(): Promise<LocalWebApiStatus> {
  if (!inTauri()) {
    return Promise.resolve({ running: true, url: window.location.origin });
  }
  return request<LocalWebApiStatus>(
    "get_local_web_api_status",
    "/api/local-web/status",
  );
}
