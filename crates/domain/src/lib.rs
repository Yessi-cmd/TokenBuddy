//! Shared, source-agnostic domain types for TokenBuddy.
//!
//! This crate deliberately has no dependency on Tauri or SQLite. Adapters
//! normalize source-specific records into these types and the storage crate
//! persists them for the desktop application.
//!
//! Every public item here is documented and the lint below keeps it that way:
//! this crate is the vocabulary every other crate and the UI speak, so an
//! undocumented field is a question someone will have to answer by reading
//! callers.
#![warn(missing_docs)]

use std::{fmt, path::PathBuf, sync::Arc};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Which AI coding tool produced a session or usage event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AppKind {
    /// Codex App or Codex CLI.
    Codex,
    /// Claude Code CLI.
    ClaudeCode,
    /// A source whose app could not be determined. Never a stand-in for a
    /// specific app that simply was not checked.
    Unknown,
}

impl AppKind {
    /// The stable identifier written to SQLite and sent to the UI.
    ///
    /// These strings are persisted, so they may not be renamed without a
    /// migration; the enum variants can be.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude_code",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for AppKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str((*self).as_str())
    }
}

/// How a request reached its provider: directly, or through a tool that routes
/// and rewrites the upstream.
///
/// This matters for attribution — a launcher knows the real provider and account
/// behind a request, which the session log never records.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LauncherKind {
    /// The app talked to the provider itself.
    Direct,
    /// Routed by CC-Switch.
    ///
    /// Renamed explicitly: `rename_all = "snake_case"` turns `CCSwitch` into
    /// `c_c_switch`, which would not match the name written to SQLite by
    /// `as_str` nor the value the frontend's `LauncherKind` union declares.
    #[serde(rename = "cc_switch")]
    CCSwitch,
    /// Routed by Cockpit Tools.
    Cockpit,
    /// Routed by TokenBuddy's own optional local proxy (spec §12, not built).
    ObserverProxy,
    /// Routing could not be determined.
    Unknown,
}

impl LauncherKind {
    /// The stable identifier written to SQLite and sent to the UI.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::CCSwitch => "cc_switch",
            Self::Cockpit => "cockpit",
            Self::ObserverProxy => "observer_proxy",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for LauncherKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str((*self).as_str())
    }
}

/// Where a usage record was read from.
///
/// Ranked by trustworthiness in spec §6.1: provider-reported usage beats OTel,
/// which beats session logs, which beat proxy logs, which beat estimates. When
/// two sources describe the same request, the higher-ranked one wins and the
/// other must not be counted again.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum IngestSource {
    /// A transcript the app wrote itself (`~/.codex/sessions`, `~/.claude/projects`).
    SessionLog,
    /// An OpenTelemetry event emitted by the app (spec §8.3, §9.3; not built).
    Otel,
    /// A proxy's request log. Ranked below session logs, so proxy rows are used
    /// for attribution rather than token counts.
    Proxy,
    /// An official quota endpoint (spec §8.4).
    QuotaApi,
    /// A third-party tool's own database, imported read-only.
    ImportedDatabase,
    /// Derived by a tokenizer or price table rather than reported by anyone.
    Estimated,
}

impl IngestSource {
    /// The stable identifier written to SQLite and sent to the UI.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionLog => "session_log",
            Self::Otel => "otel",
            Self::Proxy => "proxy",
            Self::QuotaApi => "quota_api",
            Self::ImportedDatabase => "imported_database",
            Self::Estimated => "estimated",
        }
    }

    /// Relative trust order when two sources describe the same request.
    ///
    /// This is intentionally separate from token precision: a provider may
    /// report an exact value through a future source, while a session log can
    /// also be exact for its own record. Storage compares both dimensions before
    /// replacing a correlated observation.
    pub const fn precedence(self) -> u8 {
        match self {
            Self::Otel => 4,
            Self::SessionLog => 3,
            Self::ImportedDatabase => 2,
            Self::Proxy => 1,
            Self::QuotaApi | Self::Estimated => 0,
        }
    }
}

impl fmt::Display for IngestSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str((*self).as_str())
    }
}

/// How much a stored value can be trusted (spec §14).
///
/// Precision is a product feature, not an implementation detail: every number
/// the UI shows carries the level it was established at, and the *weakest* link
/// decides. A token count read verbatim from a session log but attributed to an
/// account by a time-window match is `Correlated`, not `ExactSession`.
///
/// Displaying `Unavailable` as `0`, or `Estimated` as `Verified`, is forbidden.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PrecisionLevel {
    /// Straight from the upstream API, official OTel, or an official quota
    /// endpoint.
    Verified,
    /// Exact numbers from a stable session log, with unambiguous session
    /// ownership.
    ExactSession,
    /// The numbers are exact, but the provider, account, or session was matched
    /// by time window or model rather than stated by the source.
    Correlated,
    /// Derived by a tokenizer, a price table, or another rule.
    Estimated,
    /// The source does not carry this field. Distinct from zero.
    Unavailable,
}

impl PrecisionLevel {
    /// The stable identifier written to SQLite and sent to the UI.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::ExactSession => "exact_session",
            Self::Correlated => "correlated",
            Self::Estimated => "estimated",
            Self::Unavailable => "unavailable",
        }
    }

    /// Relative trust order used when reconciling two observations of one
    /// request. `Unavailable` is lowest because it carries no measured value.
    pub const fn precedence(self) -> u8 {
        match self {
            Self::Verified => 5,
            Self::ExactSession => 4,
            Self::Correlated => 3,
            Self::Estimated => 2,
            Self::Unavailable => 0,
        }
    }
}

/// Build the canonical identity used to correlate observations from different
/// adapters without storing any request body.
///
/// Request ids are preferred because they are normally stable across a streamed
/// response and across Session/OTel exports. A response id is the fallback for
/// providers that do not expose a request id. The app namespace prevents an
/// opaque id reused by two tools from merging unrelated calls.
pub fn correlation_key(
    app: AppKind,
    request_id: Option<&str>,
    response_id: Option<&str>,
) -> Option<String> {
    request_id
        .filter(|value| !value.is_empty())
        .map(|value| format!("{}:request:{value}", app.as_str()))
        .or_else(|| {
            response_id
                .filter(|value| !value.is_empty())
                .map(|value| format!("{}:response:{value}", app.as_str()))
        })
}

impl fmt::Display for PrecisionLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str((*self).as_str())
    }
}

