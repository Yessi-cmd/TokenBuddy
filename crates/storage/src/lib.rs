//! Persistence: SQLite schema, idempotent import, and every aggregation query.
//!
//! This is the only crate that talks to the database. Adapters produce domain
//! records, the Core hands them here as one batch per pass, and every panel
//! reads through the query methods below — so the rules that keep the numbers
//! honest (missing stays missing, a duplicate import changes nothing, a
//! launcher outranks a guess) are enforced in one place.
#![warn(missing_docs)]

mod migrations;
mod pricing;

use std::{collections::HashMap, fmt::Write as FmtWrite, path::Path, time::SystemTime};

use chrono::{DateTime, Local, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use thiserror::Error;
use tokenbuddy_domain::{
    AccountActivityWindow, AccountRecord, AccountSummary, AppKind, AppSettings, CollectionStatus,
    DashboardSummary, ExportResult, ImportBatch, ImportCursor, LauncherKind, ModelUsage,
    NormalizedUsage, PrecisionLevel, ProviderRecord, ProviderSummary, QuickSummary, QuotaSnapshot,
    QuotaSummary, SessionDetail, SessionPage, SessionProviderAttribution, SessionRecord,
    SessionSummary, SourceRecord, UsageEvent, UsageEventPage, UsageFilters, UsageTotals,
    correlation_key,
};

/// Result of any storage operation.
pub type Result<T> = std::result::Result<T, StorageError>;

/// Anything that can go wrong while reading or writing the database.
///
/// The stored-value variants name the offending field: a database written by a
/// newer version, or corrupted by hand, should say what it tripped over rather
/// than fail as an opaque parse error.
#[derive(Debug, Error)]
pub enum StorageError {
    /// SQLite itself refused the operation.
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A JSON column could not be encoded or decoded.
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    /// The database file or its directory could not be reached.
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    /// A stored timestamp is not valid RFC 3339.
    #[error("invalid datetime in {field}: {value}")]
    InvalidDateTime {
        /// Column that held the value.
        field: String,
        /// The value as stored.
        value: String,
    },
    /// A stored token count does not fit the type it is read into — negative,
    /// or beyond the representable range.
    #[error("invalid token count in {field}")]
    InvalidTokenCount {
        /// Column that held the value.
        field: String,
    },
    /// A stored enum name is not one this version knows.
    #[error("unknown stored enum value for {field}: {value}")]
    UnknownEnum {
        /// Column that held the value.
        field: String,
        /// The value as stored.
        value: String,
    },
    /// Migrations did not reach the expected version, usually because the file
    /// was written by a newer build. Refused rather than used, so a downgrade
    /// cannot silently misread a newer schema.
    #[error("database migration stopped at unsupported version {0}")]
    MigrationVersion(i64),
    /// An export format that is not supported. Never falls back to another.
    #[error("unsupported export format: {0}")]
    UnsupportedExportFormat(String),
    /// The per-install fingerprint salt could not be stored.
    #[error("failed to persist the local fingerprint salt")]
    MissingLocalSalt,
}

/// What applying one batch changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportStats {
    /// Events newly stored.
    pub inserted_events: u64,
    /// Events already present, recognised by hash.
    pub duplicate_events: u64,
    /// Lower-priority observations replaced by a stronger correlated source.
    pub reconciled_events: u64,
    /// Sessions created or refreshed.
    pub upserted_sessions: u64,
    /// Cursors advanced.
    pub updated_cursors: u64,
    /// Accounts created or refreshed.
    pub upserted_accounts: u64,
    /// Quota readings newly stored.
    pub inserted_quota_snapshots: u64,
    /// Already-stored events that a newly imported activity window attributed
    /// to a real account.
    pub attributed_account_events: u64,
}

/// What enforcing the retention window removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetentionOutcome {
    /// Usage events deleted.
    pub deleted_events: u64,
    /// Sessions deleted once they had no events left.
    pub deleted_sessions: u64,
    /// Quota readings deleted.
    pub deleted_quota: u64,
}

const SESSION_PAGE_SELECT: &str = "\
    SELECT s.id, s.external_session_id, s.parent_session_id, s.app, s.launcher,
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
        AND (?1 IS NULL OR u.occurred_at >= ?1)
        AND (?2 IS NULL OR u.occurred_at < ?2)
        AND (?3 IS NULL OR u.app = ?3)
        AND (?4 IS NULL OR u.provider_id = ?4)
        AND (?5 IS NULL OR u.account_id = ?5)
        AND (?6 IS NULL OR u.model LIKE ?6)
        AND (?7 IS NULL OR u.precision_token = ?7)
    WHERE (?8 IS NULL OR s.project_path LIKE ?8)
      AND (?9 IS NULL OR s.title LIKE ?9 OR s.project_path LIKE ?9
           OR s.external_session_id LIKE ?9 OR u.model LIKE ?9
           OR u.request_id LIKE ?9)
    GROUP BY s.id
    HAVING (?10 = 0 OR COUNT(u.id) > 0)
    ORDER BY COALESCE(s.ended_at, s.updated_at, s.started_at, s.created_at) DESC
    LIMIT ?11 OFFSET ?12";

/// An open TokenBuddy database, migrated to the current schema.
pub struct Database {
    connection: Connection,
}

