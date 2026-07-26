//! Read-only Cockpit Tools adapter.
//!
//! Cockpit Tools (the open-source `jlcodes99/cockpit-tools`) records the Codex
//! requests it routes through its local access proxy into a SQLite database at
//! `~/.antigravity_cockpit/codex_local_access_logs.sqlite`, table `request_logs`.
//! That table is the spec's "read-only database" fallback (§11.1) and is the
//! only non-sensitive, request-level surface — the WebSocket exposes account
//! aliases but no usage, and the `/report` quota API is off by default.
//!
//! TokenBuddy opens the database read-only, probes `sqlite_master`, and maps
//! each row to a usage event keyed on the stable `event_key`. Credentials and
//! the account store are never touched.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::SystemTime,
};

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, OpenFlags, Row};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokenbuddy_domain::{
    AppKind, DetectionResult, ImportBatch, ImportCursor, IngestSource, LauncherKind,
    NormalizedUsage, PrecisionLevel, ProviderRecord, SourceHealth, SourceRecord, UsageEvent,
};

pub const SOURCE_ID: &str = "cockpit";
pub const ADAPTER_TYPE: &str = "cockpit";
pub const DISPLAY_NAME: &str = "Cockpit Tools";
pub const DB_FILENAME: &str = "codex_local_access_logs.sqlite";
pub const HOME_DIRNAME: &str = ".antigravity_cockpit";
const LOGS_RESOURCE_ID: &str = "request_logs";

#[derive(Debug, Error)]
pub enum CockpitAdapterError {
    #[error("failed to read Cockpit database: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Debug, Clone)]
pub struct CockpitAdapter {
    db_path: PathBuf,
}

impl CockpitAdapter {
    /// `path` may be the request-log SQLite file, the `~/.antigravity_cockpit`
    /// directory, or its parent — whatever the user pointed the setting at.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            db_path: resolve_db_path(path.into()),
        }
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn detect_sync(&self) -> Result<DetectionResult, CockpitAdapterError> {
        let detected = self.db_path.is_file();
        Ok(DetectionResult {
            source_id: SOURCE_ID.to_owned(),
            detected,
            path_or_endpoint: Some(self.db_path.to_string_lossy().into_owned()),
            detected_version: detected.then(|| "sqlite".to_owned()),
            message: Some(if detected {
                "Cockpit request-log database detected".to_owned()
            } else {
                "Cockpit request-log database was not found".to_owned()
            }),
        })
    }

    pub fn health_sync(&self) -> SourceHealth {
        let detected = self.db_path.is_file();
        SourceHealth {
            source_id: SOURCE_ID.to_owned(),
            status: if detected {
                "healthy".to_owned()
            } else {
                "not_found".to_owned()
            },
            last_success_at: detected.then(now),
            last_error: None,
        }
    }