/// What the background collector is doing, as shown in the tray.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CollectionStatus {
    /// The Core is starting and has not completed its first pass.
    Starting,
    /// At least one source is healthy and being imported.
    Collecting,
    /// Running, but no source is configured or detected.
    Idle,
    /// The most recent pass failed for at least one source; the failure is
    /// carried alongside in a warning.
    Error,
}

impl CollectionStatus {
    /// The stable identifier written to SQLite and sent to the UI.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Collecting => "collecting",
            Self::Idle => "idle",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for CollectionStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str((*self).as_str())
    }
}

/// Capabilities advertised by a source adapter.
///
/// The descriptor is deliberately separate from the parser implementation:
/// the Core and future settings/diagnostics surfaces can answer "what does this
/// source provide?" without knowing that source's schema. This follows the
/// descriptor-driven Provider architecture used by mature tray applications,
/// while keeping the actual data path behind the adapter boundary.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdapterCapabilities {
    /// Whether the adapter emits measured token usage events.
    pub usage_events: bool,
    /// Whether it contributes provider or account attribution context.
    pub provider_context: bool,
    /// Whether it emits official quota snapshots.
    pub quota_snapshots: bool,
    /// Whether the source can be watched for low-latency updates.
    pub file_watch: bool,
}

/// Static metadata for one independent source adapter.
///
/// A descriptor is a registry entry, not a claim that the source is currently
/// detected or healthy. Runtime health remains in [`SourceRecord`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdapterDescriptor {
    /// Stable adapter/source id.
    pub id: &'static str,
    /// Schema/implementation family stored on the source row.
    pub adapter_type: &'static str,
    /// Human-readable name.
    pub display_name: &'static str,
    /// What this source can contribute.
    pub capabilities: AdapterCapabilities,
    /// Whether the adapter is read-only with respect to its external source.
    pub read_only: bool,
}

impl AdapterDescriptor {
    /// A conservative descriptor for a third-party implementation that has not
    /// yet declared its detailed capabilities.
    pub const fn minimal(id: &'static str, display_name: &'static str) -> Self {
        Self {
            id,
            adapter_type: "unknown",
            display_name,
            capabilities: AdapterCapabilities {
                usage_events: false,
                provider_context: false,
                quota_snapshots: false,
                file_watch: false,
            },
            read_only: true,
        }
    }
}

/// One official quota window, as reported by the provider.
///
/// Quota lives apart from token usage on purpose (spec §8.4): a percentage is
/// never converted back into a token count, and a token count never implies a
/// percentage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuotaSummary {
    /// The window this covers, labelled with its length when the source states
    /// one (`primary_5h`, `secondary_7d`).
    pub window_type: String,
    /// Percentage of the window consumed, or `None` if not reported.
    pub used_percent: Option<f64>,
    /// Percentage still available. Derived as the complement of `used_percent`
    /// when the source reports only the used side — arithmetic on a reported
    /// percentage, never an inference about tokens.
    pub remaining_percent: Option<f64>,
    /// When the window resets, if the source says.
    pub reset_at: Option<DateTime<Utc>>,
    /// Remaining prepaid credits, for providers that bill that way.
    pub credits_remaining: Option<f64>,
    /// Trust level of this window, decided by its weakest link — usually the
    /// account attribution rather than the numbers.
    pub precision: PrecisionLevel,
}

/// The pre-aggregated snapshot the tray popover reads.
///
/// The Core keeps this up to date so a lightweight surface never scans a log,
/// loads history, or runs an aggregation to draw itself (spec §4.1). Every field
/// is optional because "not known yet" is a normal state at startup and must not
/// be rendered as zero.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuickSummary {
    /// What the collector is doing.
    pub collection_status: CollectionStatus,
    /// App of the most recent usage event.
    pub active_app: Option<AppKind>,
    /// Domain id of the most recent session.
    pub active_session_id: Option<String>,
    /// Human-readable title of that session, when one is known.
    pub active_session_title: Option<String>,
    /// Project directory that session belongs to.
    pub active_project_path: Option<String>,
    /// Display name of the provider that served it, resolved through the
    /// providers table so a relay shows its real name.
    pub provider_name: Option<String>,
    /// Model that answered.
    pub model: Option<String>,
    /// Input tokens of the active session, `None` if any contributing event
    /// lacks the field.
    pub session_input_tokens: Option<u64>,
    /// Cache-read tokens of the active session.
    pub session_cache_read_tokens: Option<u64>,
    /// Output tokens of the active session.
    pub session_output_tokens: Option<u64>,
    /// Cache hit rate of the active session, as a percentage.
    pub session_cache_hit_rate: Option<f64>,
    /// Total tokens for the user's local calendar day — the same "today" the
    /// dashboard uses, so the two never disagree.
    pub today_total_tokens: Option<u64>,
    /// Newest official quota window for the active account, if any.
    pub quota_summary: Option<QuotaSummary>,
    /// The most recent collection warning, surfaced instead of being buried in
    /// a log the user will not read.
    pub latest_warning: Option<String>,
}

impl QuickSummary {
    /// The summary shown before the first import pass completes: status
    /// `Starting` and every value explicitly unknown.
    pub fn starting() -> Self {
        Self {
            collection_status: CollectionStatus::Starting,
            active_app: None,
            active_session_id: None,
            active_session_title: None,
            active_project_path: None,
            provider_name: None,
            model: None,
            session_input_tokens: None,
            session_cache_read_tokens: None,
            session_output_tokens: None,
            session_cache_hit_rate: None,
            today_total_tokens: None,
            quota_summary: None,
            latest_warning: None,
        }
    }
}

/// Token counts from any provider, mapped onto one vocabulary (spec §13).
///
/// OpenAI and Anthropic disagree about what "input tokens" includes — one counts
/// cached reads inside the total, the other reports them separately. Adapters
/// resolve that here so every consumer can add fields without knowing which
/// provider produced them. A field the source did not report stays `None`: it is
/// never zero, and a total is never reconstructed by adding up parts that might
/// themselves be missing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NormalizedUsage {
    /// All input tokens, including cached reads.
    pub input_tokens_total: Option<u64>,
    /// Input tokens that were not served from cache.
    pub input_tokens_uncached: Option<u64>,
    /// Input tokens served from cache, billed at the cheaper read rate.
    pub cache_read_tokens: Option<u64>,
    /// Tokens written into the cache, billed at the write rate.
    pub cache_write_tokens: Option<u64>,
    /// All output tokens, including reasoning.
    pub output_tokens_total: Option<u64>,
    /// Output tokens spent on reasoning the user never sees.
    pub reasoning_tokens: Option<u64>,
    /// Output tokens actually shown to the user.
    pub visible_output_tokens: Option<u64>,
}

