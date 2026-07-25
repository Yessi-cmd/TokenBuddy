mod migrations;

use std::{collections::HashMap, path::Path, time::SystemTime};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use thiserror::Error;
use tokenbuddy_domain::{
    AppKind, DashboardSummary, ImportBatch, ImportCursor, LauncherKind, NormalizedUsage,
    PrecisionLevel, SessionDetail, SessionPage, SessionRecord, SessionSummary, SourceRecord,
    UsageEvent, UsageEventPage, UsageTotals,
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
                    COUNT(u.id), SUM(u.input_tokens_total), SUM(u.input_tokens_uncached),
                    SUM(u.cache_read_tokens), SUM(u.cache_write_tokens),
                    SUM(u.output_tokens_total), SUM(u.reasoning_tokens),
                    SUM(u.visible_output_tokens), SUM(u.provider_reported_cost),
                    SUM(u.estimated_cost)
             FROM sessions s
             LEFT JOIN usage_events u ON u.session_id = s.id
             WHERE (?1 IS NULL OR s.title LIKE ?1 OR s.project_path LIKE ?1
                    OR s.external_session_id LIKE ?1)
             GROUP BY s.id
             ORDER BY COALESCE(s.started_at, s.created_at) DESC
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
                        COUNT(u.id), SUM(u.input_tokens_total), SUM(u.input_tokens_uncached),
                        SUM(u.cache_read_tokens), SUM(u.cache_write_tokens),
                        SUM(u.output_tokens_total), SUM(u.reasoning_tokens),
                        SUM(u.visible_output_tokens), SUM(u.provider_reported_cost),
                        SUM(u.estimated_cost)
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
            "SELECT id, occurred_at, app, launcher, ingest_source, source_id,
                    provider_id, account_id, session_id, parent_session_id, request_id,
                    response_id, model, query_source, input_tokens_total,
                    input_tokens_uncached, cache_read_tokens, cache_write_tokens,
                    output_tokens_total, reasoning_tokens, visible_output_tokens,
                    provider_reported_cost, estimated_cost, currency, http_status,
                    latency_ms, success, precision_token, precision_session,
                    precision_provider, precision_account, raw_event_hash,
                    raw_usage_json
             FROM usage_events WHERE session_id = ?1 ORDER BY occurred_at, id",
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
        let mut statement = self.connection.prepare(
            "SELECT id, occurred_at, app, launcher, ingest_source, source_id,
                    provider_id, account_id, session_id, parent_session_id, request_id,
                    response_id, model, query_source, input_tokens_total,
                    input_tokens_uncached, cache_read_tokens, cache_write_tokens,
                    output_tokens_total, reasoning_tokens, visible_output_tokens,
                    provider_reported_cost, estimated_cost, currency, http_status,
                    latency_ms, success, precision_token, precision_session,
                    precision_provider, precision_account, raw_event_hash,
                    raw_usage_json
             FROM usage_events
             WHERE (?1 IS NULL OR session_id = ?1)
             ORDER BY occurred_at, id LIMIT ?2 OFFSET ?3",
        )?;
        let events = statement
            .query_map(
                params![
                    session_id,
                    checked_i64(limit, "limit")?,
                    checked_i64(offset, "offset")?
                ],
                usage_event_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let total: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM usage_events WHERE (?1 IS NULL OR session_id = ?1)",
            params![session_id],
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
        let totals = self.connection.query_row(
            "SELECT COUNT(*), SUM(input_tokens_total), SUM(input_tokens_uncached),
                    SUM(cache_read_tokens), SUM(cache_write_tokens),
                    SUM(output_tokens_total), SUM(reasoning_tokens),
                    SUM(visible_output_tokens), SUM(provider_reported_cost),
                    SUM(estimated_cost)
             FROM usage_events
             WHERE occurred_at >= ?1 AND occurred_at < ?2",
            params![period_start.to_rfc3339(), period_end.to_rfc3339()],
            totals_from_row,
        )?;

        Ok(DashboardSummary {
            period_start,
            period_end,
            totals,
        })
    }
}

fn configure_connection(connection: &Connection) -> Result<()> {
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(())
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
    let totals = UsageTotals {
        event_count: checked_u64(row.get::<_, i64>(start)?, "event count").map_err(to_sql_error)?,
        input_tokens_total: option_u64(row.get(start + 1)?, "input_tokens_total")
            .map_err(to_sql_error)?,
        input_tokens_uncached: option_u64(row.get(start + 2)?, "input_tokens_uncached")
            .map_err(to_sql_error)?,
        cache_read_tokens: option_u64(row.get(start + 3)?, "cache_read_tokens")
            .map_err(to_sql_error)?,
        cache_write_tokens: option_u64(row.get(start + 4)?, "cache_write_tokens")
            .map_err(to_sql_error)?,
        output_tokens_total: option_u64(row.get(start + 5)?, "output_tokens_total")
            .map_err(to_sql_error)?,
        reasoning_tokens: option_u64(row.get(start + 6)?, "reasoning_tokens")
            .map_err(to_sql_error)?,
        visible_output_tokens: option_u64(row.get(start + 7)?, "visible_output_tokens")
            .map_err(to_sql_error)?,
        provider_reported_cost: row.get(start + 8)?,
        estimated_cost: row.get(start + 9)?,
        cache_hit_rate_percent: None,
    };
    Ok(UsageTotals {
        cache_hit_rate_percent: cache_hit_rate(totals.input_tokens_total, totals.cache_read_tokens),
        ..totals
    })
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
        AppKind, ImportBatch, ImportCursor, IngestSource, LauncherKind, NormalizedUsage,
        PrecisionLevel, SessionRecord, SourceRecord, UsageEvent,
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
        assert_eq!(page.sessions[0].totals.input_tokens_total, Some(100));
        assert_eq!(page.sessions[0].totals.output_tokens_total, Some(30));
        assert_eq!(page.sessions[0].totals.cache_hit_rate_percent, Some(25.0));
    }
}