impl Database {
    /// Open (creating if needed) and migrate the database at `path`.
    ///
    /// Missing parent directories are created, so a first run on a clean
    /// machine works without setup.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut connection = Connection::open(path)?;
        configure_connection(&connection)?;
        migrations::run(&mut connection)?;
        let mut database = Self { connection };
        database.refresh_estimated_costs()?;
        Ok(database)
    }

    /// A migrated in-memory database, for tests.
    pub fn open_in_memory() -> Result<Self> {
        let mut connection = Connection::open_in_memory()?;
        configure_connection(&connection)?;
        migrations::run(&mut connection)?;
        let mut database = Self { connection };
        database.refresh_estimated_costs()?;
        Ok(database)
    }

    /// The underlying connection, for tests that need to assert on raw rows.
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    fn refresh_estimated_costs(&mut self) -> Result<()> {
        refresh_estimated_costs_in_connection(&mut self.connection)
    }

    /// Apply one adapter batch in a single transaction.
    ///
    /// Ordering matters and is deliberate: sources, providers, and accounts
    /// first so events can reference real identities; attributions and activity
    /// windows before events so rows landing in this batch resolve immediately
    /// and earlier rows get backfilled; cursors last so a failure re-reads the
    /// same input rather than skipping it.
    pub fn apply_import_batch(&mut self, batch: &ImportBatch) -> Result<ImportStats> {
        // Read the setting once per transaction. The default is false, so a
        // caller that opens a fresh database cannot accidentally persist raw
        // usage metadata merely by constructing a UsageEvent with it attached.
        let save_request_metadata = self.get_app_settings()?.save_request_metadata;
        let transaction = self.connection.transaction()?;
        let mut stats = ImportStats {
            upserted_sessions: batch.sessions.len() as u64,
            updated_cursors: batch.cursors.len() as u64,
            upserted_accounts: batch.accounts.len() as u64,
            ..ImportStats::default()
        };

        if let Some(source) = &batch.source {
            upsert_source(&transaction, source)?;
        }

        for provider in &batch.providers {
            upsert_provider_record(&transaction, provider)?;
        }

        // Accounts land before events and quota rows so both can reference a
        // real identity instead of the placeholder derived from a model name.
        for account in &batch.accounts {
            upsert_account_record(&transaction, account)?;
        }

        // Windows land before events so rows arriving in this batch resolve
        // immediately, then backfill anything imported before the launcher was
        // scanned — same ordering rule as provider attributions above.
        for window in &batch.account_windows {
            upsert_account_window(&transaction, window)?;
        }
        stats.attributed_account_events +=
            backfill_account_windows(&transaction, &batch.account_windows)?;

        // Apply provider attributions before inserting events so rows landing in
        // this same batch already resolve to the real provider, and backfill any
        // events imported earlier under a guessed one.
        for attribution in &batch.attributions {
            upsert_attribution(&transaction, attribution)?;
            apply_attribution(&transaction, attribution)?;
        }

        for session in &batch.sessions {
            upsert_session(&transaction, session)?;
        }

        for event in &batch.usage_events {
            // A launcher-reported attribution is ground truth and wins over any
            // guess. Only fall back to deriving a provider from the model name
            // when nothing authoritative is known for this session.
            let attributed = event
                .session_id
                .as_deref()
                .map(|session_id| lookup_attribution(&transaction, session_id))
                .transpose()?
                .flatten();
            let derived = if attributed.is_some() {
                None
            } else {
                derive_provider(event)
            };
            if let Some(derived) = &derived {
                ensure_provider(&transaction, derived, event)?;
                ensure_account(&transaction, derived)?;
            }
            // A launcher that routed this request at this instant outranks the
            // placeholder account derived from the model name, but never a
            // launcher-reported attribution or an account the adapter resolved.
            let windowed_account = if attributed.is_none() && event.account_id.is_none() {
                account_at(&transaction, event.app, event.occurred_at)?
            } else {
                None
            };
            let inserted = insert_usage_event(
                &transaction,
                event,
                derived.as_ref(),
                attributed.as_ref(),
                windowed_account.as_deref(),
                save_request_metadata,
            )?;
            match inserted {
                InsertOutcome::Inserted => stats.inserted_events += 1,
                InsertOutcome::Duplicate => stats.duplicate_events += 1,
                InsertOutcome::Reconciled => stats.reconciled_events += 1,
            }
        }

        // Provider attributions and richer copies of a streamed response can
        // both arrive after the first observation. Re-evaluate estimates in
        // the same transaction so the UI never exposes a price for the wrong
        // relay and newly recognized provider routes become available at once.
        refresh_estimated_costs_on_connection(&transaction)?;

        for snapshot in &batch.quota_snapshots {
            if insert_quota_snapshot(&transaction, snapshot)? {
                stats.inserted_quota_snapshots += 1;
            }
        }

        for cursor in &batch.cursors {
            upsert_cursor(&transaction, cursor)?;
        }

        transaction.commit()?;
        Ok(stats)
    }

    /// Delete usage older than the configured retention window and prune the
    /// sessions and quota snapshots left behind. Returns what was removed. A
    /// `None` or zero window keeps everything (retention disabled).
    ///
    /// This is the only path in TokenBuddy that deletes stored usage: without
    /// it, deleting a source file, fixing a mis-imported home, or a single bad
    /// timestamp would pollute every statistic permanently.
    pub fn enforce_retention(
        &mut self,
        retention_days: Option<u32>,
        now: DateTime<Utc>,
    ) -> Result<RetentionOutcome> {
        let Some(days) = retention_days.filter(|days| *days > 0) else {
            return Ok(RetentionOutcome::default());
        };
        let cutoff = (now - chrono::Duration::days(i64::from(days))).to_rfc3339();
        let transaction = self.connection.transaction()?;
        let deleted_events = transaction.execute(
            "DELETE FROM usage_events WHERE occurred_at < ?1",
            params![cutoff],
        )? as u64;
        let deleted_quota = transaction.execute(
            "DELETE FROM quota_snapshots WHERE captured_at < ?1",
            params![cutoff],
        )? as u64;
        // Prune sessions that have no remaining events and whose last activity is
        // older than the window. A still-active or newly-created empty session
        // (recent updated_at) is kept.
        let deleted_sessions = transaction.execute(
            "DELETE FROM sessions
              WHERE updated_at < ?1
                AND id NOT IN (
                    SELECT session_id FROM usage_events WHERE session_id IS NOT NULL
                )",
            params![cutoff],
        )? as u64;
        transaction.commit()?;
        Ok(RetentionOutcome {
            deleted_events,
            deleted_sessions,
            deleted_quota,
        })
    }

    /// Every configured source with its health.
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

    /// Providers with their request, latency, and token aggregates.
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

    /// Accounts with their provider name and newest quota window. Placeholder
    /// accounts derived from a session log are included so the UI can show what
    /// is known and what is still `Unavailable`.
    pub fn list_accounts(&self) -> Result<Vec<AccountSummary>> {
        let mut statement = self.connection.prepare(
            "SELECT a.id, a.provider_id, a.display_name, a.account_fingerprint,
                    a.auth_mode, a.plan, p.display_name
             FROM accounts a
             LEFT JOIN providers p ON p.id = a.provider_id
             ORDER BY a.auth_mode = 'session_log', a.updated_at DESC, a.id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    AccountRecord {
                        id: row.get(0)?,
                        provider_id: row.get(1)?,
                        display_name: row.get(2)?,
                        account_fingerprint: row.get(3)?,
                        auth_mode: row.get(4)?,
                        plan: row.get(5)?,
                    },
                    row.get::<_, Option<String>>(6)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(account, provider_name)| {
                let latest_quota = self.latest_quota_summary(&account.id)?;
                Ok(AccountSummary {
                    account,
                    provider_name,
                    latest_quota,
                })
            })
            .collect()
    }

    /// Quota readings, newest first, optionally for one account.
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
             ORDER BY q.captured_at DESC,
                      CASE
                          WHEN q.window_type LIKE 'primary%' THEN 0
                          WHEN q.window_type LIKE 'secondary%' THEN 1
                          WHEN q.window_type = 'credits' THEN 3
                          ELSE 2
                      END,
                      q.id DESC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(
            params![account_id, checked_i64(limit, "quota limit")?],
            quota_snapshot_from_row,
        )?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// The per-install salt for account and credential fingerprints (spec
    /// §20.2), generated once from SQLite's CSPRNG. It is intentionally not part
    /// of `AppSettings`, so it never reaches the UI, the loopback API, or an
    /// export — a fingerprint without the salt cannot be reversed by lookup.
    pub fn local_salt(&self) -> Result<String> {
        if let Some(salt) = self.stored_local_salt()? {
            return Ok(salt);
        }
        let generated: String =
            self.connection
                .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        self.connection.execute(
            "INSERT INTO app_settings (id, local_salt) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET
                 local_salt = COALESCE(app_settings.local_salt, excluded.local_salt)",
            params![generated],
        )?;
        // Re-read instead of returning `generated`: an earlier writer may have
        // won the COALESCE, and the stored salt is the one fingerprints used.
        self.stored_local_salt()?
            .ok_or(StorageError::MissingLocalSalt)
    }

    fn stored_local_salt(&self) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT local_salt FROM app_settings WHERE id = 1",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// The stored settings, or defaults if none were ever saved.
    pub fn get_app_settings(&self) -> Result<AppSettings> {
        let settings = self
            .connection
            .query_row(
                "SELECT codex_home, claude_home, cc_switch_db_path, cockpit_path,
                        otel_port, auto_start, proxy_enabled, save_request_metadata,
                        data_retention_days, opencode_db_path, dsh_home
                 FROM app_settings WHERE id = 1",
                [],
                app_settings_from_row,
            )
            .optional()?;
        Ok(settings.unwrap_or_default())
    }

    /// Replace the settings row. The fingerprint salt lives in the same row and
    /// is deliberately not touched here.
    pub fn save_app_settings(&self, settings: &AppSettings) -> Result<()> {
        self.connection.execute(
            "INSERT INTO app_settings (
                 id, codex_home, claude_home, cc_switch_db_path, cockpit_path,
                 otel_port, auto_start, proxy_enabled, save_request_metadata,
                 data_retention_days, opencode_db_path, dsh_home
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                 codex_home = excluded.codex_home,
                 claude_home = excluded.claude_home,
                 cc_switch_db_path = excluded.cc_switch_db_path,
                 cockpit_path = excluded.cockpit_path,
                 otel_port = excluded.otel_port,
                 auto_start = excluded.auto_start,
                 proxy_enabled = excluded.proxy_enabled,
                 save_request_metadata = excluded.save_request_metadata,
                 data_retention_days = excluded.data_retention_days,
                 opencode_db_path = excluded.opencode_db_path,
                 dsh_home = excluded.dsh_home",
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
                settings.opencode_db_path,
                settings.dsh_home,
            ],
        )?;
        // Request metadata is an explicit opt-in. Clearing the setting must
        // also clear metadata already persisted by an earlier opt-in; otherwise
        // turning the switch off would only affect future imports and leave the
        // privacy-sensitive history behind. Token counts and attribution remain
        // intact because they are normalized facts, not request contents.
        if !settings.save_request_metadata {
            self.connection.execute(
                "UPDATE usage_events SET raw_usage_json = NULL
                  WHERE raw_usage_json IS NOT NULL",
                [],
            )?;
        }
        Ok(())
    }

    /// The cursor for one resource of one source, if it has been read before.
    pub fn get_import_cursor(
        &self,
        source_id: &str,
        resource_id: &str,
    ) -> Result<Option<ImportCursor>> {
        self.connection
            .query_row(
                "SELECT source_id, resource_id, file_size, modified_at, byte_offset,
                        content_hash, last_cumulative_usage, snapshot_generation,
                        last_session_id, last_model, updated_at
                 FROM import_cursors WHERE source_id = ?1 AND resource_id = ?2",
                params![source_id, resource_id],
                cursor_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Every cursor of one source, keyed by resource id, as adapters need them
    /// at the start of a pass.
    pub fn list_import_cursors(&self, source_id: &str) -> Result<HashMap<String, ImportCursor>> {
        let mut statement = self.connection.prepare(
            "SELECT source_id, resource_id, file_size, modified_at, byte_offset,
                    content_hash, last_cumulative_usage, snapshot_generation,
                    last_session_id, last_model, updated_at
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

    /// One page of sessions with totals over the filtered events.
    ///
    /// Sessions with no matching event are kept unless the filters are about
    /// events themselves — an empty session is a fact, not a gap.
    pub fn list_session_page(
        &self,
        filters: &UsageFilters,
        limit: u64,
        offset: u64,
    ) -> Result<SessionPage> {
        let filter_params = UsageFilterParams::from_filters(filters);
        // Event-level filters are applied in the JOIN so a session's totals count
        // only matching events (consistent with the dashboard). When any such
        // filter is active, only sessions with a matching event are listed.
        let require_match = i64::from(
            filter_params.period_start.is_some()
                || filter_params.period_end.is_some()
                || filter_params.app.is_some()
                || filter_params.provider_id.is_some()
                || filter_params.account_id.is_some()
                || filter_params.model.is_some()
                || filter_params.precision.is_some(),
        );
        let limit = checked_i64(limit, "limit")?;
        let offset = checked_i64(offset, "offset")?;
        let mut statement = self.connection.prepare(SESSION_PAGE_SELECT)?;
        let rows = statement.query_map(
            params![
                filter_params.period_start,
                filter_params.period_end,
                filter_params.app,
                filter_params.provider_id,
                filter_params.account_id,
                filter_params.model,
                filter_params.precision,
                filter_params.project_path,
                filter_params.search,
                require_match,
                limit,
                offset,
            ],
            session_summary_from_row,
        )?;
        let sessions = rows.collect::<std::result::Result<Vec<_>, _>>()?;

        let total: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM (
                 SELECT s.id
                 FROM sessions s
                 LEFT JOIN usage_events u ON u.session_id = s.id
                     AND (?1 IS NULL OR u.occurred_at >= ?1)
                     AND (?2 IS NULL OR u.occurred_at < ?2)
                     AND (?3 IS NULL OR u.app = ?3)
                     AND (?4 IS NULL OR u.provider_id = ?4)
                     AND (?5 IS NULL OR u.account_id = ?5)
                     AND (?6 IS NULL OR u.model LIKE ?6)
                     AND (?7 IS NULL OR u.precision_token = ?7)
                 WHERE (?8 IS NULL OR s.project_path LIKE ?8)
                   AND (?9 IS NULL OR s.title LIKE ?9 OR s.project_path LIKE ?9
                        OR s.external_session_id LIKE ?9 OR u.model LIKE ?9
                        OR u.request_id LIKE ?9)
                 GROUP BY s.id
                 HAVING (?10 = 0 OR COUNT(u.id) > 0)
             )",
            params![
                filter_params.period_start,
                filter_params.period_end,
                filter_params.app,
                filter_params.provider_id,
                filter_params.account_id,
                filter_params.model,
                filter_params.precision,
                filter_params.project_path,
                filter_params.search,
                require_match,
            ],
            |row| row.get(0),
        )?;

        Ok(SessionPage {
            sessions,
            total: checked_u64(total, "session count")?,
        })
    }

    /// One session with its full request timeline.
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

    /// One page of usage events, optionally scoped to a session.
    pub fn list_usage_events(
        &self,
        session_id: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<UsageEventPage> {
        self.list_usage_events_filtered(session_id, limit, offset, &UsageFilters::default())
    }

    /// One page of usage events narrowed by the shared filters.
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

    /// Totals over an explicit window.
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

    /// Totals over the filtered set.
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

    /// Usage grouped by model + serving provider, honouring the same filters as
    /// the dashboard so the breakdown always adds up to the headline numbers.
    pub fn model_breakdown(&self, filters: &UsageFilters) -> Result<Vec<ModelUsage>> {
        let filter_params = UsageFilterParams::from_filters(filters);
        let mut statement = self.connection.prepare(
            "SELECT u.model, u.provider_id, p.display_name, u.app,
                    COUNT(*),
                    COUNT(u.input_tokens_total), SUM(u.input_tokens_total),
                    COUNT(u.input_tokens_uncached), SUM(u.input_tokens_uncached),
                    COUNT(u.cache_read_tokens), SUM(u.cache_read_tokens),
                    COUNT(u.cache_write_tokens), SUM(u.cache_write_tokens),
                    COUNT(u.output_tokens_total), SUM(u.output_tokens_total),
                    COUNT(u.reasoning_tokens), SUM(u.reasoning_tokens),
                    COUNT(u.visible_output_tokens), SUM(u.visible_output_tokens),
                    COUNT(u.provider_reported_cost), SUM(u.provider_reported_cost),
                    COUNT(u.estimated_cost), SUM(u.estimated_cost)
             FROM usage_events u
             LEFT JOIN sessions s ON s.id = u.session_id
             LEFT JOIN providers p ON p.id = u.provider_id
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
             GROUP BY u.model, u.provider_id, u.app
             ORDER BY COUNT(*) DESC",
        )?;
        let rows = statement.query_map(
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
            |row| {
                Ok(ModelUsage {
                    model: row.get(0)?,
                    provider_id: row.get(1)?,
                    provider_name: row.get(2)?,
                    app: app_from_str(row.get::<_, String>(3)?).map_err(to_sql_error)?,
                    totals: totals_from_row_at(row, 4)?,
                })
            },
        )?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Render the filtered events as `csv` or `json`.
    ///
    /// The raw source payload is excluded: an export is shareable, and the
    /// payload is the one field that could carry something unexpected.
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

    /// Build the tray summary.
    ///
    /// Deliberately narrow: the newest event, its session's totals, today's
    /// total, and the most relevant official quota window. Everything the
    /// popover shows comes from here, so it never runs a broad aggregation.
    pub fn quick_summary(
        &self,
        now: DateTime<Utc>,
        collection_status: CollectionStatus,
        latest_warning: Option<String>,
    ) -> Result<QuickSummary> {
        let period_start = local_day_start(now).ok_or_else(|| StorageError::InvalidDateTime {
            field: "today_start".to_owned(),
            value: now.to_rfc3339(),
        })?;
        let period_end = period_start + chrono::Duration::days(1);
        let today_totals = self.usage_totals_for_period(period_start, period_end)?;
        let today_total_tokens = if today_totals.event_count == 0 {
            Some(0)
        } else {
            today_totals.total_tokens()
        };

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
        let active_quota_summary = active
            .as_ref()
            .and_then(|(_, _, _, _, _, _, account_id)| account_id.as_deref())
            .map(|account_id| self.latest_quota_summary(account_id))
            .transpose()?
            .flatten();
        // The newest session can belong to a locally inferred account (or to
        // another provider) even while the official ChatGPT account has a
        // fresh quota reading. Keep the active account when it has a match;
        // otherwise surface the newest ChatGPT quota so the tray remains a
        // useful official-quota view outside Cockpit.
        let quota_summary = match active_quota_summary {
            Some(quota) => Some(quota),
            None => self.latest_official_quota_summary()?,
        };

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
            session_provider_reported_cost: session_totals
                .as_ref()
                .and_then(|totals| totals.provider_reported_cost),
            session_estimated_cost: session_totals
                .as_ref()
                .and_then(|totals| totals.estimated_cost),
            today_total_tokens,
            today_provider_reported_cost: today_totals.provider_reported_cost,
            today_estimated_cost: today_totals.estimated_cost,
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
                 ORDER BY captured_at DESC,
                          CASE
                              WHEN window_type LIKE 'primary%' THEN 0
                              WHEN window_type LIKE 'secondary%' THEN 1
                              WHEN window_type = 'credits' THEN 3
                              ELSE 2
                          END,
                          id DESC LIMIT 1",
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

    fn latest_official_quota_summary(&self) -> Result<Option<QuotaSummary>> {
        self.connection
            .query_row(
                "SELECT q.window_type, q.used_percent, q.remaining_percent, q.reset_at,
                        q.credits_remaining, q.precision
                 FROM quota_snapshots q
                 INNER JOIN accounts a ON a.id = q.account_id
                 WHERE a.auth_mode = 'chatgpt'
                 ORDER BY q.captured_at DESC,
                          CASE
                              WHEN q.window_type LIKE 'primary%' THEN 0
                              WHEN q.window_type LIKE 'secondary%' THEN 1
                              WHEN q.window_type = 'credits' THEN 3
                              ELSE 2
                          END,
                          q.id DESC LIMIT 1",
                [],
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

    fn usage_totals_for_period(
        &self,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Result<UsageTotals> {
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
                 FROM usage_events
                 WHERE occurred_at >= ?1 AND occurred_at < ?2",
                params![period_start.to_rfc3339(), period_end.to_rfc3339()],
                totals_from_row,
            )
            .map_err(Into::into)
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
    let default_start = local_day_start(now).unwrap_or(now);
    let period_start = filters.period_start.unwrap_or(default_start);
    let period_end = filters
        .period_end
        .unwrap_or_else(|| period_start + chrono::Duration::days(1));
    (period_start, period_end)
}

/// The UTC instant at which the current local calendar day began. "Today" in
/// TokenBuddy's tray and dashboard is the user's local day, not a UTC day — an
/// observability tool that reports "今日 Token" must agree with the wall clock
/// on the machine it runs on. Uses the earliest valid instant so a daylight-time
/// gap at local midnight still resolves deterministically.
fn local_day_start(now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let local_midnight = now
        .with_timezone(&Local)
        .date_naive()
        .and_hms_opt(0, 0, 0)?;
    Local
        .from_local_datetime(&local_midnight)
        .earliest()
        .map(|start| start.with_timezone(&Utc))
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

fn upsert_attribution(conn: &Connection, attribution: &SessionProviderAttribution) -> Result<()> {
    conn.execute(
        "INSERT INTO session_provider_attributions (
             session_id, provider_id, account_id, source_id, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(session_id) DO UPDATE SET
             provider_id = excluded.provider_id,
             account_id = COALESCE(excluded.account_id, session_provider_attributions.account_id),
             source_id = excluded.source_id,
             updated_at = excluded.updated_at",
        params![
            attribution.session_id,
            attribution.provider_id,
            attribution.account_id,
            attribution.source_id,
            now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

/// Rewrite already-stored events of an attributed session onto the real
/// provider. Without this, events imported before the launcher was scanned would
/// keep the provider guessed from their model name.
fn apply_attribution(conn: &Connection, attribution: &SessionProviderAttribution) -> Result<()> {
    conn.execute(
        "UPDATE usage_events
            SET provider_id = ?2,
                account_id = COALESCE(?3, account_id),
                estimated_cost = NULL,
                currency = CASE
                    WHEN provider_reported_cost IS NULL THEN NULL
                    ELSE currency
                END
          WHERE session_id = ?1",
        params![
            attribution.session_id,
            attribution.provider_id,
            attribution.account_id,
        ],
    )?;
    Ok(())
}

fn lookup_attribution(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<SessionProviderAttribution>> {
    conn.query_row(
        "SELECT session_id, provider_id, account_id, source_id
           FROM session_provider_attributions WHERE session_id = ?1",
        params![session_id],
        |row| {
            Ok(SessionProviderAttribution {
                session_id: row.get(0)?,
                provider_id: row.get(1)?,
                account_id: row.get(2)?,
                source_id: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn upsert_provider_record(conn: &Connection, provider: &ProviderRecord) -> Result<()> {
    let timestamp = now().to_rfc3339();
    conn.execute(
        "INSERT INTO providers (
             id, provider_family, display_name, upstream_url, launcher,
             source_id, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
         ON CONFLICT(id) DO UPDATE SET
             provider_family = excluded.provider_family,
             display_name = excluded.display_name,
             upstream_url = COALESCE(excluded.upstream_url, providers.upstream_url),
             launcher = COALESCE(excluded.launcher, providers.launcher),
             source_id = COALESCE(excluded.source_id, providers.source_id),
             updated_at = excluded.updated_at",
        params![
            provider.id,
            provider.provider_family,
            provider.display_name,
            provider.upstream_url,
            provider.launcher.map(LauncherKind::as_str),
            provider.source_id,
            timestamp,
        ],
    )?;
    Ok(())
}

fn upsert_account_window(conn: &Connection, window: &AccountActivityWindow) -> Result<()> {
    conn.execute(
        "INSERT INTO account_activity_windows (
             account_id, source_id, app, started_at, ended_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(account_id, source_id, started_at) DO UPDATE SET
             ended_at = MAX(excluded.ended_at, account_activity_windows.ended_at),
             updated_at = excluded.updated_at",
        params![
            window.account_id,
            window.source_id,
            window.app.as_str(),
            window.started_at.to_rfc3339(),
            window.ended_at.to_rfc3339(),
            now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

/// The account that was serving `app` at `occurred_at`, or `None`.
///
/// Returns `None` when several accounts' windows cover the instant: overlapping
/// windows mean the launcher's log cannot say which account served this request,
/// and a coin flip between two real accounts is worse than `Unavailable`.
fn account_at(
    conn: &Connection,
    app: AppKind,
    occurred_at: DateTime<Utc>,
) -> Result<Option<String>> {
    let timestamp = occurred_at.to_rfc3339();
    let mut statement = conn.prepare(
        "SELECT DISTINCT account_id
           FROM account_activity_windows
          WHERE app = ?1 AND started_at <= ?2 AND ended_at >= ?2
          LIMIT 2",
    )?;
    let mut accounts = statement
        .query_map(params![app.as_str(), timestamp], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if accounts.len() == 1 {
        Ok(accounts.pop())
    } else {
        Ok(None)
    }
}

/// Attach newly imported windows to events that were stored before the launcher
/// was scanned, where exactly one account covers the timestamp.
///
/// Events still carrying the placeholder account that storage derives from a
/// model name are rewritten too: a launcher that actually routed the request
/// outranks a per-provider bucket, the same way a launcher-reported provider
/// outranks one guessed from the model. An account another source resolved for
/// real is left alone.
fn backfill_account_windows(conn: &Connection, windows: &[AccountActivityWindow]) -> Result<u64> {
    let mut updated = 0;
    for window in windows {
        updated += conn.execute(
            "UPDATE usage_events
                SET account_id = ?1,
                    precision_account = ?2
              WHERE (
                    account_id IS NULL
                    OR account_id IN (
                        SELECT id FROM accounts WHERE auth_mode = 'session_log'
                    )
                )
                AND app = ?3
                AND occurred_at >= ?4
                AND occurred_at <= ?5
                AND NOT EXISTS (
                    SELECT 1 FROM account_activity_windows w
                     WHERE w.app = usage_events.app
                       AND w.account_id <> ?1
                       AND w.started_at <= usage_events.occurred_at
                       AND w.ended_at >= usage_events.occurred_at
                )",
            params![
                window.account_id,
                PrecisionLevel::Correlated.as_str(),
                window.app.as_str(),
                window.started_at.to_rfc3339(),
                window.ended_at.to_rfc3339(),
            ],
        )? as u64;
    }
    Ok(updated)
}

/// Persist an account a source actually identified. `display_name`, `plan` and
/// `auth_mode` are refreshed on every import (a plan can change), but a stored
/// value is never replaced by a missing one.
fn upsert_account_record(conn: &Connection, account: &AccountRecord) -> Result<()> {
    let timestamp = now().to_rfc3339();
    conn.execute(
        "INSERT INTO accounts (
             id, provider_id, display_name, account_fingerprint, auth_mode,
             plan, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
         ON CONFLICT(id) DO UPDATE SET
             provider_id = excluded.provider_id,
             display_name = COALESCE(excluded.display_name, accounts.display_name),
             account_fingerprint = excluded.account_fingerprint,
             auth_mode = excluded.auth_mode,
             plan = COALESCE(excluded.plan, accounts.plan),
             updated_at = excluded.updated_at",
        params![
            account.id,
            account.provider_id,
            account.display_name,
            account.account_fingerprint,
            account.auth_mode,
            account.plan,
            timestamp,
        ],
    )?;
    Ok(())
}

/// Quota snapshots carry an id derived from their content, so re-importing the
/// same log line is a no-op rather than a second point in the time series.
fn insert_quota_snapshot(conn: &Connection, snapshot: &QuotaSnapshot) -> Result<bool> {
    let raw_json = snapshot
        .raw_json
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let changed = conn.execute(
        "INSERT INTO quota_snapshots (
             id, account_id, captured_at, window_type, used_percent,
             remaining_percent, reset_at, credits_remaining, precision, raw_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(id) DO NOTHING",
        params![
            snapshot.id,
            snapshot.account_id,
            snapshot.captured_at.to_rfc3339(),
            snapshot.window_type,
            snapshot.used_percent,
            snapshot.remaining_percent,
            snapshot.reset_at.map(|value| value.to_rfc3339()),
            snapshot.credits_remaining,
            snapshot.precision.as_str(),
            raw_json,
        ],
    )?;
    Ok(changed == 1)
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
             -- Sessions are imported as an incremental tail: each batch only sees
             -- the events in its own chunk, so a plain COALESCE overwrite would
             -- drag started_at forward to the newest chunk's first event on every
             -- poll. Keep the earliest start and the latest end across all chunks.
             started_at = MIN(
                 COALESCE(excluded.started_at, sessions.started_at),
                 COALESCE(sessions.started_at, excluded.started_at)
             ),
             ended_at = MAX(
                 COALESCE(excluded.ended_at, sessions.ended_at),
                 COALESCE(sessions.ended_at, excluded.ended_at)
             ),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InsertOutcome {
    /// A new request observation was stored.
    Inserted,
    /// The observation was already represented by an equal or stronger row.
    Duplicate,
    /// A weaker source row was replaced by this stronger observation.
    Reconciled,
}

#[derive(Debug)]
struct StoredCorrelation {
    id: String,
    ingest_source: tokenbuddy_domain::IngestSource,
    precision_token: PrecisionLevel,
}

fn correlated_events(conn: &Connection, event: &UsageEvent) -> Result<Vec<StoredCorrelation>> {
    let Some(_key) = correlation_key(
        event.app,
        event.request_id.as_deref(),
        event.response_id.as_deref(),
    ) else {
        return Ok(Vec::new());
    };
    let mut statement = conn.prepare(
        "SELECT id, ingest_source, precision_token
           FROM usage_events
          WHERE app = ?1
            AND ((?2 IS NOT NULL AND request_id = ?2)
              OR (?3 IS NOT NULL AND response_id = ?3))",
    )?;
    let mut rows = statement.query(params![
        event.app.as_str(),
        event.request_id.as_deref(),
        event.response_id.as_deref(),
    ])?;
    let mut correlations = Vec::new();
    while let Some(row) = rows.next()? {
        let ingest_source = ingest_source_from_str(&row.get::<_, String>(1)?)?;
        let precision_token = precision_from_str(&row.get::<_, String>(2)?)?;
        correlations.push(StoredCorrelation {
            id: row.get(0)?,
            ingest_source,
            precision_token,
        });
    }
    Ok(correlations)
}

fn insert_usage_event(
    conn: &Connection,
    event: &UsageEvent,
    derived: Option<&DerivedProvider>,
    attributed: Option<&SessionProviderAttribution>,
    windowed_account: Option<&str>,
    save_request_metadata: bool,
) -> Result<InsertOutcome> {
    // Precedence: launcher-reported truth > identity the adapter already
    // resolved > provider guessed from the model name.
    let provider_id = attributed
        .map(|value| value.provider_id.clone())
        .or_else(|| event.provider_id.clone())
        .or_else(|| derived.map(|derived| derived.id.clone()));
    let account_id = attributed
        .and_then(|value| value.account_id.clone())
        .or_else(|| event.account_id.clone())
        .or_else(|| windowed_account.map(str::to_owned))
        .or_else(|| derived.map(|derived| derived.account_id.clone()));
    // A time-window match is a correlation; say so rather than inheriting the
    // adapter's precision for an account it never saw.
    let precision_account = if windowed_account.is_some() {
        PrecisionLevel::Correlated
    } else {
        event.precision_account
    };
    let provider_reported_cost = event.provider_reported_cost;
    let estimated_cost = if provider_reported_cost.is_some() {
        None
    } else {
        let provider_upstream_url = provider_id
            .as_deref()
            .map(|provider_id| provider_upstream_url(conn, provider_id))
            .transpose()?
            .flatten();
        event.estimated_cost.or_else(|| {
            pricing::estimate_cost(
                provider_id.as_deref(),
                provider_upstream_url.as_deref(),
                event.model.as_deref(),
                &event.usage,
            )
        })
    };
    let currency = event.currency.clone().or_else(|| {
        (provider_reported_cost.is_some() || estimated_cost.is_some()).then(|| "USD".to_owned())
    });

    let same_hash_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM usage_events WHERE raw_event_hash = ?1)",
        params![event.raw_event_hash.as_str()],
        |row| row.get(0),
    )?;
    if same_hash_exists {
        // A parser upgrade may enrich the same stable row with a model, cost,
        // or a complete usage object that followed a streamed provisional
        // value. Repeated imports remain idempotent and never replace a known
        // complete usage object with a weaker observation.
        let metadata_changed = conn.execute(
            "UPDATE usage_events SET
                 model = COALESCE(model, ?1),
                 provider_id = COALESCE(provider_id, ?2),
                 account_id = COALESCE(account_id, ?3),
                 provider_reported_cost = COALESCE(?4, provider_reported_cost),
                 estimated_cost = CASE
                     WHEN ?4 IS NOT NULL THEN NULL
                     ELSE COALESCE(estimated_cost, ?5)
                 END,
                 currency = COALESCE(currency, ?6)
             WHERE raw_event_hash = ?7
               AND (
                   (?1 IS NOT NULL AND model IS NULL)
                   OR (?2 IS NOT NULL AND provider_id IS NULL)
                   OR (?3 IS NOT NULL AND account_id IS NULL)
                   OR (?4 IS NOT NULL AND provider_reported_cost IS NULL)
                   OR (?4 IS NOT NULL AND estimated_cost IS NOT NULL)
                   OR (?4 IS NULL AND ?5 IS NOT NULL AND estimated_cost IS NULL
                       AND provider_reported_cost IS NULL)
                   OR (?6 IS NOT NULL AND currency IS NULL)
               )",
            params![
                event.model.as_deref(),
                provider_id.as_deref(),
                account_id.as_deref(),
                provider_reported_cost,
                estimated_cost,
                currency.as_deref(),
                event.raw_event_hash.as_str(),
            ],
        )?;
        let existing_usage = stored_usage_by_hash(conn, event.raw_event_hash.as_str())?;
        let replace_provisional = existing_usage.as_ref().is_some_and(|existing| {
            existing.input_tokens_total.is_none() && event.usage.input_tokens_total.is_some()
        });
        let raw_usage_json = if save_request_metadata {
            event
                .raw_usage_json
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?
        } else {
            None
        };
        let usage_changed = if replace_provisional {
            // Claude Code can emit an initial {input:0, output:0} observation
            // and then a complete usage object with the same message id. Once
            // the total input becomes known, the latter is the authoritative
            // form of that same response, including a non-zero output value.
            conn.execute(
                "UPDATE usage_events SET
                     input_tokens_total = COALESCE(?1, input_tokens_total),
                     input_tokens_uncached = COALESCE(?2, input_tokens_uncached),
                     cache_read_tokens = COALESCE(?3, cache_read_tokens),
                     cache_write_tokens = COALESCE(?4, cache_write_tokens),
                     output_tokens_total = COALESCE(?5, output_tokens_total),
                     reasoning_tokens = COALESCE(?6, reasoning_tokens),
                     visible_output_tokens = COALESCE(?7, visible_output_tokens),
                     raw_usage_json = COALESCE(?8, raw_usage_json)
                 WHERE raw_event_hash = ?9",
                params![
                    option_i64(event.usage.input_tokens_total, "input_tokens_total")?,
                    option_i64(event.usage.input_tokens_uncached, "input_tokens_uncached")?,
                    option_i64(event.usage.cache_read_tokens, "cache_read_tokens")?,
                    option_i64(event.usage.cache_write_tokens, "cache_write_tokens")?,
                    option_i64(event.usage.output_tokens_total, "output_tokens_total")?,
                    option_i64(event.usage.reasoning_tokens, "reasoning_tokens")?,
                    option_i64(event.usage.visible_output_tokens, "visible_output_tokens")?,
                    raw_usage_json,
                    event.raw_event_hash.as_str(),
                ],
            )?
        } else {
            0
        };
        return Ok(if metadata_changed > 0 || usage_changed > 0 {
            InsertOutcome::Reconciled
        } else {
            InsertOutcome::Duplicate
        });
    }
    let existing = correlated_events(conn, event)?;
    let new_score = (
        event.precision_token.precedence(),
        event.ingest_source.precedence(),
    );
    let stronger_than_existing = existing.iter().max_by_key(|stored| {
        (
            stored.precision_token.precedence(),
            stored.ingest_source.precedence(),
        )
    });
    let reconciled = if let Some(stored) = stronger_than_existing {
        let existing_score = (
            stored.precision_token.precedence(),
            stored.ingest_source.precedence(),
        );
        if new_score <= existing_score {
            // Same request, same or stronger observation already stored: do not
            // count a second row merely because the source encoded it differently.
            return Ok(InsertOutcome::Duplicate);
        }
        // A higher-confidence source (for example OTel Verified) replaces the
        // lower-confidence session/proxy observation. Deleting every correlated
        // row also heals databases produced by an older build that had no
        // cross-source reconciliation yet.
        for row in &existing {
            conn.execute("DELETE FROM usage_events WHERE id = ?1", params![row.id])?;
        }
        true
    } else {
        false
    };

    let raw_usage_json = if save_request_metadata {
        event
            .raw_usage_json
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?
    } else {
        None
    };
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
            provider_id,
            account_id,
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
            provider_reported_cost,
            estimated_cost,
            currency,
            event.http_status,
            event.latency_ms,
            event.success.map(bool_to_i64),
            event.precision_token.as_str(),
            event.precision_session.as_str(),
            event.precision_provider.as_str(),
            precision_account.as_str(),
            event.raw_event_hash,
            raw_usage_json,
            now().to_rfc3339(),
        ],
    )?;
    Ok(if changed == 1 {
        if reconciled {
            InsertOutcome::Reconciled
        } else {
            InsertOutcome::Inserted
        }
    } else {
        InsertOutcome::Duplicate
    })
}

/// Recalculate API-equivalent estimates for rows that were imported before a
/// newly supported model price card existed. Provider-reported costs remain
/// authoritative and are never overwritten.
fn refresh_estimated_costs_in_connection(conn: &mut Connection) -> Result<()> {
    let transaction = conn.transaction()?;
    refresh_estimated_costs_on_connection(&transaction)?;
    transaction.commit()?;
    Ok(())
}

fn refresh_estimated_costs_on_connection(conn: &Connection) -> Result<()> {
    let candidates = {
        let mut statement = conn.prepare(
            "SELECT u.id, u.provider_id, p.upstream_url, u.model,
                    u.input_tokens_uncached, u.cache_read_tokens,
                    u.cache_write_tokens, u.output_tokens_total
             FROM usage_events u
             LEFT JOIN providers p ON p.id = u.provider_id
             WHERE u.provider_reported_cost IS NULL AND u.model IS NOT NULL",
        )?;
        let rows = statement.query_map([], |row| {
            let usage = NormalizedUsage {
                input_tokens_uncached: option_u64(row.get(4)?, "input_tokens_uncached")
                    .map_err(to_sql_error)?,
                cache_read_tokens: option_u64(row.get(5)?, "cache_read_tokens")
                    .map_err(to_sql_error)?,
                cache_write_tokens: option_u64(row.get(6)?, "cache_write_tokens")
                    .map_err(to_sql_error)?,
                output_tokens_total: option_u64(row.get(7)?, "output_tokens_total")
                    .map_err(to_sql_error)?,
                ..Default::default()
            };
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                usage,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    if candidates.is_empty() {
        return Ok(());
    }

    for (id, provider_id, provider_upstream_url, model, usage) in candidates {
        let Some(cost) = pricing::estimate_cost(
            provider_id.as_deref(),
            provider_upstream_url.as_deref(),
            model.as_deref(),
            &usage,
        ) else {
            // Some adapters (notably OpenCode) supply their own estimate. An
            // unmatched static card must not erase that independent value.
            continue;
        };
        conn.execute(
            "UPDATE usage_events
             SET estimated_cost = ?1,
                 currency = COALESCE(currency, 'USD')
             WHERE id = ?2 AND provider_reported_cost IS NULL",
            params![cost, id],
        )?;
    }
    Ok(())
}

fn provider_upstream_url(conn: &Connection, provider_id: &str) -> Result<Option<String>> {
    let upstream_url = conn
        .query_row(
            "SELECT upstream_url FROM providers WHERE id = ?1",
            params![provider_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    Ok(upstream_url.flatten())
}

fn stored_usage_by_hash(
    conn: &Connection,
    raw_event_hash: &str,
) -> Result<Option<NormalizedUsage>> {
    conn.query_row(
        "SELECT input_tokens_total, input_tokens_uncached, cache_read_tokens,
                cache_write_tokens, output_tokens_total, reasoning_tokens,
                visible_output_tokens
           FROM usage_events WHERE raw_event_hash = ?1",
        params![raw_event_hash],
        |row| {
            Ok(NormalizedUsage {
                input_tokens_total: option_u64(row.get(0)?, "input_tokens_total")
                    .map_err(to_sql_error)?,
                input_tokens_uncached: option_u64(row.get(1)?, "input_tokens_uncached")
                    .map_err(to_sql_error)?,
                cache_read_tokens: option_u64(row.get(2)?, "cache_read_tokens")
                    .map_err(to_sql_error)?,
                cache_write_tokens: option_u64(row.get(3)?, "cache_write_tokens")
                    .map_err(to_sql_error)?,
                output_tokens_total: option_u64(row.get(4)?, "output_tokens_total")
                    .map_err(to_sql_error)?,
                reasoning_tokens: option_u64(row.get(5)?, "reasoning_tokens")
                    .map_err(to_sql_error)?,
                visible_output_tokens: option_u64(row.get(6)?, "visible_output_tokens")
                    .map_err(to_sql_error)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

/// A provider (and grouping account) inferred from a session-log event's model
/// and app. Session logs do not name a provider, but the model prefix reliably
/// identifies one, which is enough to populate the Providers view honestly.
struct DerivedProvider {
    id: String,
    family: String,
    display_name: String,
    account_id: String,
    account_name: String,
}

fn derive_provider(event: &UsageEvent) -> Option<DerivedProvider> {
    // Respect an identity the adapter already resolved (e.g. a proxy source).
    if event.provider_id.is_some() {
        return None;
    }
    let (family, display_name) = provider_family(event.model.as_deref(), event.app);
    Some(DerivedProvider {
        id: family.to_owned(),
        family: family.to_owned(),
        display_name: display_name.to_owned(),
        account_id: format!("{family}:local"),
        account_name: "本地会话（来自会话日志）".to_owned(),
    })
}

fn provider_family(model: Option<&str>, app: AppKind) -> (&'static str, &'static str) {
    let model = model.unwrap_or_default().to_ascii_lowercase();
    let prefix_match = [
        ("claude", ("anthropic", "Anthropic")),
        ("gpt", ("openai", "OpenAI")),
        ("chatgpt", ("openai", "OpenAI")),
        ("o1", ("openai", "OpenAI")),
        ("o3", ("openai", "OpenAI")),
        ("o4", ("openai", "OpenAI")),
        ("codex", ("openai", "OpenAI")),
        ("gemini", ("google", "Google")),
        ("grok", ("xai", "xAI")),
    ]
    .into_iter()
    .find_map(|(prefix, provider)| model.starts_with(prefix).then_some(provider));
    if let Some(provider) = prefix_match {
        return provider;
    }
    match app {
        AppKind::Codex => ("openai", "OpenAI"),
        AppKind::ClaudeCode => ("anthropic", "Anthropic"),
        // OpenCode models are usually served through user-configured relays, so
        // a model name never implies a first-party vendor here. DeepSeek
        // Harness events carry the provider their routing stated, so a model
        // name alone must not guess one either.
        AppKind::OpenCode | AppKind::DeepseekHarness => ("unknown", "Unknown"),
        AppKind::Unknown => ("unknown", "Unknown"),
    }
}

fn ensure_provider(conn: &Connection, derived: &DerivedProvider, event: &UsageEvent) -> Result<()> {
    let timestamp = now().to_rfc3339();
    conn.execute(
        "INSERT INTO providers (
             id, provider_family, display_name, upstream_url, launcher,
             source_id, created_at, updated_at
         ) VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?6)
         ON CONFLICT(id) DO NOTHING",
        params![
            derived.id,
            derived.family,
            derived.display_name,
            event.launcher.as_str(),
            event.source_id,
            timestamp,
        ],
    )?;
    Ok(())
}

fn ensure_account(conn: &Connection, derived: &DerivedProvider) -> Result<()> {
    let timestamp = now().to_rfc3339();
    conn.execute(
        "INSERT INTO accounts (
             id, provider_id, display_name, account_fingerprint, auth_mode,
             plan, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, 'session_log', NULL, ?5, ?5)
         ON CONFLICT(id) DO NOTHING",
        params![
            derived.account_id,
            derived.id,
            derived.account_name,
            derived.account_id,
            timestamp,
        ],
    )?;
    Ok(())
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
             content_hash, last_cumulative_usage, snapshot_generation,
             last_session_id, last_model, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(source_id, resource_id) DO UPDATE SET
             file_size = excluded.file_size,
             modified_at = excluded.modified_at,
             byte_offset = excluded.byte_offset,
             content_hash = excluded.content_hash,
             last_cumulative_usage = excluded.last_cumulative_usage,
             snapshot_generation = excluded.snapshot_generation,
             last_session_id = excluded.last_session_id,
             last_model = excluded.last_model,
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
            cursor.last_session_id,
            cursor.last_model,
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
        opencode_db_path: row.get(9)?,
        dsh_home: row.get(10)?,
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
        last_session_id: row.get(8)?,
        last_model: row.get(9)?,
        updated_at: parse_datetime("updated_at", row.get(10)?).map_err(to_sql_error)?,
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
        "open_code" => Ok(AppKind::OpenCode),
        "deepseek_harness" => Ok(AppKind::DeepseekHarness),
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
    use chrono::{DateTime, Duration, Utc};
    use tokenbuddy_domain::{
        AccountActivityWindow, AccountRecord, AppKind, AppSettings, CollectionStatus, ImportBatch,
        ImportCursor, IngestSource, LauncherKind, NormalizedUsage, PrecisionLevel, ProviderRecord,
        QuotaSnapshot, SessionProviderAttribution, SessionRecord, SourceRecord, UsageEvent,
        UsageFilters,
    };

    use super::{Database, RetentionOutcome};

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

    fn official_account() -> AccountRecord {
        AccountRecord {
            id: "openai:chatgpt:fixture00000000".to_owned(),
            provider_id: "openai".to_owned(),
            display_name: Some("fixture@example.com".to_owned()),
            account_fingerprint: "fixture00000000feedfacefeedface".to_owned(),
            auth_mode: "chatgpt".to_owned(),
            plan: Some("pro".to_owned()),
        }
    }

    fn inferred_account() -> AccountRecord {
        AccountRecord {
            id: "openai:local".to_owned(),
            provider_id: "openai".to_owned(),
            display_name: Some("本地会话".to_owned()),
            account_fingerprint: "local-session-account".to_owned(),
            auth_mode: "session_log".to_owned(),
            plan: None,
        }
    }

    fn quota(id: &str, used_percent: f64, captured_at: DateTime<Utc>) -> QuotaSnapshot {
        QuotaSnapshot {
            id: id.to_owned(),
            account_id: official_account().id,
            account_name: None,
            provider_name: None,
            captured_at,
            window_type: "primary_5h".to_owned(),
            used_percent: Some(used_percent),
            remaining_percent: Some(100.0 - used_percent),
            reset_at: Some(captured_at + Duration::hours(1)),
            credits_remaining: None,
            precision: PrecisionLevel::Correlated,
            raw_json: Some(serde_json::json!({"used_percent": used_percent})),
        }
    }

    #[test]
    fn official_accounts_and_quota_windows_survive_a_repeated_import() {
        let mut database = Database::open_in_memory().expect("database opens");
        let captured_at = Utc::now();
        let mut event = event("quota-event", Some(40));
        event.account_id = Some(official_account().id);
        event.precision_account = PrecisionLevel::Correlated;
        let batch = ImportBatch {
            source: Some(source()),
            accounts: vec![official_account()],
            sessions: vec![session()],
            usage_events: vec![event],
            quota_snapshots: vec![quota("quota-1", 12.5, captured_at)],
            ..ImportBatch::default()
        };

        let first = database.apply_import_batch(&batch).expect("first import");
        assert_eq!(first.upserted_accounts, 1);
        assert_eq!(first.inserted_quota_snapshots, 1);

        let second = database.apply_import_batch(&batch).expect("second import");
        assert_eq!(second.inserted_events, 0);
        assert_eq!(
            second.inserted_quota_snapshots, 0,
            "re-importing the same window must not add a second data point"
        );
        assert_eq!(
            database
                .list_quota_snapshots(None, 10)
                .expect("quota snapshots")
                .len(),
            1
        );

        // The account resolves with its provider and newest window, and the
        // percentages are never turned into a token count.
        let accounts = database.list_accounts().expect("accounts");
        let summary = accounts
            .iter()
            .find(|summary| summary.account.auth_mode == "chatgpt")
            .expect("official account");
        assert_eq!(summary.account.plan.as_deref(), Some("pro"));
        assert_eq!(
            summary
                .latest_quota
                .as_ref()
                .and_then(|quota| quota.used_percent),
            Some(12.5)
        );

        // A newer window for the same account is a new row, and the summary
        // follows it.
        let newer = ImportBatch {
            quota_snapshots: vec![quota("quota-2", 18.75, captured_at + Duration::minutes(5))],
            ..ImportBatch::default()
        };
        assert_eq!(
            database
                .apply_import_batch(&newer)
                .expect("newer quota")
                .inserted_quota_snapshots,
            1
        );
        let summary = database
            .quick_summary(Utc::now(), CollectionStatus::Collecting, None)
            .expect("quick summary");
        let quota_summary = summary.quota_summary.expect("tray quota summary");
        assert_eq!(quota_summary.used_percent, Some(18.75));
        assert_eq!(quota_summary.window_type, "primary_5h");
        assert_eq!(quota_summary.precision, PrecisionLevel::Correlated);
    }

    #[test]
    fn quick_summary_falls_back_to_the_newest_official_quota() {
        let mut database = Database::open_in_memory().expect("database opens");
        let captured_at = Utc::now();
        let mut active_event = event("local-session-event", Some(40));
        active_event.account_id = Some(inferred_account().id);

        database
            .apply_import_batch(&ImportBatch {
                source: Some(source()),
                accounts: vec![official_account(), inferred_account()],
                sessions: vec![session()],
                usage_events: vec![active_event],
                quota_snapshots: vec![quota("official-quota", 8.0, captured_at)],
                ..ImportBatch::default()
            })
            .expect("import local session and official quota");

        let summary = database
            .quick_summary(Utc::now(), CollectionStatus::Collecting, None)
            .expect("quick summary");
        let quota_summary = summary
            .quota_summary
            .expect("official quota should be visible in the tray");
        assert_eq!(quota_summary.used_percent, Some(8.0));
        assert_eq!(quota_summary.remaining_percent, Some(92.0));
        assert_eq!(quota_summary.precision, PrecisionLevel::Correlated);
    }

    fn window(account_id: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> AccountActivityWindow {
        AccountActivityWindow {
            account_id: account_id.to_owned(),
            source_id: "cockpit".to_owned(),
            app: AppKind::Codex,
            started_at: start,
            ended_at: end,
        }
    }

    fn cockpit_account(id: &str) -> AccountRecord {
        AccountRecord {
            id: id.to_owned(),
            provider_id: "openai".to_owned(),
            display_name: Some(format!("{id}@example.com")),
            account_fingerprint: format!("fingerprint-{id}"),
            auth_mode: "cockpit".to_owned(),
            plan: None,
        }
    }

    fn event_at(hash: &str, occurred_at: DateTime<Utc>) -> UsageEvent {
        UsageEvent {
            occurred_at,
            ..event(hash, Some(10))
        }
    }

    #[test]
    fn a_launcher_activity_window_attributes_events_by_time_and_refuses_when_ambiguous() {
        let mut database = Database::open_in_memory().expect("database opens");
        let base = Utc::now() - Duration::hours(5);

        // Imported before the launcher was ever scanned.
        database
            .apply_import_batch(&ImportBatch {
                source: Some(source()),
                sessions: vec![session()],
                usage_events: vec![
                    event_at("first-account-event", base + Duration::minutes(5)),
                    event_at("second-account-event", base + Duration::minutes(65)),
                    event_at("ambiguous-event", base + Duration::minutes(125)),
                    event_at("outside-any-window-event", base + Duration::minutes(200)),
                ],
                ..ImportBatch::default()
            })
            .expect("events before the launcher scan");

        let stats = database
            .apply_import_batch(&ImportBatch {
                accounts: vec![cockpit_account("account-a"), cockpit_account("account-b")],
                account_windows: vec![
                    window("account-a", base, base + Duration::minutes(30)),
                    window(
                        "account-b",
                        base + Duration::minutes(60),
                        base + Duration::minutes(90),
                    ),
                    // Two accounts covering the same instant: the launcher log
                    // cannot say which one served it.
                    window(
                        "account-a",
                        base + Duration::minutes(120),
                        base + Duration::minutes(130),
                    ),
                    window(
                        "account-b",
                        base + Duration::minutes(121),
                        base + Duration::minutes(129),
                    ),
                ],
                ..ImportBatch::default()
            })
            .expect("launcher scan");
        assert_eq!(stats.attributed_account_events, 2);

        let stored = database
            .list_usage_events(None, 100, 0)
            .expect("events")
            .events;
        let account_of = |hash: &str| {
            stored
                .iter()
                .find(|event| event.id == hash)
                .and_then(|event| event.account_id.clone())
        };
        assert_eq!(
            account_of("first-account-event").as_deref(),
            Some("account-a")
        );
        assert_eq!(
            account_of("second-account-event").as_deref(),
            Some("account-b")
        );
        // Neither event resolves to a real account: they keep the placeholder
        // bucket a session log always lands in. Overlapping windows must not
        // pick one of two real accounts, and an instant no window covers has no
        // launcher evidence at all.
        for unattributed in ["ambiguous-event", "outside-any-window-event"] {
            assert_eq!(
                account_of(unattributed).as_deref(),
                Some("openai:local"),
                "{unattributed} must not be attributed to a real account"
            );
        }

        // An event imported *after* the windows exist resolves on insert, at
        // Correlated precision rather than the adapter's Unavailable.
        database
            .apply_import_batch(&ImportBatch {
                usage_events: vec![event_at("later-event", base + Duration::minutes(70))],
                ..ImportBatch::default()
            })
            .expect("later import");
        let later = database
            .list_usage_events(None, 100, 0)
            .expect("events")
            .events
            .into_iter()
            .find(|event| event.id == "later-event")
            .expect("later event");
        assert_eq!(later.account_id.as_deref(), Some("account-b"));
        assert_eq!(later.precision_account, PrecisionLevel::Correlated);
    }

    #[test]
    fn the_fingerprint_salt_is_generated_once_and_stays_out_of_app_settings() {
        let database = Database::open_in_memory().expect("database opens");
        let salt = database.local_salt().expect("salt");
        assert_eq!(salt.len(), 32);
        assert_eq!(salt, database.local_salt().expect("stable salt"));

        // Saving settings must not disturb it, and the salt must not travel to
        // the UI through AppSettings.
        database
            .save_app_settings(&AppSettings {
                codex_home: Some("/fixtures/codex".to_owned()),
                ..AppSettings::default()
            })
            .expect("save settings");
        assert_eq!(salt, database.local_salt().expect("salt survives save"));
        let settings = database.get_app_settings().expect("settings");
        assert!(
            !serde_json::to_string(&settings)
                .expect("json")
                .contains(&salt)
        );
    }

    #[test]
    fn raw_usage_metadata_is_opt_in_and_revocation_purges_existing_rows() {
        let mut database = Database::open_in_memory().expect("database opens");

        // Adapters may keep a sanitized usage object in memory for hashing and
        // diagnostics, but a fresh install must not persist it by default.
        database
            .apply_import_batch(&ImportBatch {
                usage_events: vec![event("metadata-default", Some(10))],
                ..ImportBatch::default()
            })
            .expect("default import");
        let stored = database
            .list_usage_events(None, 100, 0)
            .expect("stored events")
            .events;
        assert_eq!(stored[0].raw_usage_json, None);

        // The explicit opt-in preserves the sanitized usage object for future
        // imports; it does not retroactively restore metadata already discarded.
        database
            .save_app_settings(&AppSettings {
                save_request_metadata: true,
                ..AppSettings::default()
            })
            .expect("enable metadata");
        database
            .apply_import_batch(&ImportBatch {
                usage_events: vec![event("metadata-opt-in", Some(20))],
                ..ImportBatch::default()
            })
            .expect("opt-in import");
        let stored = database
            .list_usage_events(None, 100, 0)
            .expect("stored events")
            .events;
        assert!(
            stored
                .iter()
                .find(|event| event.id == "metadata-opt-in")
                .and_then(|event| event.raw_usage_json.as_ref())
                .is_some()
        );

        // Turning the setting off is a deletion request for the stored metadata,
        // not merely a preference for later imports.
        database
            .save_app_settings(&AppSettings::default())
            .expect("disable metadata");
        let stored = database
            .list_usage_events(None, 100, 0)
            .expect("stored events")
            .events;
        assert!(stored.iter().all(|event| event.raw_usage_json.is_none()));

        database
            .apply_import_batch(&ImportBatch {
                usage_events: vec![event("metadata-after-revocation", Some(30))],
                ..ImportBatch::default()
            })
            .expect("post-revocation import");
        let post_revocation = database
            .list_usage_events(None, 100, 0)
            .expect("stored events")
            .events
            .into_iter()
            .find(|event| event.id == "metadata-after-revocation")
            .expect("post-revocation event");
        assert_eq!(post_revocation.raw_usage_json, None);
    }

    #[test]
    fn a_stronger_correlated_source_replaces_a_weaker_row_without_double_counting() {
        let mut database = Database::open_in_memory().expect("database opens");
        let mut session_observation = event("session-observation", Some(100));
        session_observation.request_id = Some("shared-request".to_owned());

        let first = database
            .apply_import_batch(&ImportBatch {
                usage_events: vec![session_observation],
                ..ImportBatch::default()
            })
            .expect("session observation");
        assert_eq!(first.inserted_events, 1);

        let mut otel_observation = event("otel-observation", Some(140));
        otel_observation.request_id = Some("shared-request".to_owned());
        otel_observation.ingest_source = IngestSource::Otel;
        otel_observation.precision_token = PrecisionLevel::Verified;
        otel_observation.usage.output_tokens_total = Some(60);
        let reconciled = database
            .apply_import_batch(&ImportBatch {
                usage_events: vec![otel_observation],
                ..ImportBatch::default()
            })
            .expect("OTel observation");
        assert_eq!(reconciled.inserted_events, 0);
        assert_eq!(reconciled.reconciled_events, 1);

        let stored = database
            .list_usage_events(None, 100, 0)
            .expect("stored events")
            .events;
        assert_eq!(stored.len(), 1, "one request must remain one event");
        assert_eq!(stored[0].id, "otel-observation");
        assert_eq!(stored[0].ingest_source, IngestSource::Otel);
        assert_eq!(stored[0].usage.input_tokens_total, Some(140));

        // A later session-log copy cannot replace the verified OTel observation.
        let mut late_session = event("late-session-observation", Some(101));
        late_session.request_id = Some("shared-request".to_owned());
        let duplicate = database
            .apply_import_batch(&ImportBatch {
                usage_events: vec![late_session],
                ..ImportBatch::default()
            })
            .expect("late session observation");
        assert_eq!(duplicate.duplicate_events, 1);
        assert_eq!(
            database
                .list_usage_events(None, 100, 0)
                .expect("stored events")
                .events
                .len(),
            1
        );
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
                last_session_id: None,
                last_model: None,
                updated_at: now,
            }],
            skipped_records: 0,
            ..ImportBatch::default()
        };

        let first = database.apply_import_batch(&batch).expect("first import");
        let second = database.apply_import_batch(&batch).expect("second import");
        assert_eq!(first.inserted_events, 2);
        assert_eq!(second.inserted_events, 0);
        assert_eq!(second.duplicate_events, 2);

        let page = database
            .list_session_page(&UsageFilters::default(), 20, 0)
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
    fn complete_streamed_usage_reconciles_a_provisional_zero_row() {
        let mut database = Database::open_in_memory().expect("database opens");
        let mut provisional = event("streamed-response", None);
        provisional.app = AppKind::ClaudeCode;
        provisional.model = Some("deepseek-v4-flash".to_owned());
        provisional.response_id = Some("streamed-message-1".to_owned());
        provisional.usage.input_tokens_uncached = Some(0);
        provisional.usage.output_tokens_total = Some(0);

        let mut complete = provisional.clone();
        complete.usage = NormalizedUsage {
            input_tokens_total: Some(54_113),
            input_tokens_uncached: Some(225),
            cache_read_tokens: Some(53_888),
            cache_write_tokens: Some(0),
            output_tokens_total: Some(481),
            ..Default::default()
        };

        let report = database
            .apply_import_batch(&ImportBatch {
                usage_events: vec![provisional, complete.clone()],
                ..ImportBatch::default()
            })
            .expect("streamed usage import");
        assert_eq!(report.inserted_events, 1);
        assert_eq!(report.reconciled_events, 1);

        let stored = database
            .list_usage_events(None, 10, 0)
            .expect("stored event")
            .events
            .into_iter()
            .next()
            .expect("one event");
        assert_eq!(stored.usage, complete.usage);

        let repeated = database
            .apply_import_batch(&ImportBatch {
                usage_events: vec![complete],
                ..ImportBatch::default()
            })
            .expect("repeat complete usage");
        assert_eq!(repeated.inserted_events, 0);
        assert_eq!(repeated.duplicate_events, 1);
    }

    #[test]
    fn opencode_go_attribution_prices_deepseek_v4_flash() {
        let mut database = Database::open_in_memory().expect("database opens");
        let mut usage_event = event("opencode-go-deepseek", Some(54_113));
        usage_event.app = AppKind::ClaudeCode;
        usage_event.model = Some("deepseek-v4-flash".to_owned());
        usage_event.usage.input_tokens_uncached = Some(225);
        usage_event.usage.cache_read_tokens = Some(53_888);
        usage_event.usage.cache_write_tokens = Some(0);
        usage_event.usage.output_tokens_total = Some(481);
        let provider_id = "cc-switch:claude:opencode-go".to_owned();

        database
            .apply_import_batch(&ImportBatch {
                providers: vec![ProviderRecord {
                    id: provider_id.clone(),
                    provider_family: "claude".to_owned(),
                    display_name: "OpenCode Go".to_owned(),
                    upstream_url: Some("https://opencode.ai/zen/go".to_owned()),
                    launcher: Some(LauncherKind::CCSwitch),
                    source_id: Some("cc-switch".to_owned()),
                }],
                attributions: vec![SessionProviderAttribution {
                    session_id: "session-1".to_owned(),
                    provider_id,
                    account_id: None,
                    source_id: "cc-switch".to_owned(),
                }],
                sessions: vec![session()],
                usage_events: vec![usage_event],
                ..ImportBatch::default()
            })
            .expect("OpenCode Go import");

        let stored = database
            .list_usage_events(None, 10, 0)
            .expect("priced event")
            .events
            .into_iter()
            .next()
            .expect("one event");
        assert!(
            (stored.estimated_cost.expect("estimated cost") - 0.0003170664).abs() < f64::EPSILON
        );
        assert_eq!(stored.currency.as_deref(), Some("USD"));
    }

    #[test]
    fn official_deepseek_attribution_prices_deepseek_v4_flash() {
        let mut database = Database::open_in_memory().expect("database opens");
        let mut usage_event = event("official-deepseek-v4-flash", Some(54_113));
        usage_event.app = AppKind::ClaudeCode;
        usage_event.model = Some("deepseek-v4-flash".to_owned());
        usage_event.usage.input_tokens_uncached = Some(225);
        usage_event.usage.cache_read_tokens = Some(53_888);
        usage_event.usage.cache_write_tokens = Some(0);
        usage_event.usage.output_tokens_total = Some(481);
        let provider_id = "cc-switch:claude:deepseek-official".to_owned();

        database
            .apply_import_batch(&ImportBatch {
                providers: vec![ProviderRecord {
                    id: provider_id.clone(),
                    provider_family: "claude".to_owned(),
                    display_name: "DeepSeek Official".to_owned(),
                    upstream_url: Some("https://api.deepseek.com/anthropic".to_owned()),
                    launcher: Some(LauncherKind::CCSwitch),
                    source_id: Some("cc-switch".to_owned()),
                }],
                attributions: vec![SessionProviderAttribution {
                    session_id: "session-1".to_owned(),
                    provider_id,
                    account_id: None,
                    source_id: "cc-switch".to_owned(),
                }],
                sessions: vec![session()],
                usage_events: vec![usage_event],
                ..ImportBatch::default()
            })
            .expect("official DeepSeek import");

        let stored = database
            .list_usage_events(None, 10, 0)
            .expect("priced event")
            .events
            .into_iter()
            .next()
            .expect("one event");
        assert!(
            (stored.estimated_cost.expect("estimated cost") - 0.0003170664).abs() < f64::EPSILON
        );
        assert_eq!(stored.currency.as_deref(), Some("USD"));
    }

    #[test]
    fn estimates_known_model_costs_and_reconciles_old_unavailable_rows() {
        let mut database = Database::open_in_memory().expect("database opens");
        let mut legacy = event("priced-event", Some(100));
        legacy.model = None;
        legacy.usage.input_tokens_uncached = Some(75);

        let first = database
            .apply_import_batch(&ImportBatch {
                source: Some(source()),
                sessions: vec![session()],
                usage_events: vec![legacy.clone()],
                ..ImportBatch::default()
            })
            .expect("legacy import");
        assert_eq!(first.inserted_events, 1);
        assert_eq!(
            database
                .list_usage_events(None, 10, 0)
                .expect("legacy event")
                .events[0]
                .estimated_cost,
            None
        );

        let mut enriched = legacy;
        enriched.model = Some("gpt-5-codex".to_owned());
        let second = database
            .apply_import_batch(&ImportBatch {
                usage_events: vec![enriched],
                ..ImportBatch::default()
            })
            .expect("enriched re-import");
        assert_eq!(second.inserted_events, 0);
        assert_eq!(second.reconciled_events, 1);

        let event = database
            .list_usage_events(None, 10, 0)
            .expect("enriched event")
            .events
            .into_iter()
            .next()
            .expect("one event");
        assert_eq!(event.model.as_deref(), Some("gpt-5-codex"));
        assert!((event.estimated_cost.expect("estimated cost") - 0.000396875).abs() < f64::EPSILON);
        assert_eq!(event.currency.as_deref(), Some("USD"));

        let quick = database
            .quick_summary(Utc::now(), CollectionStatus::Collecting, None)
            .expect("quick summary");
        assert!(
            (quick.session_estimated_cost.expect("session cost") - 0.000396875).abs()
                < f64::EPSILON
        );
        assert!(
            (quick.today_estimated_cost.expect("today cost") - 0.000396875).abs() < f64::EPSILON
        );
    }

    #[test]
    fn refreshes_existing_rows_when_a_new_model_price_card_is_added() {
        let mut database = Database::open_in_memory().expect("database opens");
        let mut event = event("gpt-56-sol-existing", Some(100));
        event.provider_id = Some("openai".to_owned());
        event.model = Some("gpt-5.6-sol".to_owned());
        event.usage.input_tokens_uncached = Some(75);

        database
            .apply_import_batch(&ImportBatch {
                source: Some(source()),
                sessions: vec![session()],
                usage_events: vec![event],
                ..ImportBatch::default()
            })
            .expect("known model import");
        database
            .connection()
            .execute(
                "UPDATE usage_events SET estimated_cost = NULL, currency = NULL
                 WHERE id = 'gpt-56-sol-existing'",
                [],
            )
            .expect("simulate a legacy unavailable estimate");

        database
            .refresh_estimated_costs()
            .expect("refresh estimates");
        let stored = database
            .list_usage_events(None, 10, 0)
            .expect("refreshed event")
            .events
            .into_iter()
            .next()
            .expect("one event");
        assert!((stored.estimated_cost.expect("estimated cost") - 0.0012875).abs() < f64::EPSILON);
        assert_eq!(stored.currency.as_deref(), Some("USD"));
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

    #[test]
    fn incremental_session_upsert_keeps_earliest_start_and_latest_end() {
        let mut database = Database::open_in_memory().expect("database opens");
        let base = Utc::now();
        let mut early = session();
        early.started_at = Some(base);
        early.ended_at = Some(base + Duration::minutes(5));
        database
            .apply_import_batch(&ImportBatch {
                sessions: vec![early],
                ..ImportBatch::default()
            })
            .expect("first chunk");

        // A later incremental chunk only sees its own newer events; a plain
        // overwrite would drag started_at forward to this chunk's first event.
        let mut later = session();
        later.started_at = Some(base + Duration::minutes(10));
        later.ended_at = Some(base + Duration::minutes(20));
        database
            .apply_import_batch(&ImportBatch {
                sessions: vec![later],
                ..ImportBatch::default()
            })
            .expect("second chunk");

        let page = database
            .list_session_page(&UsageFilters::default(), 10, 0)
            .expect("sessions");
        assert_eq!(page.sessions[0].session.started_at, Some(base));
        assert_eq!(
            page.sessions[0].session.ended_at,
            Some(base + Duration::minutes(20))
        );
    }

    #[test]
    fn launcher_attribution_beats_provider_guessed_from_the_model_name() {
        let mut database = Database::open_in_memory().expect("database opens");

        // A relay-served model: the name says nothing about the real upstream.
        let mut relayed = event("relayed-1", Some(100));
        relayed.model = Some("deepseek-v4-pro".to_owned());
        relayed.app = AppKind::ClaudeCode;
        database
            .apply_import_batch(&ImportBatch {
                sessions: vec![session()],
                usage_events: vec![relayed.clone()],
                ..ImportBatch::default()
            })
            .expect("session-log import");

        // Without attribution the provider is guessed — and guesses wrong here.
        let guessed = database
            .list_usage_events(None, 10, 0)
            .expect("events")
            .events[0]
            .provider_id
            .clone();
        assert_eq!(guessed.as_deref(), Some("anthropic"));

        // The launcher then reports the truth for that session.
        database
            .apply_import_batch(&ImportBatch {
                attributions: vec![SessionProviderAttribution {
                    session_id: "session-1".to_owned(),
                    provider_id: "cc-switch:claude:deepseek".to_owned(),
                    account_id: None,
                    source_id: "cc-switch".to_owned(),
                }],
                ..ImportBatch::default()
            })
            .expect("attribution import");

        // Existing rows are corrected...
        let corrected = database
            .list_usage_events(None, 10, 0)
            .expect("events")
            .events[0]
            .provider_id
            .clone();
        assert_eq!(corrected.as_deref(), Some("cc-switch:claude:deepseek"));

        // ...and events imported afterwards never fall back to the guess.
        let mut later = event("relayed-2", Some(50));
        later.model = Some("deepseek-v4-pro".to_owned());
        later.app = AppKind::ClaudeCode;
        database
            .apply_import_batch(&ImportBatch {
                usage_events: vec![later],
                ..ImportBatch::default()
            })
            .expect("later import");
        let page = database.list_usage_events(None, 10, 0).expect("events");
        assert_eq!(page.total, 2);
        assert!(
            page.events
                .iter()
                .all(|event| event.provider_id.as_deref() == Some("cc-switch:claude:deepseek"))
        );
    }

    #[test]
    fn retention_prunes_old_usage_and_orphan_sessions() {
        let mut database = Database::open_in_memory().expect("database opens");
        let now = Utc::now();

        let mut old_session = session();
        old_session.id = "old".to_owned();
        old_session.updated_at = now - Duration::days(40);
        old_session.ended_at = Some(now - Duration::days(40));
        let mut old_event = event("old-event", Some(100));
        old_event.session_id = Some("old".to_owned());
        old_event.occurred_at = now - Duration::days(40);

        let mut fresh_session = session();
        fresh_session.id = "fresh".to_owned();
        let mut fresh_event = event("fresh-event", Some(50));
        fresh_event.session_id = Some("fresh".to_owned());
        fresh_event.occurred_at = now;

        database
            .apply_import_batch(&ImportBatch {
                sessions: vec![old_session, fresh_session],
                usage_events: vec![old_event, fresh_event],
                ..ImportBatch::default()
            })
            .expect("seed data");

        // A disabled window keeps everything.
        assert_eq!(
            database.enforce_retention(None, now).expect("disabled"),
            RetentionOutcome::default()
        );
        assert_eq!(
            database
                .enforce_retention(Some(0), now)
                .expect("zero window"),
            RetentionOutcome::default()
        );

        let outcome = database
            .enforce_retention(Some(30), now)
            .expect("enforce retention");
        assert_eq!(outcome.deleted_events, 1);
        assert_eq!(outcome.deleted_sessions, 1);

        let events = database.list_usage_events(None, 10, 0).expect("events");
        assert_eq!(events.total, 1);
        assert_eq!(events.events[0].id, "fresh-event");
        let sessions = database
            .list_session_page(&UsageFilters::default(), 10, 0)
            .expect("sessions");
        assert_eq!(sessions.total, 1);
        assert_eq!(sessions.sessions[0].session.id, "fresh");
    }
}