impl NormalizedUsage {
    /// Whether the source reported no token field at all.
    ///
    /// Distinct from "reported all zeros", which is a real measurement.
    pub fn is_empty(&self) -> bool {
        self.input_tokens_total.is_none()
            && self.input_tokens_uncached.is_none()
            && self.cache_read_tokens.is_none()
            && self.cache_write_tokens.is_none()
            && self.output_tokens_total.is_none()
            && self.reasoning_tokens.is_none()
            && self.visible_output_tokens.is_none()
    }

    /// Share of input tokens served from cache, as a percentage.
    ///
    /// `None` unless both counts are known and consistent: a zero total has no
    /// rate, and a cache read larger than the total means the two numbers came
    /// from different accounting schemes and must not be divided.
    pub fn cache_hit_rate_percent(&self) -> Option<f64> {
        let input_total = self.input_tokens_total?;
        let cache_read = self.cache_read_tokens?;
        if input_total == 0 || cache_read > input_total {
            return None;
        }

        Some((cache_read as f64 / input_total as f64) * 100.0)
    }

    /// The usage added since `previous`, for sources that log a running total.
    ///
    /// `None` when the difference is not a real increment — either the counter
    /// moved backwards (the file rotated, or a new session reset it) or nothing
    /// changed. Callers treat `None` as "record nothing", which is what keeps a
    /// cumulative snapshot from being counted twice.
    ///
    /// "Nothing changed" covers a repeated snapshot, whose difference is zero
    /// rather than absent: recording that would add an event worth no tokens
    /// and inflate the request count.
    pub fn delta_from(&self, previous: &Self) -> Option<Self> {
        let current = self.checked_delta(previous)?;
        if current.is_empty() || current.is_zero() {
            None
        } else {
            Some(current)
        }
    }

    /// Whether every field the source reported is zero.
    ///
    /// Distinct from [`is_empty`](Self::is_empty), which means the source
    /// reported no field at all.
    pub fn is_zero(&self) -> bool {
        [
            self.input_tokens_total,
            self.input_tokens_uncached,
            self.cache_read_tokens,
            self.cache_write_tokens,
            self.output_tokens_total,
            self.reasoning_tokens,
            self.visible_output_tokens,
        ]
        .into_iter()
        .all(|value| value.unwrap_or(0) == 0)
    }

    /// Field-by-field subtraction that refuses to underflow.
    ///
    /// Returns `None` if any field of `previous` exceeds the matching field
    /// here, so a rolled-back counter can never produce a negative delta that
    /// would silently subtract tokens from the totals.
    pub fn checked_delta(&self, previous: &Self) -> Option<Self> {
        Some(Self {
            input_tokens_total: checked_difference(
                self.input_tokens_total,
                previous.input_tokens_total,
            )?,
            input_tokens_uncached: checked_difference(
                self.input_tokens_uncached,
                previous.input_tokens_uncached,
            )?,
            cache_read_tokens: checked_difference(
                self.cache_read_tokens,
                previous.cache_read_tokens,
            )?,
            cache_write_tokens: checked_difference(
                self.cache_write_tokens,
                previous.cache_write_tokens,
            )?,
            output_tokens_total: checked_difference(
                self.output_tokens_total,
                previous.output_tokens_total,
            )?,
            reasoning_tokens: checked_difference(self.reasoning_tokens, previous.reasoning_tokens)?,
            visible_output_tokens: checked_difference(
                self.visible_output_tokens,
                previous.visible_output_tokens,
            )?,
        })
    }
}

fn checked_difference(current: Option<u64>, previous: Option<u64>) -> Option<Option<u64>> {
    match (current, previous) {
        (Some(current), Some(previous)) => Some(Some(current.checked_sub(previous)?)),
        (Some(current), None) => Some(Some(current)),
        (None, _) => Some(None),
    }
}

/// One measured request: the unit every statistic is built from.
///
/// Precision is tracked per dimension rather than once for the row, because the
/// token count and the account attribution routinely come from different places
/// with different confidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsageEvent {
    /// Domain id, equal to `raw_event_hash` so the same source record always
    /// lands on the same row.
    pub id: String,
    /// When the request happened, per the source.
    pub occurred_at: DateTime<Utc>,
    /// Which app made it.
    pub app: AppKind,
    /// Whether a launcher sat in between.
    pub launcher: LauncherKind,
    /// What kind of source this was read from.
    pub ingest_source: IngestSource,
    /// Id of the configured source (`codex-session`, `cc-switch`, …).
    pub source_id: String,
    /// Provider that served it, once known.
    pub provider_id: Option<String>,
    /// Account it was billed to, once known.
    pub account_id: Option<String>,
    /// Session it belongs to.
    pub session_id: Option<String>,
    /// Parent session, when a sub-agent produced this request.
    pub parent_session_id: Option<String>,
    /// Provider's request id, when logged.
    pub request_id: Option<String>,
    /// Provider's response id, when logged.
    pub response_id: Option<String>,
    /// Model that answered.
    pub model: Option<String>,
    /// What triggered the request (main agent, sub-agent, tool call), when the
    /// source distinguishes them.
    pub query_source: Option<String>,
    /// The token counts.
    pub usage: NormalizedUsage,
    /// Cost stated by the provider itself.
    pub provider_reported_cost: Option<f64>,
    /// Cost derived from a price table. Stays `None` while no price table
    /// exists — an unpriced request is unavailable, not free (spec §18.2).
    pub estimated_cost: Option<f64>,
    /// Currency of whichever cost is present.
    pub currency: Option<String>,
    /// HTTP status, when the source records one.
    pub http_status: Option<i64>,
    /// Round-trip latency in milliseconds.
    pub latency_ms: Option<i64>,
    /// Whether the request succeeded.
    pub success: Option<bool>,
    /// Confidence in the token counts.
    pub precision_token: PrecisionLevel,
    /// Confidence in the session attribution.
    pub precision_session: PrecisionLevel,
    /// Confidence in the provider attribution.
    pub precision_provider: PrecisionLevel,
    /// Confidence in the account attribution.
    pub precision_account: PrecisionLevel,
    /// Stable hash of the underlying source record. Re-importing the same log
    /// line produces the same hash, which is what makes imports idempotent.
    pub raw_event_hash: String,
    /// The source's own usage object, kept for auditing. Prompt text,
    /// completions, headers, and credentials are never included.
    pub raw_usage_json: Option<serde_json::Value>,
}

