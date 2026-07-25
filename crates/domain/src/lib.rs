//! Shared, source-agnostic domain types for TokenBuddy.
//!
//! This crate deliberately has no dependency on Tauri or SQLite. Adapters
//! normalize source-specific records into these types and the storage crate
//! persists them for the desktop application.

use std::{fmt, path::PathBuf, sync::Arc};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AppKind {
    Codex,
    ClaudeCode,
    Unknown,
}

impl AppKind {
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LauncherKind {
    Direct,
    CCSwitch,
    Cockpit,
    ObserverProxy,
    Unknown,
}

impl LauncherKind {
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum IngestSource {
    SessionLog,
    Otel,
    Proxy,
    QuotaApi,
    ImportedDatabase,
    Estimated,
}

impl IngestSource {
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
}

impl fmt::Display for IngestSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str((*self).as_str())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PrecisionLevel {
    Verified,
    ExactSession,
    Correlated,
    Estimated,
    Unavailable,
}

impl PrecisionLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::ExactSession => "exact_session",
            Self::Correlated => "correlated",
            Self::Estimated => "estimated",
            Self::Unavailable => "unavailable",
        }
    }
}

impl fmt::Display for PrecisionLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str((*self).as_str())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NormalizedUsage {
    pub input_tokens_total: Option<u64>,
    pub input_tokens_uncached: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub output_tokens_total: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub visible_output_tokens: Option<u64>,
}

impl NormalizedUsage {
    pub fn is_empty(&self) -> bool {
        self.input_tokens_total.is_none()
            && self.input_tokens_uncached.is_none()
            && self.cache_read_tokens.is_none()
            && self.cache_write_tokens.is_none()
            && self.output_tokens_total.is_none()
            && self.reasoning_tokens.is_none()
            && self.visible_output_tokens.is_none()
    }

    pub fn cache_hit_rate_percent(&self) -> Option<f64> {
        let input_total = self.input_tokens_total?;
        let cache_read = self.cache_read_tokens?;
        if input_total == 0 || cache_read > input_total {
            return None;
        }

        Some((cache_read as f64 / input_total as f64) * 100.0)
    }

    pub fn delta_from(&self, previous: &Self) -> Option<Self> {
        let current = self.checked_delta(previous)?;
        if current.is_empty() {
            None
        } else {
            Some(current)
        }
    }

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsageEvent {
    pub id: String,
    pub occurred_at: DateTime<Utc>,
    pub app: AppKind,
    pub launcher: LauncherKind,
    pub ingest_source: IngestSource,
    pub source_id: String,
    pub provider_id: Option<String>,
    pub account_id: Option<String>,
    pub session_id: Option<String>,
    pub parent_session_id: Option<String>,
    pub request_id: Option<String>,
    pub response_id: Option<String>,
    pub model: Option<String>,
    pub query_source: Option<String>,
    pub usage: NormalizedUsage,
    pub provider_reported_cost: Option<f64>,
    pub estimated_cost: Option<f64>,
    pub currency: Option<String>,
    pub http_status: Option<i64>,
    pub latency_ms: Option<i64>,
    pub success: Option<bool>,
    pub precision_token: PrecisionLevel,
    pub precision_session: PrecisionLevel,
    pub precision_provider: PrecisionLevel,
    pub precision_account: PrecisionLevel,
    pub raw_event_hash: String,
    pub raw_usage_json: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceRecord {
    pub id: String,
    pub adapter_type: String,
    pub display_name: String,
    pub path_or_endpoint: Option<String>,
    pub enabled: bool,
    pub detected_version: Option<String>,
    pub health_status: Option<String>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionRecord {
    pub id: String,
    pub external_session_id: Option<String>,
    pub parent_session_id: Option<String>,
    pub app: AppKind,
    pub launcher: Option<LauncherKind>,
    pub project_path: Option<String>,
    pub title: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub source_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportCursor {
    pub source_id: String,
    pub resource_id: String,
    pub file_size: Option<i64>,
    pub modified_at: Option<DateTime<Utc>>,
    pub byte_offset: i64,
    pub content_hash: Option<String>,
    pub last_cumulative_usage: Option<NormalizedUsage>,
    pub snapshot_generation: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DetectionResult {
    pub source_id: String,
    pub detected: bool,
    pub path_or_endpoint: Option<String>,
    pub detected_version: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceHealth {
    pub source_id: String,
    pub status: String,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WatcherHandle {
    pub source_id: String,
}

pub type EventSink = Arc<dyn Fn(UsageEvent) + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdapterError {
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
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;

    async fn detect(&self) -> Result<DetectionResult, AdapterError>;
    async fn import_history(
        &self,
        cursor: Option<ImportCursor>,
    ) -> Result<ImportBatch, AdapterError>;
    async fn start_watch(&self, _sink: EventSink) -> Result<WatcherHandle, AdapterError> {
        Err(AdapterError {
            message: "file watching is not implemented by this adapter yet".to_owned(),
        })
    }
    async fn health(&self) -> Result<SourceHealth, AdapterError>;
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ImportBatch {
    pub source: Option<SourceRecord>,
    pub sessions: Vec<SessionRecord>,
    pub usage_events: Vec<UsageEvent>,
    pub cursors: Vec<ImportCursor>,
    pub skipped_records: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UsageTotals {
    pub event_count: u64,
    pub input_tokens_total: Option<u64>,
    pub input_tokens_uncached: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub output_tokens_total: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub visible_output_tokens: Option<u64>,
    pub provider_reported_cost: Option<f64>,
    pub estimated_cost: Option<f64>,
    pub cache_hit_rate_percent: Option<f64>,
}

impl UsageTotals {
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSummary {
    pub session: SessionRecord,
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionDetail {
    pub summary: SessionSummary,
    pub usage_events: Vec<UsageEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DashboardSummary {
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsageEventPage {
    pub events: Vec<UsageEvent>,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionPage {
    pub sessions: Vec<SessionSummary>,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PathCandidate {
    pub path: PathBuf,
    pub exists: bool,
}

#[cfg(test)]
mod tests {
    use super::{NormalizedUsage, PrecisionLevel};

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
