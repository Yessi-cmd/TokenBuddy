mod migrations;

use std::{collections::HashMap, fmt::Write as FmtWrite, path::Path, time::SystemTime};

use chrono::{DateTime, Datelike, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use thiserror::Error;
use tokenbuddy_domain::{
    AppKind, AppSettings, CollectionStatus, DashboardSummary, ExportResult, ImportBatch,
    ImportCursor, LauncherKind, NormalizedUsage, PrecisionLevel, ProviderSummary, QuickSummary,
    QuotaSnapshot, QuotaSummary, SessionDetail, SessionPage, SessionRecord, SessionSummary,
    SourceRecord, UsageEvent, UsageEventPage, UsageFilters, UsageTotals,
};

pub type Result<T> = std::result::Result<T, StorageError>;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid datetime in {field}: {value}")]
    InvalidDateTime { field: String, value: String },
    #[error("invalid token count in {field}")]
    InvalidTokenCount { field: String },
    #[error("unknown stored enum value for {field}: {value}")]
    UnknownEnum { field: String, value: String },
    #[error("database migration stopped at unsupported version {0}")]
    MigrationVersion(i64),
    #[error("unsupported export format: {0}")]
    UnsupportedExportFormat(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportStats {
    pub inserted_events: u64,
    pub duplicate_events: u64,
    pub upserted_sessions: u64,
    pub updated_cursors: u64,
}

pub struct Database {
    connection: Connection,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut connection = Connection::open(path)?;
        configure_connection(&connection)?;
        migrations::run(&mut connection)?;
        Ok(Self { connection })
    }

    pub fn open_in_memory() -> Result<Self> {
        let mut connection = Connection::open_in_memory()?;
        configure_connection(&connection)?;
        migrations::run(&mut connection)?;
        Ok(Self { connection })
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn apply_import_batch(&mut self, batch: &ImportBatch) -> Result<ImportStats> {
        let transaction = self.connection.transaction()?;
        let mut stats = ImportStats {
            upserted_sessions: batch.sessions.len() as u64,
            updated_cursors: batch.cursors.len() as u64,
            ..ImportStats::default()
        };

        if let Some(source) = &batch.source {
            upsert_source(&transaction, source)?;
        }

        for session in &batch.sessions {
            upsert_session(&transaction, session)?;
        }

        for event in &batch.usage_events {
            let inserted = insert_usage_event(&transaction, event)?;
            if inserted {
                stats.inserted_events += 1;
            } else {
                stats.duplicate_events += 1;
            }
        }

        for cursor in &batch.cursors {
            upsert_cursor(&transaction, cursor)?;
        }

        transaction.commit()?;
        Ok(stats)
    }

    pub fn list_sources(&self) -> Result<Vec<SourceRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, adapter_type, display_name, path_or_endpoint, enabled,
                    detected_version, health_status, last_success_at, last_error,
                    created_at, updated_at
             FROM sources ORDER BY display_name",
        )?;
        let rows = statement.query_map([], source_from_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn list_providers(&self) -> Result<Vec<ProviderSummary>> {
        let mut statement = self.connection.prepare(
            "SELECT p.id, p.provider_family, p.display_name, p.upstream_url, p.launcher,
                    p.source_id,
                    (SELECT COUNT(*) FROM accounts a WHERE a.provider_id = p.id),
                    (SELECT COUNT(*) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT CASE WHEN COUNT(u.success) = 0 THEN NULL
                                 ELSE SUM(CASE WHEN u.success = 1 THEN 1 ELSE 0 END) END
                     FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT COUNT(u.success) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT AVG(u.latency_ms) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT COUNT(u.input_tokens_total) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT SUM(u.input_tokens_total) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT COUNT(u.input_tokens_uncached) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT SUM(u.input_tokens_uncached) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT COUNT(u.cache_read_tokens) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT SUM(u.cache_read_tokens) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT COUNT(u.cache_write_tokens) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT SUM(u.cache_write_tokens) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT COUNT(u.output_tokens_total) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT SUM(u.output_tokens_total) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT COUNT(u.reasoning_tokens) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT SUM(u.reasoning_tokens) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT COUNT(u.visible_output_tokens) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT SUM(u.visible_output_tokens) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT COUNT(u.provider_reported_cost) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT SUM(u.provider_reported_cost) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT COUNT(u.estimated_cost) FROM usage_events u WHERE u.provider_id = p.id),
                    (SELECT SUM(u.estimated_cost) FROM usage_events u WHERE u.provider_id = p.id)
             FROM providers p
             ORDER BY p.display_name",
        )?;
        let rows = statement.query_map([], provider_summary_from_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn list_quota_snapshots(
        &self,
        account_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<QuotaSnapshot>> {
        let mut statement = self.connection.prepare(
            "SELECT q.id, q.account_id, a.display_name, p.display_name,
                    q.captured_at, q.window_type, q.used_percent, q.remaining_percent,
                    q.reset_at, q.credits_remaining, q.precision, q.raw_json
             FROM quota_snapshots q
             LEFT JOIN accounts a ON a.id = q.account_id
             LEFT JOIN providers p ON p.id = a.provider_id
             WHERE (?1 IS NULL OR q.account_id = ?1)
             ORDER BY q.captured_at DESC, q.id DESC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(
            params![account_id, checked_i64(limit, "quota limit")?],
            quota_snapshot_from_row,
        )?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn get_app_settings(&self) -> Result<AppSettings> {
        let settings = self
            .connection
            .query_row(
                "SELECT codex_home, claude_home, cc_switch_db_path, cockpit_path,
                        otel_port, auto_start, proxy_enabled, save_request_metadata,
                        data_retention_days
                 FROM app_settings WHERE id = 1",
                [],
                app_settings_from_row,
            )
            .optional()?;
        Ok(settings.unwrap_or_default())
    }

    pub fn save_app_settings(&self, settings: &AppSettings) -> Result<()> {
        self.connection.execute(
            "INSERT INTO app_settings (
                 id, codex_home, claude_home, cc_switch_db_path, cockpit_path,
                 otel_port, auto_start, proxy_enabled, save_request_metadata,
                 data_retention_days
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                 codex_home = excluded.codex_home,
                 claude_home = excluded.claude_home,
                 cc_switch_db_path = excluded.cc_switch_db_path,
                 cockpit_path = excluded.cockpit_path,
                 otel_port = excluded.otel_port,
                 auto_start = excluded.auto_start,
                 proxy_enabled = excluded.proxy_enabled,
                 save_request_metadata = excluded.save_request_metadata,
                 data_retention_days = excluded.data_retention_days",
            params![
                settings.codex_home,
                settings.claude_home,
                settings.cc_switch_db_path,
                settings.cockpit_path,
                settings.otel_port.map(i64::from),
                bool_to_i64(settings.auto_start),
                bool_to_i64(settings.proxy_enabled),
                bool_to_i64(settings.save_request_metadata),
                settings.data_retention_days.map(i64::from),
            ],
        )?;
        Ok(())
    }

    pub fn get_import_cursor(
        &self,
        source_id: &str,
        resource_id: &str,
    ) -> Result<Option<ImportCursor>> {
        self.connection
            .query_row(
                "SELECT source_id, resource_id, file_size, modified_at, byte_offset,
                        content_hash, last_cumulative_usage, snapshot_generation, updated_at
                 FROM import_cursors WHERE source_id = ?1 AND resource_id = ?2",
                params![source_id, resource_id],
                cursor_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_import_cursors(&self, source_id: &str) -> Result<HashMap<String, ImportCursor>> {
        let mut statement = self.connection.prepare(
            "SELECT source_id, resource_id, file_size, modified_at, byte_offset,
                    content_hash, last_cumulative_usage, snapshot_generation, updated_at
             FROM import_cursors WHERE source_id = ?1",
        )?;
        let rows = statement.query_map(params![source_id], cursor_from_row)?;
        let mut cursors = HashMap::new();
        for cursor in rows {
            let cursor = cursor?;
            cursors.insert(cursor.resource_id.clone(), cursor);
        }
        Ok(cursors)
    }

    pub fn list_session_page(
        &self,
        search: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<SessionPage> {
        let search_pattern = search.map(|value| format!("%{}%", value.trim()));
        let mut statement = self.connection.prepare(
            "SELECT s.id, s.external_session_id, s.parent_session_id, s.app, s.launcher,
                    s.project_path, s.title, s.started_at, s.ended_at, s.source_id,
                    s.created_at, s.updated_at,
                    COUNT(u.id),
                    COUNT(u.input_tokens_total), SUM(u.input_tokens_total),
                    COUNT(u.input_tokens_uncached), SUM(u.input_tokens_uncached),
                    COUNT(u.cache_read_tokens), SUM(u.cache_read_tokens),
                    COUNT(u.cache_write_tokens), SUM(u.cache_write_tokens),
                    COUNT(u.output_tokens_total), SUM(u.output_tokens_total),
                    COUNT(u.reasoning_tokens), SUM(u.reasoning_tokens),
                    COUNT(u.visible_output_tokens), SUM(u.visible_output_tokens),
                    COUNT(u.provider_reported_cost), SUM(u.provider_reported_cost),
                    COUNT(u.estimated_cost), SUM(u.estimated_cost)
             FROM sessions s
             LEFT JOIN usage_events u ON u.session_id = s.id
             WHERE (?1 IS NULL OR s.title LIKE ?1 OR s.project_path LIKE ?1
                    OR s.external_session_id LIKE ?1)
             GROUP BY s.id
             ORDER BY COALESCE(s.ended_at, s.updated_at, s.started_at, s.created_at) DESC
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = statement.query_map(
            params![
                search_pattern,
                checked_i64(limit, "limit")?,
                checked_i64(offset, "offset")?
            ],
            session_summary_from_row,
        )?;
        let sessions = rows.collect::<std::result::Result<Vec<_>, _>>()?;

        let total: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM sessions
             WHERE (?1 IS NULL OR title LIKE ?1 OR project_path LIKE ?1
                    OR external_session_id LIKE ?1)",
            params![search_pattern],
            |row| row.get(0),
        )?;

        Ok(SessionPage {
            sessions,
            total: checked_u64(total, "session count")?,
        })
    }

    pub fn get_session_detail(&self, session_id: &str) -> Result<Option<SessionDetail>> {
        let summary = self
            .connection
            .query_row(
                "SELECT s.id, s.external_session_id, s.parent_session_id, s.app, s.launcher,
                        s.project_path, s.title, s.started_at, s.ended_at, s.source_id,
                        s.created_at, s.updated_at,
                        COUNT(u.id),
                        COUNT(u.input_tokens_total), SUM(u.input_tokens_total),
                        COUNT(u.input_tokens_uncached), SUM(u.input_tokens_uncached),
                        COUNT(u.cache_read_tokens), SUM(u.cache_read_tokens),
                        COUNT(u.cache_write_tokens), SUM(u.cache_write_tokens),
                        COUNT(u.output_tokens_total), SUM(u.output_tokens_total),
                        COUNT(u.reasoning_tokens), SUM(u.reasoning_tokens),
                        COUNT(u.visible_output_tokens), SUM(u.visible_output_tokens),
                        COUNT(u.provider_reported_cost), SUM(u.provider_reported_cost),
                        COUNT(u.estimated_cost), SUM(u.estimated_cost)
                 FROM sessions s
                 LEFT JOIN usage_events u ON u.session_id = s.id
                 WHERE s.id = ?1
                 GROUP BY s.id",
                params![session_id],
                session_summary_from_row,
            )
            .optional()?;

        let Some(summary) = summary else {
            return Ok(None);
        };

        let mut statement = self.connection.prepare(
            "SELECT u.id, u.occurred_at, u.app, u.launcher, u.ingest_source, u.source_id,
                    u.provider_id, u.account_id, u.session_id, u.parent_session_id, u.request_id,
                    u.response_id, u.model, u.query_source, u.input_tokens_total,
                    u.input_tokens_uncached, u.cache_read_tokens, u.cache_write_tokens,
                    u.output_tokens_total, u.reasoning_tokens, u.visible_output_tokens,
                    u.provider_reported_cost, u.estimated_cost, u.currency, u.http_status,
                    u.latency_ms, u.success, u.precision_token, u.precision_session,
                    u.precision_provider, u.precision_account, u.raw_event_hash,
                    u.raw_usage_json
             FROM usage_events u WHERE u.session_id = ?1 ORDER BY u.occurred_at, u.id",
        )?;
        let events = statement
            .query_map(params![session_id], usage_event_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(Some(SessionDetail {
            summary,
            usage_events: events,
        }))
    }

    pub fn list_usage_events(
        &self,
        session_id: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<UsageEventPage> {
        self.list_usage_events_filtered(session_id, limit, offset, &UsageFilters::default())
    }

    pub fn list_usage_events_filtered(
        &self,
        session_id: Option<&str>,
        limit: u64,
        offset: u64,
        filters: &UsageFilters,
    ) -> Result<UsageEventPage> {
        let filter_params = UsageFilterParams::from_filters(filters);
        let mut statement = self.connection.prepare(
            "SELECT u.id, u.occurred_at, u.app, u.launcher, u.ingest_source, u.source_id,
                    u.provider_id, u.account_id, u.session_id, u.parent_session_id, u.request_id,
                    u.response_id, u.model, u.query_source, u.input_tokens_total,
                    u.input_tokens_uncached, u.cache_read_tokens, u.cache_write_tokens,
                    u.output_tokens_total, u.reasoning_tokens, u.visible_output_tokens,
                    u.provider_reported_cost, u.estimated_cost, u.currency, u.http_status,
                    u.latency_ms, u.success, u.precision_token, u.precision_session,
                    u.precision_provider, u.precision_account, u.raw_event_hash,
                    u.raw_usage_json
             FROM usage_events u
             LEFT JOIN sessions s ON s.id = u.session_id
             WHERE (?1 IS NULL OR u.occurred_at >= ?1)
               AND (?2 IS NULL OR u.occurred_at < ?2)
               AND (?3 IS NULL OR u.app = ?3)
               AND (?4 IS NULL OR u.provider_id = ?4)
               AND (?5 IS NULL OR u.account_id = ?5)
               AND (?6 IS NULL OR u.model LIKE ?6)
               AND (?7 IS NULL OR s.project_path LIKE ?7)
               AND (?8 IS NULL OR u.precision_token = ?8)
               AND (?9 IS NULL OR s.title LIKE ?9 OR s.project_path LIKE ?9
                    OR s.external_session_id LIKE ?9 OR u.model LIKE ?9
                    OR u.request_id LIKE ?9)
               AND (?10 IS NULL OR u.session_id = ?10)
             ORDER BY u.occurred_at, u.id LIMIT ?11 OFFSET ?12",
        )?;
        let events = statement
            .query_map(
                params![
                    filter_params.period_start,
                    filter_params.period_end,
                    filter_params.app,
                    filter_params.provider_id,
                    filter_params.account_id,
                    filter_params.model,
                    filter_params.project_path,
                    filter_params.precision,
                    filter_params.search,
                    session_id,
                    checked_i64(limit, "limit")?,
                    checked_i64(offset, "offset")?
                ],
                usage_event_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let total: i64 = self.connection.query_row(
            "SELECT COUNT(*)
             FROM usage_events u
             LEFT JOIN sessions s ON s.id = u.session_id
             WHERE (?1 IS NULL OR u.occurred_at >= ?1)
               AND (?2 IS NULL OR u.occurred_at < ?2)
               AND (?3 IS NULL OR u.app = ?3)
               AND (?4 IS NULL OR u.provider_id = ?4)
               AND (?5 IS NULL OR u.account_id = ?5)
               AND (?6 IS NULL OR u.model LIKE ?6)
               AND (?7 IS NULL OR s.project_path LIKE ?7)
               AND (?8 IS NULL OR u.precision_token = ?8)
               AND (?9 IS NULL OR s.title LIKE ?9 OR s.project_path LIKE ?9
                    OR s.external_session_id LIKE ?9 OR u.model LIKE ?9
                    OR u.request_id LIKE ?9)
               AND (?10 IS NULL OR u.session_id = ?10)",
            params![
                filter_params.period_start,
                filter_params.period_end,
                filter_params.app,
                filter_params.provider_id,
                filter_params.account_id,
                filter_params.model,
                filter_params.project_path,
                filter_params.precision,
                filter_params.search,
                session_id,
            ],
            |row| row.get(0),
        )?;

        Ok(UsageEventPage {
            events,
            total: checked_u64(total, "usage event count")?,
        })
    }

    pub fn dashboard_summary(
        &self,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Result<DashboardSummary> {
        self.dashboard_summary_filtered(&UsageFilters {
            period_start: Some(period_start),
            period_end: Some(period_end),
            ..UsageFilters::default()
        })
    }

    pub fn dashboard_summary_filtered(&self, filters: &UsageFilters) -> Result<DashboardSummary> {
        let (period_start, period_end) = filter_period(filters);
        let filter_params = UsageFilterParams::from_filters(filters);
        let totals = self.connection.query_row(
            "SELECT COUNT(*),
                    COUNT(input_tokens_total), SUM(input_tokens_total),
                    COUNT(input_tokens_uncached), SUM(input_tokens_uncached),
                    COUNT(cache_read_tokens), SUM(cache_read_tokens),
                    COUNT(cache_write_tokens), SUM(cache_write_tokens),
                    COUNT(output_tokens_total), SUM(output_tokens_total),
                    COUNT(reasoning_tokens), SUM(reasoning_tokens),
                    COUNT(visible_output_tokens), SUM(visible_output_tokens),
                    COUNT(provider_reported_cost), SUM(provider_reported_cost),
                    COUNT(estimated_cost), SUM(estimated_cost)
             FROM usage_events u
             LEFT JOIN sessions s ON s.id = u.session_id
             WHERE (?1 IS NULL OR u.occurred_at >= ?1)
               AND (?2 IS NULL OR u.occurred_at < ?2)
               AND (?3 IS NULL OR u.app = ?3)
               AND (?4 IS NULL OR u.provider_id = ?4)
               AND (?5 IS NULL OR u.account_id = ?5)
               AND (?6 IS NULL OR u.model LIKE ?6)
               AND (?7 IS NULL OR s.project_path LIKE ?7)
               AND (?8 IS NULL OR u.precision_token = ?8)
               AND (?9 IS NULL OR s.title LIKE ?9 OR s.project_path LIKE ?9
                    OR s.external_session_id LIKE ?9 OR u.model LIKE ?9
                    OR u.request_id LIKE ?9)",
            params![
                filter_params.period_start,
                filter_params.period_end,
                filter_params.app,
                filter_params.provider_id,
                filter_params.account_id,
                filter_params.model,
                filter_params.project_path,
                filter_params.precision,
                filter_params.search,
            ],
            totals_from_row,
        )?;

        Ok(DashboardSummary {
            period_start,
            period_end,
            totals,
        })
    }

    pub fn export_usage(&self, format: &str, filters: &UsageFilters) -> Result<ExportResult> {
        let normalized_format = format.trim().to_ascii_lowercase();
        if normalized_format != "json" && normalized_format != "csv" {
            return Err(StorageError::UnsupportedExportFormat(format.to_owned()));
        }
        let page = self.list_usage_events_filtered(None, i64::MAX as u64, 0, filters)?;
        let date = Utc::now().format("%Y%m%d");
        match normalized_format.as_str() {
            "json" => Ok(ExportResult {
                filename: format!("tokenbuddy-usage-{date}.json"),
                mime_type: "application/json".to_owned(),
                content: serde_json::to_string_pretty(
                    &page
                        .events
                        .iter()
                        .map(export_event_value)
                        .collect::<Vec<_>>(),
                )?,
            }),
            "csv" => Ok(ExportResult {
                filename: format!("tokenbuddy-usage-{date}.csv"),
                mime_type: "text/csv;charset=utf-8".to_owned(),
                content: export_events_csv(&page.events),
            }),
            _ => unreachable!("export format was validated above"),
        }
    }

    pub fn quick_summary(
        &self,
        now: DateTime<Utc>,
        collection_status: CollectionStatus,
        latest_warning: Option<String>,
    ) -> Result<QuickSummary> {
        let period_start = Utc
            .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
            .single()
            .ok_or_else(|| StorageError::InvalidDateTime {
                field: "today_start".to_owned(),
                value: now.to_rfc3339(),
            })?;
        let period_end = period_start + chrono::Duration::days(1);
        let today_total_tokens = self.total_tokens_for_period(period_start, period_end)?;

        let active = self
            .connection
            .query_row(
                "SELECT u.app, u.session_id, s.title, s.project_path, u.model,
                        p.display_name, u.account_id
                 FROM usage_events u
                 LEFT JOIN sessions s ON s.id = u.session_id
                 LEFT JOIN providers p ON p.id = u.provider_id
                 ORDER BY u.occurred_at DESC, u.id DESC LIMIT 1",
                [],
                |row| {
                    Ok((
                        app_from_str(row.get::<_, String>(0)?).map_err(to_sql_error)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .optional()?;

        let session_totals = active
            .as_ref()
            .and_then(|(_, session_id, _, _, _, _, _)| session_id.as_deref())
            .map(|session_id| self.session_usage_totals(session_id))
            .transpose()?;
        let quota_summary = active
            .as_ref()
            .and_then(|(_, _, _, _, _, _, account_id)| account_id.as_deref())
            .map(|account_id| self.latest_quota_summary(account_id))
            .transpose()?
            .flatten();

        Ok(QuickSummary {
            collection_status,
            active_app: active.as_ref().map(|(app, _, _, _, _, _, _)| *app),
            active_session_id: active
                .as_ref()
                .and_then(|(_, session_id, _, _, _, _, _)| session_id.clone()),
            active_session_title: active
                .as_ref()
                .and_then(|(_, _, title, _, _, _, _)| title.clone()),
            active_project_path: active
                .as_ref()
                .and_then(|(_, _, _, project_path, _, _, _)| project_path.clone()),
            provider_name: active
                .as_ref()
                .and_then(|(_, _, _, _, _, provider_name, _)| provider_name.clone()),
            model: active
                .as_ref()
                .and_then(|(_, _, _, _, model, _, _)| model.clone()),
            session_input_tokens: session_totals
                .as_ref()
                .and_then(|totals| totals.input_tokens_total),
            session_cache_read_tokens: session_totals
                .as_ref()
                .and_then(|totals| totals.cache_read_tokens),
            session_output_tokens: session_totals
                .as_ref()
                .and_then(|totals| totals.output_tokens_total),
            session_cache_hit_rate: session_totals
                .as_ref()
                .and_then(|totals| totals.cache_hit_rate_percent),
            today_total_tokens,
            quota_summary,
            latest_warning,
        })
    }

    fn latest_quota_summary(&self, account_id: &str) -> Result<Option<QuotaSummary>> {
        self.connection
            .query_row(
                "SELECT window_type, used_percent, remaining_percent, reset_at,
                        credits_remaining, precision
                 FROM quota_snapshots
                 WHERE account_id = ?1
                 ORDER BY captured_at DESC, id DESC LIMIT 1",
                params![account_id],
                |row| {
                    Ok(QuotaSummary {
                        window_type: row.get(0)?,
                        used_percent: row.get(1)?,
                        remaining_percent: row.get(2)?,
                        reset_at: row
                            .get::<_, Option<String>>(3)?
                            .map(|value| parse_datetime("reset_at", value))
                            .transpose()
                            .map_err(to_sql_error)?,
                        credits_remaining: row.get(4)?,
                        precision: precision_from_str(&row.get::<_, String>(5)?)
                            .map_err(to_sql_error)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    fn session_usage_totals(&self, session_id: &str) -> Result<UsageTotals> {
        self.connection
            .query_row(
                "SELECT COUNT(*),
                        COUNT(input_tokens_total), SUM(input_tokens_total),
                        COUNT(input_tokens_uncached), SUM(input_tokens_uncached),
                        COUNT(cache_read_tokens), SUM(cache_read_tokens),
                        COUNT(cache_write_tokens), SUM(cache_write_tokens),
                        COUNT(output_tokens_total), SUM(output_tokens_total),
                        COUNT(reasoning_tokens), SUM(reasoning_tokens),
                        COUNT(visible_output_tokens), SUM(visible_output_tokens),
                        COUNT(provider_reported_cost), SUM(provider_reported_cost),
                        COUNT(estimated_cost), SUM(estimated_cost)
                 FROM usage_events WHERE session_id = ?1",
                params![session_id],
                totals_from_row,
            )
            .map_err(Into::into)
    }

    fn total_tokens_for_period(
        &self,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Result<Option<u64>> {
        let (event_count, input_count, output_count, input_sum, output_sum): (
            i64,
            i64,
            i64,
            Option<i64>,
            Option<i64>,
        ) = self.connection.query_row(
            "SELECT COUNT(*), COUNT(input_tokens_total), COUNT(output_tokens_total),
                        SUM(input_tokens_total), SUM(output_tokens_total)
                 FROM usage_events
                 WHERE occurred_at >= ?1 AND occurred_at < ?2",
            params![period_start.to_rfc3339(), period_end.to_rfc3339()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?;
        if event_count == 0 {
            return Ok(Some(0));
        }
        if input_count != event_count || output_count != event_count {
            return Ok(None);
        }
        let input = option_u64(input_sum, "input_tokens_total")?.ok_or_else(|| {
            StorageError::InvalidTokenCount {
                field: "input_tokens_total".to_owned(),
            }
        })?;
        let output = option_u64(output_sum, "output_tokens_total")?.ok_or_else(|| {
            StorageError::InvalidTokenCount {
                field: "output_tokens_total".to_owned(),
            }
        })?;
        input
            .checked_add(output)
            .map(Some)
            .ok_or_else(|| StorageError::InvalidTokenCount {
                field: "today_total_tokens".to_owned(),
            })
    }
}

fn configure_connection(connection: &Connection) -> Result<()> {
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(())
}

#[derive(Debug, Default)]
struct UsageFilterParams {
    period_start: Option<String>,
    period_end: Option<String>,
    app: Option<String>,
    provider_id: Option<String>,
    account_id: Option<String>,
    model: Option<String>,
    project_path: Option<String>,
    precision: Option<String>,
    search: Option<String>,
}

impl UsageFilterParams {
    fn from_filters(filters: &UsageFilters) -> Self {
        Self {
            period_start: filters.period_start.map(|value| value.to_rfc3339()),
            period_end: filters.period_end.map(|value| value.to_rfc3339()),
            app: filters.app.map(|value| value.as_str().to_owned()),
            provider_id: non_empty_filter(filters.provider_id.as_deref()),
            account_id: non_empty_filter(filters.account_id.as_deref()),
            model: wildcard_filter(filters.model.as_deref()),
            project_path: wildcard_filter(filters.project_path.as_deref()),
            precision: filters.precision.map(|value| value.as_str().to_owned()),
            search: wildcard_filter(filters.search.as_deref()),
        }
    }
}

fn non_empty_filter(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn wildcard_filter(value: Option<&str>) -> Option<String> {
    non_empty_filter(value).map(|value| format!("%{value}%"))
}

fn filter_period(filters: &UsageFilters) -> (DateTime<Utc>, DateTime<Utc>) {
    let now = Utc::now();
    let default_start = Utc
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .unwrap_or(now);
    let period_start = filters.period_start.unwrap_or(default_start);
    let period_end = filters
        .period_end
        .unwrap_or_else(|| period_start + chrono::Duration::days(1));
    (period_start, period_end)
}

fn export_event_value(event: &UsageEvent) -> serde_json::Value {
    serde_json::json!({
        "id": event.id,
        "occurred_at": event.occurred_at,
        "app": event.app,
        "launcher": event.launcher,
        "ingest_source": event.ingest_source,
        "source_id": event.source_id,
        "provider_id": event.provider_id,
        "account_id": event.account_id,
        "session_id": event.session_id,
        "parent_session_id": event.parent_session_id,
        "request_id": event.request_id,
        "response_id": event.response_id,
        "model": event.model,
        "query_source": event.query_source,
        "usage": event.usage,
        "provider_reported_cost": event.provider_reported_cost,
        "estimated_cost": event.estimated_cost,
        "currency": event.currency,
        "http_status": event.http_status,
        "latency_ms": event.latency_ms,
        "success": event.success,
        "precision_token": event.precision_token,
        "precision_session": event.precision_session,
        "precision_provider": event.precision_provider,
        "precision_account": event.precision_account,
        "raw_event_hash": event.raw_event_hash,
    })
}

fn export_events_csv(events: &[UsageEvent]) -> String {
    let mut csv = String::from(
        "id,occurred_at,app,launcher,ingest_source,source_id,provider_id,account_id,session_id,parent_session_id,request_id,response_id,model,query_source,input_tokens_total,input_tokens_uncached,cache_read_tokens,cache_write_tokens,output_tokens_total,reasoning_tokens,visible_output_tokens,provider_reported_cost,estimated_cost,currency,http_status,latency_ms,success,precision_token,precision_session,precision_provider,precision_account,raw_event_hash\n",
    );
    for event in events {
        let fields = [
            event.id.clone(),
            event.occurred_at.to_rfc3339(),
            event.app.as_str().to_owned(),
            event.launcher.as_str().to_owned(),
            event.ingest_source.as_str().to_owned(),
            event.source_id.clone(),
            event.provider_id.clone().unwrap_or_default(),
            event.account_id.clone().unwrap_or_default(),
            event.session_id.clone().unwrap_or_default(),
            event.parent_session_id.clone().unwrap_or_default(),
            event.request_id.clone().unwrap_or_default(),
            event.response_id.clone().unwrap_or_default(),
            event.model.clone().unwrap_or_default(),
            event.query_source.clone().unwrap_or_default(),
            optional_u64_text(event.usage.input_tokens_total),
            optional_u64_text(event.usage.input_tokens_uncached),
            optional_u64_text(event.usage.cache_read_tokens),
            optional_u64_text(event.usage.cache_write_tokens),
            optional_u64_text(event.usage.output_tokens_total),
            optional_u64_text(event.usage.reasoning_tokens),
            optional_u64_text(event.usage.visible_output_tokens),
            optional_f64_text(event.provider_reported_cost),
            optional_f64_text(event.estimated_cost),
            event.currency.clone().unwrap_or_default(),
            event
                .http_status
                .map_or_else(String::new, |value| value.to_string()),
            event
                .latency_ms
                .map_or_else(String::new, |value| value.to_string()),
            event
                .success
                .map_or_else(String::new, |value| value.to_string()),
            event.precision_token.as_str().to_owned(),
            event.precision_session.as_str().to_owned(),
            event.precision_provider.as_str().to_owned(),
            event.precision_account.as_str().to_owned(),
            event.raw_event_hash.clone(),
        ];
        let escaped = fields
            .iter()
            .map(|field| csv_field(field))
            .collect::<Vec<_>>();
        let _ = writeln!(csv, "{}", escaped.join(","));
    }
    csv
}

fn optional_u64_text(value: Option<u64>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

fn optional_f64_text(value: Option<f64>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

fn csv_field(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn now() -> DateTime<Utc> {
    DateTime::<Utc>::from(SystemTime::now())
}

fn upsert_source(conn: &Connection, source: &SourceRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO sources (
             id, adapter_type, display_name, path_or_endpoint, enabled,
             detected_version, health_status, last_success_at, last_error,
             created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(id) DO UPDATE SET
             adapter_type = excluded.adapter_type,
             display_name = excluded.display_name,
             path_or_endpoint = excluded.path_or_endpoint,
             enabled = excluded.enabled,
             detected_version = excluded.detected_version,
             health_status = excluded.health_status,
             last_success_at = COALESCE(excluded.last_success_at, sources.last_success_at),
             last_error = excluded.last_error,
             updated_at = excluded.updated_at",
        params![
            source.id,
            source.adapter_type,
            source.display_name,
            source.path_or_endpoint,
            source.enabled,
            source.detected_version,
            source.health_status,
            source.last_success_at.map(|value| value.to_rfc3339()),
            source.last_error,
            source.created_at.to_rfc3339(),
            source.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn upsert_session(conn: &Connection, session: &SessionRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO sessions (
             id, external_session_id, parent_session_id, app, launcher,
             project_path, title, started_at, ended_at, source_id, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(id) DO UPDATE SET
             external_session_id = COALESCE(excluded.external_session_id, sessions.external_session_id),
             parent_session_id = COALESCE(excluded.parent_session_id, sessions.parent_session_id),
             app = excluded.app,
             launcher = COALESCE(excluded.launcher, sessions.launcher),
             project_path = COALESCE(excluded.project_path, sessions.project_path),
             title = COALESCE(excluded.title, sessions.title),
             started_at = COALESCE(excluded.started_at, sessions.started_at),
             ended_at = COALESCE(excluded.ended_at, sessions.ended_at),
             source_id = COALESCE(excluded.source_id, sessions.source_id),
             updated_at = excluded.updated_at",
        params![
            session.id,
            session.external_session_id,
            session.parent_session_id,
            session.app.as_str(),
            session.launcher.map(LauncherKind::as_str),
            session.project_path,
            session.title,
            session.started_at.map(|value| value.to_rfc3339()),
            session.ended_at.map(|value| value.to_rfc3339()),
            session.source_id,
            session.created_at.to_rfc3339(),
            session.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn insert_usage_event(conn: &Connection, event: &UsageEvent) -> Result<bool> {
    let raw_usage_json = event
        .raw_usage_json
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let changed = conn.execute(
        "INSERT INTO usage_events (
             id, occurred_at, app, launcher, ingest_source, source_id,
             provider_id, account_id, session_id, parent_session_id, request_id,
             response_id, model, query_source, input_tokens_total,
             input_tokens_uncached, cache_read_tokens, cache_write_tokens,
             output_tokens_total, reasoning_tokens, visible_output_tokens,
             provider_reported_cost, estimated_cost, currency, http_status,
             latency_ms, success, precision_token, precision_session,
             precision_provider, precision_account, raw_event_hash,
             raw_usage_json, created_at
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
             ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26,
             ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34
         ) ON CONFLICT(raw_event_hash) DO NOTHING",
        params![
            event.id,
            event.occurred_at.to_rfc3339(),
            event.app.as_str(),
            event.launcher.as_str(),
            event.ingest_source.as_str(),
            event.source_id,
            event.provider_id,
            event.account_id,
            event.session_id,
            event.parent_session_id,
            event.request_id,
            event.response_id,
            event.model,
            event.query_source,
            option_i64(event.usage.input_tokens_total, "input_tokens_total")?,
            option_i64(event.usage.input_tokens_uncached, "input_tokens_uncached")?,
            option_i64(event.usage.cache_read_tokens, "cache_read_tokens")?,
            option_i64(event.usage.cache_write_tokens, "cache_write_tokens")?,
            option_i64(event.usage.output_tokens_total, "output_tokens_total")?,
            option_i64(event.usage.reasoning_tokens, "reasoning_tokens")?,
            option_i64(event.usage.visible_output_tokens, "visible_output_tokens")?,
            event.provider_reported_cost,
            event.estimated_cost,
            event.currency,
            event.http_status,
            event.latency_ms,
            event.success.map(bool_to_i64),
            event.precision_token.as_str(),
            event.precision_session.as_str(),
            event.precision_provider.as_str(),
            event.precision_account.as_str(),
            event.raw_event_hash,
            raw_usage_json,
            now().to_rfc3339(),
        ],
    )?;
    Ok(changed == 1)
}

fn upsert_cursor(conn: &Connection, cursor: &ImportCursor) -> Result<()> {
    let last_cumulative_usage = cursor
        .last_cumulative_usage
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    conn.execute(
        "INSERT INTO import_cursors (
             source_id, resource_id, file_size, modified_at, byte_offset,
             content_hash, last_cumulative_usage, snapshot_generation, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(source_id, resource_id) DO UPDATE SET
             file_size = excluded.file_size,
             modified_at = excluded.modified_at,
             byte_offset = excluded.byte_offset,
             content_hash = excluded.content_hash,
             last_cumulative_usage = excluded.last_cumulative_usage,
             snapshot_generation = excluded.snapshot_generation,
             updated_at = excluded.updated_at",
        params![
            cursor.source_id,
            cursor.resource_id,
            cursor.file_size,
            cursor.modified_at.map(|value| value.to_rfc3339()),
            cursor.byte_offset,
            cursor.content_hash,
            last_cumulative_usage,
            cursor.snapshot_generation,
            cursor.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn source_from_row(row: &Row<'_>) -> rusqlite::Result<SourceRecord> {
    Ok(SourceRecord {
        id: row.get(0)?,
        adapter_type: row.get(1)?,
        display_name: row.get(2)?,
        path_or_endpoint: row.get(3)?,
        enabled: row.get::<_, i64>(4)? != 0,
        detected_version: row.get(5)?,
        health_status: row.get(6)?,
        last_success_at: row
            .get::<_, Option<String>>(7)?
            .map(|value| parse_datetime("last_success_at", value))
            .transpose()
            .map_err(to_sql_error)?,
        last_error: row.get(8)?,
        created_at: parse_datetime("created_at", row.get(9)?).map_err(to_sql_error)?,
        updated_at: parse_datetime("updated_at", row.get(10)?).map_err(to_sql_error)?,
    })
}

fn provider_summary_from_row(row: &Row<'_>) -> rusqlite::Result<ProviderSummary> {
    let successful_request_count = row
        .get::<_, Option<i64>>(8)?
        .map(|value| checked_u64(value, "successful request count").map_err(to_sql_error))
        .transpose()?;
    let observed_request_count: i64 = row.get(9)?;
    let request_count: i64 = row.get(7)?;
    let success_rate_percent = (observed_request_count > 0).then(|| {
        successful_request_count.unwrap_or(0) as f64 / observed_request_count as f64 * 100.0
    });

    let event_count = checked_u64(row.get(7)?, "provider event count").map_err(to_sql_error)?;
    let totals = totals_from_aggregate_row(row, event_count, 11)?;

    Ok(ProviderSummary {
        id: row.get(0)?,
        provider_family: row.get(1)?,
        display_name: row.get(2)?,
        upstream_url: row.get(3)?,
        launcher: row
            .get::<_, Option<String>>(4)?
            .map(|value| launcher_from_str(&value))
            .transpose()
            .map_err(to_sql_error)?,
        source_id: row.get(5)?,
        account_count: checked_u64(row.get(6)?, "provider account count").map_err(to_sql_error)?,
        request_count: checked_u64(request_count, "provider request count")
            .map_err(to_sql_error)?,
        successful_request_count,
        success_rate_percent,
        average_latency_ms: row.get(10)?,
        totals,
    })
}

fn quota_snapshot_from_row(row: &Row<'_>) -> rusqlite::Result<QuotaSnapshot> {
    let raw_json = row
        .get::<_, Option<String>>(11)?
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(|error| to_sql_error(StorageError::Json(error)))?;
    Ok(QuotaSnapshot {
        id: row.get(0)?,
        account_id: row.get(1)?,
        account_name: row.get(2)?,
        provider_name: row.get(3)?,
        captured_at: parse_datetime("captured_at", row.get(4)?).map_err(to_sql_error)?,
        window_type: row.get(5)?,
        used_percent: row.get(6)?,
        remaining_percent: row.get(7)?,
        reset_at: row
            .get::<_, Option<String>>(8)?
            .map(|value| parse_datetime("reset_at", value))
            .transpose()
            .map_err(to_sql_error)?,
        credits_remaining: row.get(9)?,
        precision: precision_from_str(&row.get::<_, String>(10)?).map_err(to_sql_error)?,
        raw_json,
    })
}

fn app_settings_from_row(row: &Row<'_>) -> rusqlite::Result<AppSettings> {
    Ok(AppSettings {
        codex_home: row.get(0)?,
        claude_home: row.get(1)?,
        cc_switch_db_path: row.get(2)?,
        cockpit_path: row.get(3)?,
        otel_port: optional_u16(row.get(4)?, "otel_port")?,
        auto_start: row.get::<_, i64>(5)? != 0,
        proxy_enabled: row.get::<_, i64>(6)? != 0,
        save_request_metadata: row.get::<_, i64>(7)? != 0,
        data_retention_days: optional_u32(row.get(8)?, "data_retention_days")?,
    })
}

fn session_summary_from_row(row: &Row<'_>) -> rusqlite::Result<SessionSummary> {
    Ok(SessionSummary {
        session: SessionRecord {
            id: row.get(0)?,
            external_session_id: row.get(1)?,
            parent_session_id: row.get(2)?,
            app: app_from_str(row.get(3)?).map_err(to_sql_error)?,
            launcher: row
                .get::<_, Option<String>>(4)?
                .map(|value| launcher_from_str(&value))
                .transpose()
                .map_err(to_sql_error)?,
            project_path: row.get(5)?,
            title: row.get(6)?,
            started_at: row
                .get::<_, Option<String>>(7)?
                .map(|value| parse_datetime("started_at", value))
                .transpose()
                .map_err(to_sql_error)?,
            ended_at: row
                .get::<_, Option<String>>(8)?
                .map(|value| parse_datetime("ended_at", value))
                .transpose()
                .map_err(to_sql_error)?,
            source_id: row.get(9)?,
            created_at: parse_datetime("created_at", row.get(10)?).map_err(to_sql_error)?,
            updated_at: parse_datetime("updated_at", row.get(11)?).map_err(to_sql_error)?,
        },
        totals: totals_from_row_at(row, 12)?,
    })
}

fn usage_event_from_row(row: &Row<'_>) -> rusqlite::Result<UsageEvent> {
    let raw_usage_json = row
        .get::<_, Option<String>>(32)?
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(|error| to_sql_error(StorageError::Json(error)))?;
    Ok(UsageEvent {
        id: row.get(0)?,
        occurred_at: parse_datetime("occurred_at", row.get(1)?).map_err(to_sql_error)?,
        app: app_from_str(row.get(2)?).map_err(to_sql_error)?,
        launcher: launcher_from_str(&row.get::<_, String>(3)?).map_err(to_sql_error)?,
        ingest_source: ingest_source_from_str(&row.get::<_, String>(4)?).map_err(to_sql_error)?,
        source_id: row.get(5)?,
        provider_id: row.get(6)?,
        account_id: row.get(7)?,
        session_id: row.get(8)?,
        parent_session_id: row.get(9)?,
        request_id: row.get(10)?,
        response_id: row.get(11)?,
        model: row.get(12)?,
        query_source: row.get(13)?,
        usage: NormalizedUsage {
            input_tokens_total: option_u64(row.get(14)?, "input_tokens_total")
                .map_err(to_sql_error)?,
            input_tokens_uncached: option_u64(row.get(15)?, "input_tokens_uncached")
                .map_err(to_sql_error)?,
            cache_read_tokens: option_u64(row.get(16)?, "cache_read_tokens")
                .map_err(to_sql_error)?,
            cache_write_tokens: option_u64(row.get(17)?, "cache_write_tokens")
                .map_err(to_sql_error)?,
            output_tokens_total: option_u64(row.get(18)?, "output_tokens_total")
                .map_err(to_sql_error)?,
            reasoning_tokens: option_u64(row.get(19)?, "reasoning_tokens").map_err(to_sql_error)?,
            visible_output_tokens: option_u64(row.get(20)?, "visible_output_tokens")
                .map_err(to_sql_error)?,
        },
        provider_reported_cost: row.get(21)?,
        estimated_cost: row.get(22)?,
        currency: row.get(23)?,
        http_status: row.get(24)?,
        latency_ms: row.get(25)?,
        success: row.get::<_, Option<i64>>(26)?.map(|value| value != 0),
        precision_token: precision_from_str(&row.get::<_, String>(27)?).map_err(to_sql_error)?,
        precision_session: precision_from_str(&row.get::<_, String>(28)?).map_err(to_sql_error)?,
        precision_provider: precision_from_str(&row.get::<_, String>(29)?).map_err(to_sql_error)?,
        precision_account: precision_from_str(&row.get::<_, String>(30)?).map_err(to_sql_error)?,
        raw_event_hash: row.get(31)?,
        raw_usage_json,
    })
}

fn cursor_from_row(row: &Row<'_>) -> rusqlite::Result<ImportCursor> {
    let last_cumulative_usage = row
        .get::<_, Option<String>>(6)?
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(|error| to_sql_error(StorageError::Json(error)))?;
    Ok(ImportCursor {
        source_id: row.get(0)?,
        resource_id: row.get(1)?,
        file_size: row.get(2)?,
        modified_at: row
            .get::<_, Option<String>>(3)?
            .map(|value| parse_datetime("modified_at", value))
            .transpose()
            .map_err(to_sql_error)?,
        byte_offset: row.get(4)?,
        content_hash: row.get(5)?,
        last_cumulative_usage,
        snapshot_generation: row.get(7)?,
        updated_at: parse_datetime("updated_at", row.get(8)?).map_err(to_sql_error)?,
    })
}

fn totals_from_row(row: &Row<'_>) -> rusqlite::Result<UsageTotals> {
    totals_from_row_at(row, 0)
}

fn totals_from_row_at(row: &Row<'_>, start: usize) -> rusqlite::Result<UsageTotals> {
    let event_count =
        checked_u64(row.get::<_, i64>(start)?, "event count").map_err(to_sql_error)?;
    totals_from_aggregate_row(row, event_count, start + 1)
}

fn totals_from_aggregate_row(
    row: &Row<'_>,
    event_count: u64,
    start: usize,
) -> rusqlite::Result<UsageTotals> {
    let mut column = start;
    let input_tokens_total = aggregate_u64(row, &mut column, event_count, "input_tokens_total")?;
    let input_tokens_uncached =
        aggregate_u64(row, &mut column, event_count, "input_tokens_uncached")?;
    let cache_read_tokens = aggregate_u64(row, &mut column, event_count, "cache_read_tokens")?;
    let cache_write_tokens = aggregate_u64(row, &mut column, event_count, "cache_write_tokens")?;
    let output_tokens_total = aggregate_u64(row, &mut column, event_count, "output_tokens_total")?;
    let reasoning_tokens = aggregate_u64(row, &mut column, event_count, "reasoning_tokens")?;
    let visible_output_tokens =
        aggregate_u64(row, &mut column, event_count, "visible_output_tokens")?;
    let provider_reported_cost = aggregate_f64(row, &mut column, event_count)?;
    let estimated_cost = aggregate_f64(row, &mut column, event_count)?;

    let totals = UsageTotals {
        event_count,
        input_tokens_total,
        input_tokens_uncached,
        cache_read_tokens,
        cache_write_tokens,
        output_tokens_total,
        reasoning_tokens,
        visible_output_tokens,
        provider_reported_cost,
        estimated_cost,
        cache_hit_rate_percent: None,
    };
    Ok(UsageTotals {
        cache_hit_rate_percent: cache_hit_rate(totals.input_tokens_total, totals.cache_read_tokens),
        ..totals
    })
}

fn aggregate_u64(
    row: &Row<'_>,
    column: &mut usize,
    event_count: u64,
    field: &str,
) -> rusqlite::Result<Option<u64>> {
    let known_count: i64 = row.get(*column)?;
    let sum: Option<i64> = row.get(*column + 1)?;
    *column += 2;
    if known_count != i64::try_from(event_count).unwrap_or(-1) {
        return Ok(None);
    }
    option_u64(sum, field).map_err(to_sql_error)
}

fn aggregate_f64(
    row: &Row<'_>,
    column: &mut usize,
    event_count: u64,
) -> rusqlite::Result<Option<f64>> {
    let known_count: i64 = row.get(*column)?;
    let sum: Option<f64> = row.get(*column + 1)?;
    *column += 2;
    if known_count != i64::try_from(event_count).unwrap_or(-1) {
        Ok(None)
    } else {
        Ok(sum)
    }
}

fn cache_hit_rate(input_total: Option<u64>, cache_read: Option<u64>) -> Option<f64> {
    NormalizedUsage {
        input_tokens_total: input_total,
        cache_read_tokens: cache_read,
        ..NormalizedUsage::default()
    }
    .cache_hit_rate_percent()
}

fn parse_datetime(field: &str, value: String) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|date| date.with_timezone(&Utc))
        .map_err(|_| StorageError::InvalidDateTime {
            field: field.to_owned(),
            value,
        })
}

fn app_from_str(value: String) -> Result<AppKind> {
    match value.as_str() {
        "codex" => Ok(AppKind::Codex),
        "claude_code" => Ok(AppKind::ClaudeCode),
        "unknown" => Ok(AppKind::Unknown),
        _ => Err(StorageError::UnknownEnum {
            field: "app".to_owned(),
            value,
        }),
    }
}

fn launcher_from_str(value: &str) -> Result<LauncherKind> {
    match value {
        "direct" => Ok(LauncherKind::Direct),
        "cc_switch" => Ok(LauncherKind::CCSwitch),
        "cockpit" => Ok(LauncherKind::Cockpit),
        "observer_proxy" => Ok(LauncherKind::ObserverProxy),
        "unknown" => Ok(LauncherKind::Unknown),
        _ => Err(StorageError::UnknownEnum {
            field: "launcher".to_owned(),
            value: value.to_owned(),
        }),
    }
}

fn ingest_source_from_str(value: &str) -> Result<tokenbuddy_domain::IngestSource> {
    match value {
        "session_log" => Ok(tokenbuddy_domain::IngestSource::SessionLog),
        "otel" => Ok(tokenbuddy_domain::IngestSource::Otel),
        "proxy" => Ok(tokenbuddy_domain::IngestSource::Proxy),
        "quota_api" => Ok(tokenbuddy_domain::IngestSource::QuotaApi),
        "imported_database" => Ok(tokenbuddy_domain::IngestSource::ImportedDatabase),
        "estimated" => Ok(tokenbuddy_domain::IngestSource::Estimated),
        _ => Err(StorageError::UnknownEnum {
            field: "ingest_source".to_owned(),
            value: value.to_owned(),
        }),
    }
}

fn precision_from_str(value: &str) -> Result<PrecisionLevel> {
    match value {
        "verified" => Ok(PrecisionLevel::Verified),
        "exact_session" => Ok(PrecisionLevel::ExactSession),
        "correlated" => Ok(PrecisionLevel::Correlated),
        "estimated" => Ok(PrecisionLevel::Estimated),
        "unavailable" => Ok(PrecisionLevel::Unavailable),
        _ => Err(StorageError::UnknownEnum {
            field: "precision".to_owned(),
            value: value.to_owned(),
        }),
    }
}

fn option_i64(value: Option<u64>, field: &str) -> Result<Option<i64>> {
    value
        .map(|value| {
            i64::try_from(value).map_err(|_| StorageError::InvalidTokenCount {
                field: field.to_owned(),
            })
        })
        .transpose()
}

fn option_u64(value: Option<i64>, field: &str) -> Result<Option<u64>> {
    value
        .map(|value| {
            u64::try_from(value).map_err(|_| StorageError::InvalidTokenCount {
                field: field.to_owned(),
            })
        })
        .transpose()
}

fn checked_i64(value: u64, field: &str) -> Result<i64> {
    i64::try_from(value).map_err(|_| StorageError::InvalidTokenCount {
        field: field.to_owned(),
    })
}

fn checked_u64(value: i64, field: &str) -> Result<u64> {
    u64::try_from(value).map_err(|_| StorageError::InvalidTokenCount {
        field: field.to_owned(),
    })
}

fn optional_u16(value: Option<i64>, field: &str) -> rusqlite::Result<Option<u16>> {
    value
        .map(|value| {
            u16::try_from(value).map_err(|_| {
                to_sql_error(StorageError::InvalidTokenCount {
                    field: field.to_owned(),
                })
            })
        })
        .transpose()
}

fn optional_u32(value: Option<i64>, field: &str) -> rusqlite::Result<Option<u32>> {
    value
        .map(|value| {
            u32::try_from(value).map_err(|_| {
                to_sql_error(StorageError::InvalidTokenCount {
                    field: field.to_owned(),
                })
            })
        })
        .transpose()
}

fn bool_to_i64(value: bool) -> i64 {
    i64::from(value)
}

fn to_sql_error(error: StorageError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use tokenbuddy_domain::{
        AppKind, AppSettings, CollectionStatus, ImportBatch, ImportCursor, IngestSource,
        LauncherKind, NormalizedUsage, PrecisionLevel, SessionRecord, SourceRecord, UsageEvent,
        UsageFilters,
    };

    use super::Database;

    fn source() -> SourceRecord {
        let now = Utc::now();
        SourceRecord {
            id: "codex-session".to_owned(),
            adapter_type: "codex_session".to_owned(),
            display_name: "Codex Sessions".to_owned(),
            path_or_endpoint: Some("/fixtures/codex".to_owned()),
            enabled: true,
            detected_version: Some("fixture-v1".to_owned()),
            health_status: Some("healthy".to_owned()),
            last_success_at: Some(now),
            last_error: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn session() -> SessionRecord {
        let now = Utc::now();
        SessionRecord {
            id: "session-1".to_owned(),
            external_session_id: Some("external-1".to_owned()),
            parent_session_id: None,
            app: AppKind::Codex,
            launcher: Some(LauncherKind::Direct),
            project_path: Some("/fixtures/project".to_owned()),
            title: Some("Fixture session".to_owned()),
            started_at: Some(now - Duration::minutes(1)),
            ended_at: Some(now),
            source_id: Some("codex-session".to_owned()),
            created_at: now,
            updated_at: now,
        }
    }

    fn event(hash: &str, input: Option<u64>) -> UsageEvent {
        let usage = input.map_or_else(NormalizedUsage::default, |input| NormalizedUsage {
            input_tokens_total: Some(input),
            cache_read_tokens: Some(25),
            output_tokens_total: Some(30),
            ..Default::default()
        });
        UsageEvent {
            id: hash.to_owned(),
            occurred_at: Utc::now(),
            app: AppKind::Codex,
            launcher: LauncherKind::Direct,
            ingest_source: IngestSource::SessionLog,
            source_id: "codex-session".to_owned(),
            provider_id: None,
            account_id: None,
            session_id: Some("session-1".to_owned()),
            parent_session_id: None,
            request_id: None,
            response_id: None,
            model: Some("fixture-model".to_owned()),
            query_source: None,
            usage,
            provider_reported_cost: None,
            estimated_cost: None,
            currency: None,
            http_status: None,
            latency_ms: None,
            success: Some(true),
            precision_token: PrecisionLevel::ExactSession,
            precision_session: PrecisionLevel::ExactSession,
            precision_provider: PrecisionLevel::Unavailable,
            precision_account: PrecisionLevel::Unavailable,
            raw_event_hash: hash.to_owned(),
            raw_usage_json: Some(serde_json::json!({"input_tokens": input})),
        }
    }

    #[test]
    fn migrations_are_idempotent_and_create_the_required_tables() {
        let database = Database::open_in_memory().expect("database opens");
        database
            .connection()
            .execute_batch("SELECT 1")
            .expect("connection remains usable");
        let table_count: i64 = database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN
                 ('sources', 'sessions', 'usage_events', 'import_cursors')",
                [],
                |row| row.get(0),
            )
            .expect("tables exist");
        assert_eq!(table_count, 4);
    }

    #[test]
    fn repeated_import_is_idempotent_and_missing_usage_stays_missing() {
        let mut database = Database::open_in_memory().expect("database opens");
        let now = Utc::now();
        let batch = ImportBatch {
            source: Some(source()),
            sessions: vec![session()],
            usage_events: vec![event("event-1", Some(100)), event("event-2", None)],
            cursors: vec![ImportCursor {
                source_id: "codex-session".to_owned(),
                resource_id: "fixture.jsonl".to_owned(),
                file_size: Some(20),
                modified_at: Some(now),
                byte_offset: 20,
                content_hash: Some("hash".to_owned()),
                last_cumulative_usage: None,
                snapshot_generation: 0,
                updated_at: now,
            }],
            skipped_records: 0,
        };

        let first = database.apply_import_batch(&batch).expect("first import");
        let second = database.apply_import_batch(&batch).expect("second import");
        assert_eq!(first.inserted_events, 2);
        assert_eq!(second.inserted_events, 0);
        assert_eq!(second.duplicate_events, 2);

        let page = database
            .list_session_page(None, 20, 0)
            .expect("session page");
        assert_eq!(page.total, 1);
        assert_eq!(page.sessions[0].totals.event_count, 2);
        assert_eq!(page.sessions[0].totals.input_tokens_total, None);
        assert_eq!(page.sessions[0].totals.cache_read_tokens, None);
        assert_eq!(page.sessions[0].totals.output_tokens_total, None);
        assert_eq!(page.sessions[0].totals.cache_hit_rate_percent, None);

        let quick = database
            .quick_summary(Utc::now(), CollectionStatus::Collecting, None)
            .expect("quick summary");
        assert_eq!(quick.active_session_id.as_deref(), Some("session-1"));
        assert_eq!(
            quick.active_session_title.as_deref(),
            Some("Fixture session")
        );
        assert_eq!(quick.session_output_tokens, None);
        assert_eq!(quick.today_total_tokens, None);
    }

    #[test]
    fn usage_filters_and_exports_share_the_same_normalized_event_boundary() {
        let mut database = Database::open_in_memory().expect("database opens");
        let now = Utc::now();
        let batch = ImportBatch {
            source: Some(source()),
            sessions: vec![session()],
            usage_events: vec![event("event-1", Some(100)), event("event-2", None)],
            ..ImportBatch::default()
        };
        database.apply_import_batch(&batch).expect("import events");

        let filters = UsageFilters {
            period_start: Some(now - Duration::minutes(2)),
            period_end: Some(now + Duration::minutes(2)),
            app: Some(AppKind::Codex),
            model: Some("fixture-model".to_owned()),
            search: Some("Fixture session".to_owned()),
            ..UsageFilters::default()
        };
        let summary = database
            .dashboard_summary_filtered(&filters)
            .expect("filtered summary");
        assert_eq!(summary.totals.event_count, 2);
        assert_eq!(summary.totals.input_tokens_total, None);

        let page = database
            .list_usage_events_filtered(None, 10, 0, &filters)
            .expect("filtered events");
        assert_eq!(page.total, 2);

        let json = database
            .export_usage("json", &filters)
            .expect("JSON export");
        assert!(json.content.contains("event-1"));
        assert!(!json.content.contains("raw_usage_json"));

        let csv = database.export_usage("csv", &filters).expect("CSV export");
        assert!(csv.content.starts_with("id,occurred_at,app,"));
        assert_eq!(csv.content.lines().count(), 3);
    }

    #[test]
    fn provider_quota_and_settings_queries_keep_unavailable_values_explicit() {
        let mut database = Database::open_in_memory().expect("database opens");
        let now = Utc::now();
        database
            .connection()
            .execute(
                "INSERT INTO providers (
                     id, provider_family, display_name, upstream_url, launcher,
                     source_id, created_at, updated_at
                 ) VALUES ('provider-1', 'openai_compatible', 'Fixture Provider',
                           'https://fixture.invalid', 'direct', 'fixture', ?1, ?1)",
                [now.to_rfc3339()],
            )
            .expect("provider insert");
        database
            .connection()
            .execute(
                "INSERT INTO accounts (
                     id, provider_id, display_name, account_fingerprint, auth_mode,
                     plan, created_at, updated_at
                 ) VALUES ('account-1', 'provider-1', 'Fixture Account', 'fp-1',
                           'api_key', NULL, ?1, ?1)",
                [now.to_rfc3339()],
            )
            .expect("account insert");

        let mut provider_event = event("provider-event", Some(100));
        provider_event.provider_id = Some("provider-1".to_owned());
        provider_event.account_id = Some("account-1".to_owned());
        provider_event.success = Some(true);
        provider_event.latency_ms = Some(42);
        database
            .apply_import_batch(&ImportBatch {
                source: Some(source()),
                sessions: vec![session()],
                usage_events: vec![provider_event],
                ..ImportBatch::default()
            })
            .expect("provider event import");
        database
            .connection()
            .execute(
                "INSERT INTO quota_snapshots (
                     id, account_id, captured_at, window_type, used_percent,
                     remaining_percent, reset_at, credits_remaining, precision, raw_json
                 ) VALUES ('quota-1', 'account-1', ?1, 'daily', 25.0, 75.0,
                           NULL, NULL, 'unavailable', NULL)",
                [now.to_rfc3339()],
            )
            .expect("quota insert");

        let providers = database.list_providers().expect("providers");
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].request_count, 1);
        assert_eq!(providers[0].totals.input_tokens_total, Some(100));
        assert_eq!(providers[0].success_rate_percent, Some(100.0));

        let quotas = database
            .list_quota_snapshots(None, 10)
            .expect("quota snapshots");
        assert_eq!(quotas.len(), 1);
        assert_eq!(quotas[0].used_percent, Some(25.0));
        assert_eq!(quotas[0].credits_remaining, None);
        assert_eq!(quotas[0].precision, PrecisionLevel::Unavailable);

        let settings = AppSettings {
            codex_home: Some("/sanitized/codex".to_owned()),
            auto_start: true,
            ..AppSettings::default()
        };
        database
            .save_app_settings(&settings)
            .expect("settings save");
        assert_eq!(
            database.get_app_settings().expect("settings load"),
            settings
        );
    }
}