/// A configured data source and its health, as shown on the Sources page.
///
/// Health is stored rather than recomputed on demand so a source that failed
/// stays visibly failed instead of quietly disappearing between polls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceRecord {
    /// Stable source id (`codex-session`, `cockpit`, …).
    pub id: String,
    /// Which adapter reads it.
    pub adapter_type: String,
    /// Name shown in the UI.
    pub display_name: String,
    /// Where it is on disk, or the endpoint it is reached at.
    pub path_or_endpoint: Option<String>,
    /// Whether the user has this source turned on.
    pub enabled: bool,
    /// Schema or format version the adapter detected.
    pub detected_version: Option<String>,
    /// `healthy`, `not_found`, or `error`.
    pub health_status: Option<String>,
    /// Last time an import of this source succeeded.
    pub last_success_at: Option<DateTime<Utc>>,
    /// The most recent failure, kept so the UI can explain a stalled source.
    pub last_error: Option<String>,
    /// When TokenBuddy first saw this source.
    pub created_at: DateTime<Utc>,
    /// When this row was last written.
    pub updated_at: DateTime<Utc>,
}

/// The real provider/account behind a session, reported by a launcher that owns
/// the routing (CC-Switch, Cockpit).
///
/// Session logs record which model answered but never which upstream served it,
/// so provider identity would otherwise be guessed from the model name — which
/// is wrong whenever a relay or aggregator is in front (e.g. `deepseek-v4-pro`
/// served by DeepSeek but reached through a Claude-compatible endpoint). A
/// launcher that proxied the request knows the truth and states it here instead
/// of emitting its own token counts, which would double-count the session log
/// (spec §6.1 ranks session logs above proxy logs as the token source).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionProviderAttribution {
    /// The session's domain id as minted by the *native* session adapter, so the
    /// attribution lands on the same rows that adapter imported.
    pub session_id: String,
    /// The provider that actually served the session.
    pub provider_id: String,
    /// The account it was billed to, when the launcher knows it.
    pub account_id: Option<String>,
    /// Which launcher reported this.
    pub source_id: String,
}

/// A provider identity supplied by an adapter that knows the real routing (e.g.
/// CC-Switch), as opposed to one inferred from a model name. Persisted into the
/// `providers` table so the Providers view shows real names and upstream URLs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderRecord {
    /// Stable provider id.
    pub id: String,
    /// Vendor family (`openai`, `anthropic`, …), used for grouping.
    pub provider_family: String,
    /// Name shown in the UI.
    pub display_name: String,
    /// Endpoint requests were routed to, when the launcher reports it.
    pub upstream_url: Option<String>,
    /// Launcher that routes to this provider.
    pub launcher: Option<LauncherKind>,
    /// Source that reported this identity.
    pub source_id: Option<String>,
}

/// A provider with its aggregates, for the Providers page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderSummary {
    /// Stable provider id.
    pub id: String,
    /// Vendor family.
    pub provider_family: String,
    /// Name shown in the UI.
    pub display_name: String,
    /// Endpoint requests were routed to.
    pub upstream_url: Option<String>,
    /// Launcher that routes to it.
    pub launcher: Option<LauncherKind>,
    /// Source that reported it.
    pub source_id: Option<String>,
    /// Accounts grouped under this provider.
    pub account_count: u64,
    /// Requests attributed to it.
    pub request_count: u64,
    /// Successful requests, `None` when no event recorded an outcome.
    pub successful_request_count: Option<u64>,
    /// Success rate, `None` when no outcome was recorded — never 0%.
    pub success_rate_percent: Option<f64>,
    /// Mean latency over the events that reported one.
    pub average_latency_ms: Option<f64>,
    /// Token and cost totals.
    pub totals: UsageTotals,
}

/// An account identity reported by a source that can actually see one — the
/// Codex `auth.json` names the signed-in ChatGPT account — as opposed to the
/// synthetic per-provider placeholder the storage layer derives from a session
/// log when nothing better is known.
///
/// The identifying secret never reaches this struct: adapters hash it into
/// `account_fingerprint` (spec §20.2) and drop the original. `id` is derived
/// from that fingerprint so the same account keeps the same row across imports
/// without storing the raw account id, API key, or any OAuth token.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccountRecord {
    /// Domain id, derived from the fingerprint so it is stable without
    /// containing the identity it stands for.
    pub id: String,
    /// Provider this account belongs to.
    pub provider_id: String,
    /// Label for the UI — usually the account's email.
    pub display_name: Option<String>,
    /// Salted SHA-256 of the account id or credential (spec §20.2).
    pub account_fingerprint: String,
    /// How the account authenticates (`chatgpt`, `api_key`, `cockpit`, or
    /// `session_log` for the placeholder storage derives from a model name).
    pub auth_mode: String,
    /// Subscription plan, when the source states one.
    pub plan: Option<String>,
}

/// A period during which a launcher routed requests for one account.
///
/// Session logs never name an account, and `auth.json` only knows who is signed
/// in *now*, so neither can attribute history when a launcher rotates several
/// accounts. A launcher that proxied the requests does know which account served
/// them and when — that turns account attribution into a time-window match
/// (spec §17.2, `Correlated`) instead of a guess. Windows that overlap leave the
/// answer ambiguous, and an ambiguous answer is reported as unknown.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccountActivityWindow {
    /// The account that was serving during this period.
    pub account_id: String,
    /// Launcher that reported it.
    pub source_id: String,
    /// The app whose usage events this window may attribute.
    pub app: AppKind,
    /// Start of the period, padded to absorb timestamp skew between the proxy
    /// log and the app's own log.
    pub started_at: DateTime<Utc>,
    /// End of the period, padded the same way.
    pub ended_at: DateTime<Utc>,
}