    pub fn import_history_sync(
        &self,
        cursors: &HashMap<String, ImportCursor>,
    ) -> Result<ImportBatch, CockpitAdapterError> {
        if !self.db_path.is_file() {
            return Ok(ImportBatch {
                source: Some(self.source_record("not_found")),
                ..ImportBatch::default()
            });
        }
        let connection =
            Connection::open_with_flags(&self.db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;

        let mut batch = ImportBatch {
            source: Some(self.source_record("healthy")),
            ..ImportBatch::default()
        };
        self.import_request_logs(&connection, cursors.get(LOGS_RESOURCE_ID), &mut batch)?;
        Ok(batch)
    }

    fn source_record(&self, status: &str) -> SourceRecord {
        let timestamp = now();
        SourceRecord {
            id: SOURCE_ID.to_owned(),
            adapter_type: ADAPTER_TYPE.to_owned(),
            display_name: DISPLAY_NAME.to_owned(),
            path_or_endpoint: Some(self.db_path.to_string_lossy().into_owned()),
            enabled: true,
            detected_version: Some("sqlite".to_owned()),
            health_status: Some(status.to_owned()),
            last_success_at: (status == "healthy").then_some(timestamp),
            last_error: None,
            created_at: timestamp,
            updated_at: timestamp,
        }
    }

    fn import_request_logs(
        &self,
        connection: &Connection,
        cursor: Option<&ImportCursor>,
        batch: &mut ImportBatch,
    ) -> Result<(), CockpitAdapterError> {
        if !table_exists(connection, "request_logs")? {
            return Ok(());
        }
        let columns = column_set(connection, "request_logs")?;
        // `event_key` (unique) is the dedup identity; `timestamp` drives the
        // incremental cursor. Neither can be missing.
        if !columns.contains("event_key") || !columns.contains("timestamp") {
            return Ok(());
        }
        let since = cursor.map_or(0, |value| value.byte_offset.max(0));

        let mut statement = connection
            .prepare("SELECT * FROM request_logs WHERE timestamp >= ?1 ORDER BY timestamp ASC")?;
        let names = column_names(&statement);
        let mut referenced_providers = HashSet::<String>::new();
        let mut max_timestamp = since;
        let mut skipped = 0_usize;

        let mut rows = statement.query([since])?;
        while let Some(row) = rows.next()? {
            let Some(event_key) = string_col(row, &names, "event_key") else {
                skipped += 1;
                continue;
            };
            let timestamp = int_col(row, &names, "timestamp").unwrap_or(0);
            let Some(occurred_at) = epoch_to_utc(timestamp) else {
                skipped += 1;
                continue;
            };
            max_timestamp = max_timestamp.max(timestamp);

            let gateway_mode = string_col(row, &names, "gateway_mode")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "cockpit".to_owned());
            let provider_id = format!("{SOURCE_ID}:{gateway_mode}");
            referenced_providers.insert(gateway_mode.clone());

            let account_id = string_col(row, &names, "account_id")
                .filter(|value| !value.is_empty())
                .map(|value| format!("{SOURCE_ID}:{value}"));
            let request_id =
                string_col(row, &names, "request_id").filter(|value| !value.is_empty());
            let model = string_col(row, &names, "model_id").filter(|value| !value.is_empty());
            let status_code = int_col(row, &names, "http_status");
            let success = int_col(row, &names, "success").map(|value| value != 0);
            let latency = int_col(row, &names, "latency_ms");
            let cost = float_col(row, &names, "estimated_cost_usd")
                .filter(|value| value.is_finite() && *value >= 0.0);

            let raw_event_hash = hash_parts([SOURCE_ID, "identity", event_key.as_str()]);
            batch.usage_events.push(UsageEvent {
                id: raw_event_hash.clone(),
                occurred_at,
                app: AppKind::Codex,
                launcher: LauncherKind::Cockpit,
                ingest_source: IngestSource::Proxy,
                source_id: SOURCE_ID.to_owned(),
                provider_id: Some(provider_id),
                account_id,
                session_id: None,
                parent_session_id: None,
                request_id,
                response_id: None,
                model,
                query_source: Some(gateway_mode),
                usage: NormalizedUsage {
                    input_tokens_total: int_col(row, &names, "input_tokens").map(cast_u64),
                    input_tokens_uncached: uncached_input(row, &names),
                    cache_read_tokens: int_col(row, &names, "cached_tokens").map(cast_u64),
                    cache_write_tokens: None,
                    output_tokens_total: int_col(row, &names, "output_tokens").map(cast_u64),
                    reasoning_tokens: int_col(row, &names, "reasoning_tokens").map(cast_u64),
                    visible_output_tokens: visible_output(row, &names),
                },
                provider_reported_cost: None,
                estimated_cost: cost,
                currency: cost.map(|_| "USD".to_owned()),
                http_status: status_code,
                latency_ms: latency,
                success,
                // Exact per-request token counts measured by the local proxy, but
                // the provider is inferred from the gateway mode and there is no
                // session grouping — so those axes are Correlated, not Verified.
                precision_token: PrecisionLevel::ExactSession,
                precision_session: PrecisionLevel::Correlated,
                precision_provider: PrecisionLevel::Correlated,
                precision_account: PrecisionLevel::ExactSession,
                raw_event_hash,
                raw_usage_json: Some(raw_usage_json(row, &names)),
            });
        }

        for gateway_mode in referenced_providers {
            batch.providers.push(ProviderRecord {
                id: format!("{SOURCE_ID}:{gateway_mode}"),
                provider_family: SOURCE_ID.to_owned(),
                display_name: format!("Cockpit · {gateway_mode}"),
                upstream_url: None,
                launcher: Some(LauncherKind::Cockpit),
                source_id: Some(SOURCE_ID.to_owned()),
            });
        }

        batch.skipped_records += skipped;
        batch.cursors.push(ImportCursor {
            source_id: SOURCE_ID.to_owned(),
            resource_id: LOGS_RESOURCE_ID.to_owned(),
            file_size: None,
            modified_at: Some(now()),
            byte_offset: max_timestamp,
            content_hash: None,
            last_cumulative_usage: None,
            snapshot_generation: 0,
            last_session_id: None,
            updated_at: now(),
        });
        Ok(())
    }
}