/// Fingerprint an account identifier or credential for storage (spec §20.2).
///
/// This is the only way an account identity is allowed to reach the database:
/// the raw ChatGPT account id, API key, or OAuth token stays in the adapter that
/// read it and is dropped. `salt` is the per-install random value kept outside
/// `AppSettings`, so a copied database cannot be matched back to an account by
/// hashing a guess — the salt is needed too.
///
/// The separator byte keeps `salt + secret` unambiguous: without it, salts and
/// secrets that concatenate to the same bytes would fingerprint identically.
pub fn account_fingerprint(salt: &str, secret: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update([0x00]);
    hasher.update(secret.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// An account with the context the Accounts view needs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccountSummary {
    /// The stored identity.
    pub account: AccountRecord,
    /// Provider display name, resolved through the providers table.
    pub provider_name: Option<String>,
    /// Newest quota window for this account, if one was ever reported.
    pub latest_quota: Option<QuotaSummary>,
}

/// One quota reading at one moment — a point in a time series, unlike
/// [`QuotaSummary`], which is the newest reading for display.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuotaSnapshot {
    /// Content-derived id, so re-importing the same reading is a no-op rather
    /// than a second point in the series.
    pub id: String,
    /// Account this window belongs to.
    pub account_id: String,
    /// Account label, joined for display.
    pub account_name: Option<String>,
    /// Provider label, joined for display.
    pub provider_name: Option<String>,
    /// When the reading was taken.
    pub captured_at: DateTime<Utc>,
    /// Window this covers, labelled with its length when known.
    pub window_type: String,
    /// Percentage consumed.
    pub used_percent: Option<f64>,
    /// Percentage remaining.
    pub remaining_percent: Option<f64>,
    /// When the window resets.
    pub reset_at: Option<DateTime<Utc>>,
    /// Remaining prepaid credits.
    pub credits_remaining: Option<f64>,
    /// Trust level, decided by the weakest link.
    pub precision: PrecisionLevel,
    /// The source's own rate-limit object. Numbers only — no credentials.
    pub raw_json: Option<serde_json::Value>,
}

/// User-facing configuration, persisted by the Core.
///
/// The per-install fingerprint salt is deliberately *not* here: it lives in the
/// same table but never leaves storage, so it cannot reach the UI, the loopback
/// API, or an export (spec §20.2).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    /// Codex home, or `None` to use the platform default.
    pub codex_home: Option<String>,
    /// Claude home, or `None` to use the platform default.
    pub claude_home: Option<String>,
    /// CC-Switch database, read-only.
    pub cc_switch_db_path: Option<String>,
    /// Cockpit Tools database or directory, read-only.
    pub cockpit_path: Option<String>,
    /// Port for the OTel receiver (spec §5, not built).
    pub otel_port: Option<u16>,
    /// Launch TokenBuddy at login. Changed only when the user asks.
    pub auto_start: bool,
    /// Whether the optional local proxy may run (spec §12, not built).
    pub proxy_enabled: bool,
    /// Whether to keep request metadata beyond token counts.
    pub save_request_metadata: bool,
    /// Days of history to keep. `None` or zero keeps everything.
    pub data_retention_days: Option<u32>,
}

/// Filters shared by every query, so the dashboard, the session list, and an
/// export all narrow the data the same way.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UsageFilters {
    /// Inclusive start of the window.
    pub period_start: Option<DateTime<Utc>>,
    /// Exclusive end of the window.
    pub period_end: Option<DateTime<Utc>>,
    /// Restrict to one app.
    pub app: Option<AppKind>,
    /// Restrict to one provider.
    pub provider_id: Option<String>,
    /// Restrict to one account.
    pub account_id: Option<String>,
    /// Match the model name.
    pub model: Option<String>,
    /// Match the project path.
    pub project_path: Option<String>,
    /// Restrict to one token-precision level.
    pub precision: Option<PrecisionLevel>,
    /// Free-text search over title, project, session id, model, and request id.
    pub search: Option<String>,
}

/// A rendered export, ready to be written to disk or downloaded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExportResult {
    /// Suggested filename, including the extension.
    pub filename: String,
    /// MIME type for the download.
    pub mime_type: String,
    /// The rendered CSV or JSON. Excludes the raw source payload.
    pub content: String,
}

/// One conversation, as the app recorded it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionRecord {
    /// Domain id, derived from the source id and the external session id so
    /// every adapter mints the same id for the same session.
    pub id: String,
    /// The id the app itself used.
    pub external_session_id: Option<String>,
    /// Parent session, when a sub-agent produced this one.
    pub parent_session_id: Option<String>,
    /// App that ran it.
    pub app: AppKind,
    /// Launcher that routed it, when known.
    pub launcher: Option<LauncherKind>,
    /// Project directory it ran in.
    pub project_path: Option<String>,
    /// Human-readable title, when the app records one.
    pub title: Option<String>,
    /// Earliest event seen for this session across all imports.
    pub started_at: Option<DateTime<Utc>>,
    /// Latest event seen for this session across all imports.
    pub ended_at: Option<DateTime<Utc>>,
    /// Source that imported it.
    pub source_id: Option<String>,
    /// When TokenBuddy first saw it.
    pub created_at: DateTime<Utc>,
    /// When this row was last written.
    pub updated_at: DateTime<Utc>,
}

/// How far one resource has been imported, so the next pass reads only what is
/// new and a repeated pass changes nothing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportCursor {
    /// Source this cursor belongs to.
    pub source_id: String,
    /// The file or table within that source.
    pub resource_id: String,
    /// Size at the last read, used to notice truncation.
    pub file_size: Option<i64>,
    /// Modification time at the last read.
    pub modified_at: Option<DateTime<Utc>>,
    /// Where to resume. For table sources this holds the high-water timestamp
    /// instead of a byte position.
    pub byte_offset: i64,
    /// Hash of the file's first line, used to notice rotation: same path, new
    /// file. A mismatch restarts the import from the beginning.
    pub content_hash: Option<String>,
    /// The last cumulative total seen, so the next snapshot can be differenced
    /// instead of added.
    pub last_cumulative_usage: Option<NormalizedUsage>,
    /// Incremented when a cumulative counter rolls backwards, which separates
    /// the events of the new counter run from the old one in the event hash.
    pub snapshot_generation: i64,
    /// The session identity in force at `byte_offset`. Codex rollout files write
    /// the session UUID once in the header `session_meta` line; later
    /// `token_count` rows carry no id. An incremental import that resumes past
    /// the header must remember this identity, or every appended usage row is
    /// misattributed to the file-stem fallback and the session splits in two.
    pub last_session_id: Option<String>,
    /// The model in force at `byte_offset`. Codex rollout usage rows often omit
    /// the model after the header, so incremental imports must carry the last
    /// known value across the cursor boundary just like the session identity.
    pub last_model: Option<String>,
    /// When this cursor was last written.
    pub updated_at: DateTime<Utc>,
}

/// The answer to "is this source on this machine?".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DetectionResult {
    /// Source that was probed.
    pub source_id: String,
    /// Whether it was found.
    pub detected: bool,
    /// Where it was looked for, so a failed detection can be acted on.
    pub path_or_endpoint: Option<String>,
    /// Schema or format version, when detected.
    pub detected_version: Option<String>,
    /// Explanation for the user, in either outcome.
    pub message: Option<String>,
}

/// An adapter's self-reported health.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceHealth {
    /// Source being reported on.
    pub source_id: String,
    /// `healthy`, `not_found`, or `error`.
    pub status: String,
    /// Last successful import.
    pub last_success_at: Option<DateTime<Utc>>,
    /// The most recent failure.
    pub last_error: Option<String>,
}

/// Handle to a running watcher, returned by [`UsageAdapter::start_watch`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WatcherHandle {
    /// Source being watched.
    pub source_id: String,
}

/// Callback a watching adapter pushes events into.
pub type EventSink = Arc<dyn Fn(UsageEvent) + Send + Sync>;

/// An adapter failure, reduced to a message.
///
/// Source-specific error types stay inside their crates; only the text crosses
/// the boundary, so one adapter's error enum can change without touching the
/// application. A failing adapter degrades only itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdapterError {
    /// What went wrong, in terms a user can act on.
    pub message: String,
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AdapterError {}

/// Source adapters expose a common async contract. The first MVP adapters may
/// implement the file import synchronously internally; the async surface keeps
/// the application boundary ready for network and watcher-backed sources.
#[allow(async_fn_in_trait)]
pub trait UsageAdapter: Send + Sync {
    /// Stable source id this adapter reads.
    fn id(&self) -> &'static str;
    /// Name shown in the UI.
    fn display_name(&self) -> &'static str;
    /// Static metadata used by the Core's adapter registry and diagnostics.
    ///
    /// The default is intentionally conservative: a new adapter cannot
    /// accidentally advertise token or quota support before it has declared
    /// those semantics explicitly.
    fn descriptor(&self) -> AdapterDescriptor {
        AdapterDescriptor::minimal(self.id(), self.display_name())
    }

    /// Whether this source exists on this machine, and where.
    async fn detect(&self) -> Result<DetectionResult, AdapterError>;
    /// Read everything new since `cursor` and return it as one batch.
    ///
    /// Implementations must be idempotent: importing the same input twice may
    /// not change any stored count.
    async fn import_history(
        &self,
        cursor: Option<ImportCursor>,
    ) -> Result<ImportBatch, AdapterError>;
    /// Stream events as they appear, for sources that can push.
    ///
    /// The default refuses: file-backed adapters are driven by the Core's
    /// watcher instead, and pretending to watch would silently stop collection.
    async fn start_watch(&self, _sink: EventSink) -> Result<WatcherHandle, AdapterError> {
        Err(AdapterError {
            message: "file watching is not implemented by this adapter yet".to_owned(),
        })
    }
    /// Current health, for the Sources page.
    async fn health(&self) -> Result<SourceHealth, AdapterError>;
}

/// Everything one import pass produced, applied to storage in a single
/// transaction so a partial import never leaves half-attributed rows.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ImportBatch {
    /// Updated health for the source itself.
    pub source: Option<SourceRecord>,
    /// Provider identities the source can vouch for.
    pub providers: Vec<ProviderRecord>,
    /// Account identities the source can vouch for.
    pub accounts: Vec<AccountRecord>,
    /// Periods during which each account was serving.
    pub account_windows: Vec<AccountActivityWindow>,
    /// Session-level provider attributions.
    pub attributions: Vec<SessionProviderAttribution>,
    /// Sessions seen in this pass.
    pub sessions: Vec<SessionRecord>,
    /// Measured requests. Only token sources fill this; attribution-only
    /// sources leave it empty so the same request is not counted twice.
    pub usage_events: Vec<UsageEvent>,
    /// Official quota windows reported by the source. Kept separate from token
    /// usage on purpose (spec §8.4): a percentage never becomes a token count.
    pub quota_snapshots: Vec<QuotaSnapshot>,
    /// Updated cursors, so the next pass resumes here.
    pub cursors: Vec<ImportCursor>,
    /// Records that could not be parsed. Counted and surfaced rather than
    /// silently dropped, so a schema change is visible.
    pub skipped_records: usize,
}

/// Aggregated usage over a set of events.
///
/// A total is `None` unless *every* contributing event reported that field.
/// Summing only the events that happen to carry a value would present a partial
/// sum as a complete one, which is the single easiest way for an observability
/// tool to lie.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UsageTotals {
    /// How many events were aggregated.
    pub event_count: u64,
    /// Summed input tokens.
    pub input_tokens_total: Option<u64>,
    /// Summed uncached input tokens.
    pub input_tokens_uncached: Option<u64>,
    /// Summed cache-read tokens.
    pub cache_read_tokens: Option<u64>,
    /// Summed cache-write tokens.
    pub cache_write_tokens: Option<u64>,
    /// Summed output tokens.
    pub output_tokens_total: Option<u64>,
    /// Summed reasoning tokens.
    pub reasoning_tokens: Option<u64>,
    /// Summed visible output tokens.
    pub visible_output_tokens: Option<u64>,
    /// Summed provider-reported cost.
    pub provider_reported_cost: Option<f64>,
    /// Summed estimated cost.
    pub estimated_cost: Option<f64>,
    /// Cache hit rate across the set.
    pub cache_hit_rate_percent: Option<f64>,
}

impl UsageTotals {
    /// Totals for a single event's usage, with costs left unknown.
    pub fn from_usage(event_count: u64, usage: &NormalizedUsage) -> Self {
        Self {
            event_count,
            input_tokens_total: usage.input_tokens_total,
            input_tokens_uncached: usage.input_tokens_uncached,
            cache_read_tokens: usage.cache_read_tokens,
            cache_write_tokens: usage.cache_write_tokens,
            output_tokens_total: usage.output_tokens_total,
            reasoning_tokens: usage.reasoning_tokens,
            visible_output_tokens: usage.visible_output_tokens,
            provider_reported_cost: None,
            estimated_cost: None,
            cache_hit_rate_percent: usage.cache_hit_rate_percent(),
        }
    }