fn resolve_db_path(path: PathBuf) -> PathBuf {
    if path.is_file() {
        return path;
    }
    if path.is_dir() {
        // Accept either the ~/.antigravity_cockpit directory or its parent.
        let direct = path.join(DB_FILENAME);
        if direct.is_file() {
            return direct;
        }
        let nested = path.join(HOME_DIRNAME).join(DB_FILENAME);
        if nested.is_file() {
            return nested;
        }
        return direct;
    }
    path
}

pub fn default_cockpit_db() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.map(|home| PathBuf::from(home).join(HOME_DIRNAME).join(DB_FILENAME))
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, CockpitAdapterError> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn column_set(
    connection: &Connection,
    table: &str,
) -> Result<HashSet<String>, CockpitAdapterError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    let mut columns = HashSet::new();
    for name in rows {
        columns.insert(name?);
    }
    Ok(columns)
}

fn column_names(statement: &rusqlite::Statement<'_>) -> HashMap<String, usize> {
    statement
        .column_names()
        .into_iter()
        .enumerate()
        .map(|(index, name)| (name.to_owned(), index))
        .collect()
}

fn string_col(row: &Row<'_>, names: &HashMap<String, usize>, name: &str) -> Option<String> {
    let index = *names.get(name)?;
    row.get::<_, Option<String>>(index).ok().flatten()
}

fn int_col(row: &Row<'_>, names: &HashMap<String, usize>, name: &str) -> Option<i64> {
    let index = *names.get(name)?;
    row.get::<_, Option<i64>>(index).ok().flatten()
}

fn float_col(row: &Row<'_>, names: &HashMap<String, usize>, name: &str) -> Option<f64> {
    let index = *names.get(name)?;
    row.get::<_, Option<f64>>(index).ok().flatten()
}

fn uncached_input(row: &Row<'_>, names: &HashMap<String, usize>) -> Option<u64> {
    let input = int_col(row, names, "input_tokens")?;
    let cached = int_col(row, names, "cached_tokens").unwrap_or(0);
    (input >= cached).then(|| cast_u64(input - cached))
}

fn visible_output(row: &Row<'_>, names: &HashMap<String, usize>) -> Option<u64> {
    let output = int_col(row, names, "output_tokens")?;
    let reasoning = int_col(row, names, "reasoning_tokens").unwrap_or(0);
    (output >= reasoning).then(|| cast_u64(output - reasoning))
}

fn cast_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn epoch_to_utc(value: i64) -> Option<DateTime<Utc>> {
    if value <= 0 {
        return None;
    }
    if value > 100_000_000_000 {
        Utc.timestamp_millis_opt(value).single()
    } else {
        Utc.timestamp_opt(value, 0).single()
    }
}