    /// Input plus output, or `None` if either side is unknown or the sum would
    /// overflow.
    pub fn total_tokens(&self) -> Option<u64> {
        match (self.input_tokens_total, self.output_tokens_total) {
            (Some(input), Some(output)) => input.checked_add(output),
            _ => None,
        }
    }
}

/// A session with its aggregates, for the session list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSummary {
    /// The session itself.
    pub session: SessionRecord,
    /// Totals over the events matching the active filters.
    pub totals: UsageTotals,
}

/// A session plus its request-level timeline, so a total can always be traced
/// back to the individual requests behind it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionDetail {
    /// The session and its totals.
    pub summary: SessionSummary,
    /// Every request in the session, oldest first.
    pub usage_events: Vec<UsageEvent>,
}

/// Totals for one window, with the window stated so the UI never has to guess
/// what it is showing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DashboardSummary {
    /// Inclusive start, defaulting to the start of the user's local day.
    pub period_start: DateTime<Utc>,
    /// Exclusive end.
    pub period_end: DateTime<Utc>,
    /// Totals over that window.
    pub totals: UsageTotals,
}

/// Usage grouped by the model that answered and the provider that served it.
/// `provider_name` resolves through the providers table so a relay shows its
/// real name rather than a synthetic id.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelUsage {
    /// Model that answered.
    pub model: Option<String>,
    /// Provider that served it.
    pub provider_id: Option<String>,
    /// That provider's display name.
    pub provider_name: Option<String>,
    /// App the requests came from.
    pub app: AppKind,
    /// Totals for this model and provider.
    pub totals: UsageTotals,
}

/// One page of events, with the unpaged count so the UI can show how much more
/// there is.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsageEventPage {
    /// Events on this page.
    pub events: Vec<UsageEvent>,
    /// Total matching the query, ignoring pagination.
    pub total: u64,
}

/// One page of sessions, with the unpaged count.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionPage {
    /// Sessions on this page.
    pub sessions: Vec<SessionSummary>,
    /// Total matching the query, ignoring pagination.
    pub total: u64,
}

/// A default location that was checked, and whether it was there — so path
/// detection can explain what it looked at rather than only saying "not found".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PathCandidate {
    /// The location checked.
    pub path: PathBuf,
    /// Whether it exists.
    pub exists: bool,
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        pin::pin,
        task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
    };

    use super::{
        AccountRecord, AdapterError, AppKind, CollectionStatus, IngestSource, LauncherKind,
        NormalizedUsage, PrecisionLevel, UsageAdapter, UsageTotals, account_fingerprint,
        correlation_key,
    };

    /// `as_str` is what reaches SQLite; the serde representation is what reaches
    /// the UI. They are documented as the same string, and a mismatch would let
    /// a value written by one path fail to parse on the other.
    #[test]
    fn stored_and_serialized_names_agree_for_every_enum_variant() {
        fn check<T: serde::Serialize + std::fmt::Display + Copy>(value: T, as_str: &str) {
            let json = serde_json::to_string(&value).expect("serialize");
            assert_eq!(json, format!("\"{as_str}\""), "serde name differs");
            assert_eq!(value.to_string(), as_str, "Display differs");
        }

        for (value, name) in [
            (AppKind::Codex, "codex"),
            (AppKind::ClaudeCode, "claude_code"),
            (AppKind::Unknown, "unknown"),
        ] {
            assert_eq!(value.as_str(), name);
            check(value, name);
        }
        for (value, name) in [
            (LauncherKind::Direct, "direct"),
            (LauncherKind::CCSwitch, "cc_switch"),
            (LauncherKind::Cockpit, "cockpit"),
            (LauncherKind::ObserverProxy, "observer_proxy"),
            (LauncherKind::Unknown, "unknown"),
        ] {
            assert_eq!(value.as_str(), name);
            check(value, name);
        }
        for (value, name) in [
            (IngestSource::SessionLog, "session_log"),
            (IngestSource::Otel, "otel"),
            (IngestSource::Proxy, "proxy"),
            (IngestSource::QuotaApi, "quota_api"),
            (IngestSource::ImportedDatabase, "imported_database"),
            (IngestSource::Estimated, "estimated"),
        ] {
            assert_eq!(value.as_str(), name);
            check(value, name);
        }
        for (value, name) in [
            (PrecisionLevel::Verified, "verified"),
            (PrecisionLevel::ExactSession, "exact_session"),
            (PrecisionLevel::Correlated, "correlated"),
            (PrecisionLevel::Estimated, "estimated"),
            (PrecisionLevel::Unavailable, "unavailable"),
        ] {
            assert_eq!(value.as_str(), name);
            check(value, name);
        }
        for (value, name) in [
            (CollectionStatus::Starting, "starting"),
            (CollectionStatus::Collecting, "collecting"),
            (CollectionStatus::Idle, "idle"),
            (CollectionStatus::Error, "error"),
        ] {
            assert_eq!(value.as_str(), name);
            check(value, name);
        }
    }

    #[test]
    fn correlation_keys_are_namespaced_and_precedence_is_explicit() {
        assert_eq!(
            correlation_key(AppKind::Codex, Some("req-1"), Some("resp-1")),
            Some("codex:request:req-1".to_owned())
        );
        assert_eq!(
            correlation_key(AppKind::ClaudeCode, None, Some("resp-1")),
            Some("claude_code:response:resp-1".to_owned())
        );
        assert_eq!(correlation_key(AppKind::Codex, Some(""), None), None);
        assert!(PrecisionLevel::Verified.precedence() > PrecisionLevel::ExactSession.precedence());
        assert!(IngestSource::Otel.precedence() > IngestSource::SessionLog.precedence());
    }

    #[test]
    fn an_unchanged_cumulative_snapshot_yields_no_event() {
        let snapshot = NormalizedUsage {
            input_tokens_total: Some(100),
            output_tokens_total: Some(40),
            ..Default::default()
        };
        // Same totals twice means the log repeated itself; recording a
        // zero-token event would inflate the event count for no usage.
        assert_eq!(snapshot.delta_from(&snapshot), None);
        // The raw subtraction still yields a value — it is zero, not absent —
        // which is exactly why `delta_from` has to reject it.
        let raw = snapshot.checked_delta(&snapshot).expect("subtractable");
        assert!(!raw.is_empty());
        assert!(raw.is_zero());
        assert_eq!(raw.input_tokens_total, Some(0));
    }

    #[test]
    fn a_field_absent_from_the_previous_snapshot_carries_through_whole() {
        let previous = NormalizedUsage {
            input_tokens_total: Some(100),
            ..Default::default()
        };
        let current = NormalizedUsage {
            input_tokens_total: Some(160),
            // The source only started reporting this field now, so the whole
            // value is new usage rather than an unmeasurable difference.
            reasoning_tokens: Some(12),
            ..Default::default()
        };

        let delta = current.delta_from(&previous).expect("delta");
        assert_eq!(delta.input_tokens_total, Some(60));
        assert_eq!(delta.reasoning_tokens, Some(12));
        // A field neither snapshot reported stays unknown.
        assert_eq!(delta.cache_read_tokens, None);
    }

    #[test]
    fn totals_from_a_single_event_leave_cost_unknown() {
        let usage = NormalizedUsage {
            input_tokens_total: Some(100),
            cache_read_tokens: Some(25),
            output_tokens_total: Some(40),
            ..Default::default()
        };
        let totals = UsageTotals::from_usage(1, &usage);

        assert_eq!(totals.event_count, 1);
        assert_eq!(totals.input_tokens_total, Some(100));
        assert_eq!(totals.cache_hit_rate_percent, Some(25.0));
        assert_eq!(totals.total_tokens(), Some(140));
        // No price table exists, so cost is unavailable — not zero (spec §18.2).
        assert_eq!(totals.provider_reported_cost, None);
        assert_eq!(totals.estimated_cost, None);
    }

    #[test]
    fn a_total_needs_both_sides_and_never_overflows() {
        let one_sided = UsageTotals {
            input_tokens_total: Some(100),
            ..Default::default()
        };
        assert_eq!(one_sided.total_tokens(), None);
        assert_eq!(UsageTotals::default().total_tokens(), None);

        let overflowing = UsageTotals {
            input_tokens_total: Some(u64::MAX),
            output_tokens_total: Some(1),
            ..Default::default()
        };
        assert_eq!(overflowing.total_tokens(), None);
    }

    #[test]
    fn adapter_errors_cross_the_boundary_as_their_message() {
        let error = AdapterError {
            message: "Codex session directory was not found".to_owned(),
        };
        assert_eq!(error.to_string(), "Codex session directory was not found");
        // It is a std error, so callers can chain it without a custom wrapper.
        let _: &dyn std::error::Error = &error;
    }

    #[test]
    fn watching_is_refused_by_default_rather_than_silently_doing_nothing() {
        struct FileBackedAdapter;

        impl UsageAdapter for FileBackedAdapter {
            fn id(&self) -> &'static str {
                "fixture"
            }
            fn display_name(&self) -> &'static str {
                "Fixture"
            }
            async fn detect(&self) -> Result<super::DetectionResult, AdapterError> {
                unimplemented!("not exercised")
            }
            async fn import_history(
                &self,
                _cursor: Option<super::ImportCursor>,
            ) -> Result<super::ImportBatch, AdapterError> {
                unimplemented!("not exercised")
            }
            async fn health(&self) -> Result<super::SourceHealth, AdapterError> {
                unimplemented!("not exercised")
            }
        }

        // An adapter that cannot push must say so: pretending to watch would
        // stop collection without any error surfacing.
        let refusal = futures_lite_block_on(FileBackedAdapter.start_watch(std::sync::Arc::new(
            |_event: super::UsageEvent| unreachable!("no events are pushed"),
        )));
        assert!(refusal.is_err());
    }

    /// Minimal executor: the trait is async but the default body never awaits,
    /// so polling once is enough and no runtime dependency is needed.
    fn futures_lite_block_on<F: Future>(future: F) -> F::Output {
        const VTABLE: RawWakerVTable =
            RawWakerVTable::new(|data| RawWaker::new(data, &VTABLE), |_| {}, |_| {}, |_| {});
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        match pin!(future).poll(&mut Context::from_waker(&waker)) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("the default implementation must not await"),
        }
    }

    #[test]
    fn a_fingerprint_hides_the_secret_and_depends_on_the_salt() {
        let secret = "acct-fixture-0001";
        let fingerprint = account_fingerprint("install-salt", secret);

        assert_eq!(fingerprint.len(), 64, "SHA-256 hex");
        assert!(!fingerprint.contains(secret));
        // Stable for the same pair, different for another install.
        assert_eq!(fingerprint, account_fingerprint("install-salt", secret));
        assert_ne!(fingerprint, account_fingerprint("other-salt", secret));
        // The separator keeps salt and secret from running together: without it
        // these two pairs would hash identically.
        assert_ne!(
            account_fingerprint("ab", "cd"),
            account_fingerprint("a", "bcd")
        );

        let record = AccountRecord {
            id: "openai:chatgpt:0123456789abcdef".to_owned(),
            provider_id: "openai".to_owned(),
            display_name: Some("user@example.com".to_owned()),
            account_fingerprint: fingerprint,
            auth_mode: "chatgpt".to_owned(),
            plan: Some("pro".to_owned()),
        };
        assert!(
            !serde_json::to_string(&record)
                .expect("json")
                .contains(secret)
        );
    }

    #[test]
    fn cache_hit_rate_requires_known_non_zero_totals() {
        assert_eq!(
            NormalizedUsage {
                input_tokens_total: Some(100),
                cache_read_tokens: Some(25),
                ..Default::default()
            }
            .cache_hit_rate_percent(),
            Some(25.0)
        );
        assert_eq!(
            NormalizedUsage {
                input_tokens_total: Some(0),
                cache_read_tokens: Some(0),
                ..Default::default()
            }
            .cache_hit_rate_percent(),
            None
        );
        assert_eq!(
            NormalizedUsage {
                input_tokens_total: Some(100),
                ..Default::default()
            }
            .cache_hit_rate_percent(),
            None
        );
    }

    #[test]
    fn cumulative_delta_never_creates_a_negative_value() {
        let previous = NormalizedUsage {
            input_tokens_total: Some(100),
            ..Default::default()
        };
        let current = NormalizedUsage {
            input_tokens_total: Some(90),
            ..Default::default()
        };

        assert_eq!(current.delta_from(&previous), None);
    }

    #[test]
    fn precision_names_are_stable_for_storage_and_ui() {
        assert_eq!(PrecisionLevel::ExactSession.as_str(), "exact_session");
        assert_eq!(PrecisionLevel::Unavailable.as_str(), "unavailable");
    }
}