fn raw_usage_json(row: &Row<'_>, names: &HashMap<String, usize>) -> serde_json::Value {
    // Only non-sensitive accounting fields; never prompts, emails, or tokens.
    serde_json::json!({
        "input_tokens": int_col(row, names, "input_tokens"),
        "output_tokens": int_col(row, names, "output_tokens"),
        "cached_tokens": int_col(row, names, "cached_tokens"),
        "reasoning_tokens": int_col(row, names, "reasoning_tokens"),
        "total_tokens": int_col(row, names, "total_tokens"),
        "estimated_cost_usd": float_col(row, names, "estimated_cost_usd"),
        "model": string_col(row, names, "model_id"),
        "http_status": int_col(row, names, "http_status"),
        "gateway_mode": string_col(row, names, "gateway_mode"),
    })
}

fn hash_parts<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

fn now() -> DateTime<Utc> {
    DateTime::<Utc>::from(SystemTime::now())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rusqlite::Connection;
    use tokenbuddy_domain::{AppKind, IngestSource, LauncherKind, PrecisionLevel};

    use super::{CockpitAdapter, LOGS_RESOURCE_ID};

    // Mirrors the real `request_logs` schema verified from a live Cockpit DB.
    fn write_fixture(rows: bool) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(super::DB_FILENAME);
        let connection = Connection::open(&path).expect("open fixture");
        connection
            .execute_batch(
                "CREATE TABLE request_logs (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     event_key TEXT NOT NULL UNIQUE,
                     timestamp INTEGER NOT NULL,
                     request_id TEXT NOT NULL DEFAULT '',
                     account_id TEXT NOT NULL DEFAULT '',
                     email TEXT NOT NULL DEFAULT '',
                     api_key_label TEXT NOT NULL DEFAULT '',
                     model_id TEXT NOT NULL DEFAULT '',
                     gateway_mode TEXT NOT NULL DEFAULT '',
                     service_tier TEXT NOT NULL DEFAULT '',
                     success INTEGER NOT NULL DEFAULT 0,
                     http_status INTEGER,
                     latency_ms INTEGER NOT NULL DEFAULT 0,
                     input_tokens INTEGER NOT NULL DEFAULT 0,
                     output_tokens INTEGER NOT NULL DEFAULT 0,
                     total_tokens INTEGER NOT NULL DEFAULT 0,
                     cached_tokens INTEGER NOT NULL DEFAULT 0,
                     reasoning_tokens INTEGER NOT NULL DEFAULT 0,
                     estimated_cost_usd REAL NOT NULL DEFAULT 0
                 );",
            )
            .expect("create request_logs");
        if rows {
            connection
                .execute_batch(
                    "INSERT INTO request_logs
                         (event_key, timestamp, request_id, account_id, email, model_id,
                          gateway_mode, service_tier, success, http_status, latency_ms,
                          input_tokens, output_tokens, total_tokens, cached_tokens,
                          reasoning_tokens, estimated_cost_usd) VALUES
                         ('evt-1', 1785000000, 'req-1', 'codex_abc', 'user@example.com', 'gpt-5-codex',
                          'proxy', 'default', 1, 200, 350, 1200, 300, 1500, 400, 80, 0.0234),
                         ('evt-2', 1785000100, 'req-2', 'codex_abc', 'user@example.com', 'gpt-5-codex',
                          'proxy', 'default', 0, 429, 90, 500, 0, 500, 0, 0, 0.0);",
                )
                .expect("seed rows");
        }
        drop(connection);
        dir
    }

    #[test]
    fn imports_request_logs_with_cost_and_provider_context() {
        let dir = write_fixture(true);
        let adapter = CockpitAdapter::new(dir.path().join(super::DB_FILENAME));
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import");

        assert_eq!(batch.usage_events.len(), 2);
        let event = &batch.usage_events[0];
        assert_eq!(event.app, AppKind::Codex);
        assert_eq!(event.launcher, LauncherKind::Cockpit);
        assert_eq!(event.ingest_source, IngestSource::Proxy);
        assert_eq!(event.request_id.as_deref(), Some("req-1"));
        assert_eq!(event.provider_id.as_deref(), Some("cockpit:proxy"));
        assert_eq!(event.account_id.as_deref(), Some("cockpit:codex_abc"));
        assert_eq!(event.usage.input_tokens_total, Some(1200));
        assert_eq!(event.usage.cache_read_tokens, Some(400));
        assert_eq!(event.usage.input_tokens_uncached, Some(800));
        assert_eq!(event.usage.output_tokens_total, Some(300));
        assert_eq!(event.usage.visible_output_tokens, Some(220));
        assert_eq!(event.estimated_cost, Some(0.0234));
        assert_eq!(event.currency.as_deref(), Some("USD"));
        assert_eq!(event.http_status, Some(200));
        assert_eq!(event.success, Some(true));
        assert_eq!(event.precision_token, PrecisionLevel::ExactSession);
        assert_eq!(event.precision_provider, PrecisionLevel::Correlated);

        let provider = batch
            .providers
            .iter()
            .find(|provider| provider.id == "cockpit:proxy")
            .expect("provider record");
        assert_eq!(provider.display_name, "Cockpit · proxy");

        // A failed request keeps its status without inventing token totals.
        let failed = &batch.usage_events[1];
        assert_eq!(failed.success, Some(false));
        assert_eq!(failed.http_status, Some(429));
    }

    #[test]
    fn incremental_cursor_advances_by_timestamp() {
        let dir = write_fixture(true);
        let adapter = CockpitAdapter::new(dir.path().join(super::DB_FILENAME));
        let first = adapter.import_history_sync(&HashMap::new()).expect("first");
        let cursors: HashMap<_, _> = first
            .cursors
            .iter()
            .map(|cursor| (cursor.resource_id.clone(), cursor.clone()))
            .collect();
        assert_eq!(
            cursors.get(LOGS_RESOURCE_ID).unwrap().byte_offset,
            1785000100
        );
        let second = adapter.import_history_sync(&cursors).expect("second");
        // Only the boundary row is revisited; event_key dedup collapses it.
        assert!(second.usage_events.len() <= 1);
    }

    #[test]
    fn empty_request_log_table_is_not_an_error() {
        let dir = write_fixture(false);
        let adapter = CockpitAdapter::new(dir.path().join(super::DB_FILENAME));
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import");
        assert!(batch.usage_events.is_empty());
        assert_eq!(
            batch.source.as_ref().unwrap().health_status.as_deref(),
            Some("healthy")
        );
    }

    #[test]
    fn missing_database_reports_not_found() {
        let adapter = CockpitAdapter::new("/nonexistent/cockpit.sqlite");
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import");
        assert!(batch.usage_events.is_empty());
        assert_eq!(
            batch.source.as_ref().unwrap().health_status.as_deref(),
            Some("not_found")
        );
    }

    // Opt-in smoke test against a real Cockpit database. Run with:
    //   COCKPIT_REAL_DB=~/.antigravity_cockpit/codex_local_access_logs.sqlite \
    //     cargo test -p tokenbuddy-cockpit -- --ignored --nocapture real_database
    #[test]
    #[ignore]
    fn real_database_parses_without_panicking() {
        let Some(path) = std::env::var_os("COCKPIT_REAL_DB") else {
            return;
        };
        let adapter = CockpitAdapter::new(path);
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("real import");
        println!(
            "real Cockpit import: {} events, {} providers, {} skipped",
            batch.usage_events.len(),
            batch.providers.len(),
            batch.skipped_records
        );
        assert!(
            batch
                .usage_events
                .iter()
                .all(|event| event.launcher == LauncherKind::Cockpit)
        );
    }
}
