//! Read-only CC-Switch adapter.
//!
//! CC-Switch keeps a SQLite database at `~/.cc-switch/cc-switch.db`. TokenBuddy
//! opens it read-only, probes `sqlite_master` before touching any table, and
//! maps two things into the shared domain model:
//!
//! - `providers` + `provider_endpoints` → real provider names and upstream URLs
//!   (the Providers view no longer has to guess a provider from a model name).
//! - `proxy_request_logs` rows that CC-Switch measured *through its own proxy*
//!   → request-level usage events with real cost, latency, and status.
//!
//! Crucially, CC-Switch also re-derives usage from the same `~/.codex` and
//! `~/.claude` session logs that TokenBuddy imports directly (`data_source` of
//! `codex_session` / `session_log`). Those rows are skipped so the two paths do
//! not double-count; only genuinely proxy-measured rows become usage events.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    time::SystemTime,
};

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, OpenFlags, Row};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokenbuddy_domain::{
    AppKind, DetectionResult, ImportBatch, ImportCursor, IngestSource, LauncherKind,
    NormalizedUsage, PrecisionLevel, ProviderRecord, SessionRecord, SourceHealth, SourceRecord,
    UsageEvent,
};

pub const SOURCE_ID: &str = "cc-switch";
pub const ADAPTER_TYPE: &str = "cc_switch";
pub const DISPLAY_NAME: &str = "CC-Switch";
pub const DB_FILENAME: &str = "cc-switch.db";
const LOGS_RESOURCE_ID: &str = "proxy_request_logs";

#[derive(Debug, Error)]
pub enum CcSwitchAdapterError {
    #[error("failed to read CC-Switch database: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Debug, Clone)]
pub struct CcSwitchAdapter {
    db_path: PathBuf,
}

impl CcSwitchAdapter {
    /// `path` may be the `cc-switch.db` file itself or the `~/.cc-switch`
    /// directory that contains it.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            db_path: resolve_db_path(path.into()),
        }
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn detect_sync(&self) -> Result<DetectionResult, CcSwitchAdapterError> {
        let detected = self.db_path.is_file();
        Ok(DetectionResult {
            source_id: SOURCE_ID.to_owned(),
            detected,
            path_or_endpoint: Some(self.db_path.to_string_lossy().into_owned()),
            detected_version: detected.then(|| "sqlite".to_owned()),
            message: Some(if detected {
                "CC-Switch database detected".to_owned()
            } else {
                "CC-Switch database was not found".to_owned()
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
    ) -> Result<ImportBatch, CcSwitchAdapterError> {
        if !self.db_path.is_file() {
            return Ok(ImportBatch {
                source: Some(self.source_record("not_found")),
                ..ImportBatch::default()
            });
        }
        let connection = self.open_readonly()?;

        let mut batch = ImportBatch {
            source: Some(self.source_record("healthy")),
            ..ImportBatch::default()
        };

        let providers = self.read_providers(&connection)?;
        self.import_proxy_logs(
            &connection,
            &providers,
            cursors.get(LOGS_RESOURCE_ID),
            &mut batch,
        )?;

        Ok(batch)
    }

    fn open_readonly(&self) -> Result<Connection, CcSwitchAdapterError> {
        // Open strictly read-only so a running CC-Switch is never disturbed.
        Ok(Connection::open_with_flags(
            &self.db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?)
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

    /// Read `providers` (+ `provider_endpoints`) into a lookup keyed by
    /// `(id, app_type)`. Tolerant of a missing table or missing columns.
    fn read_providers(
        &self,
        connection: &Connection,
    ) -> Result<HashMap<(String, String), CcProvider>, CcSwitchAdapterError> {
        let mut providers = HashMap::new();
        if !table_exists(connection, "providers")? {
            return Ok(providers);
        }

        let endpoints = self.read_endpoints(connection)?;
        let mut statement = connection.prepare("SELECT * FROM providers")?;
        let names = column_names(&statement);
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let Some(id) = string_col(row, &names, "id") else {
                continue;
            };
            let app_type = string_col(row, &names, "app_type").unwrap_or_default();
            let name = string_col(row, &names, "name").filter(|value| !value.is_empty());
            let website = string_col(row, &names, "website_url").filter(|value| !value.is_empty());
            let provider_type =
                string_col(row, &names, "provider_type").filter(|value| !value.is_empty());
            let upstream = endpoints
                .get(&(id.clone(), app_type.clone()))
                .cloned()
                .or(website);
            providers.insert(
                (id.clone(), app_type.clone()),
                CcProvider {
                    display_name: name.unwrap_or_else(|| id.clone()),
                    upstream_url: upstream,
                    family: provider_type.unwrap_or_else(|| app_type.clone()),
                },
            );
        }
        Ok(providers)
    }

    fn read_endpoints(
        &self,
        connection: &Connection,
    ) -> Result<HashMap<(String, String), String>, CcSwitchAdapterError> {
        let mut endpoints = HashMap::new();
        if !table_exists(connection, "provider_endpoints")? {
            return Ok(endpoints);
        }
        let mut statement = connection.prepare("SELECT * FROM provider_endpoints")?;
        let names = column_names(&statement);
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let (Some(provider_id), Some(url)) = (
                string_col(row, &names, "provider_id"),
                string_col(row, &names, "url").filter(|value| !value.is_empty()),
            ) else {
                continue;
            };
            let app_type = string_col(row, &names, "app_type").unwrap_or_default();
            // First endpoint wins; CC-Switch keeps the active one first.
            endpoints.entry((provider_id, app_type)).or_insert(url);
        }
        Ok(endpoints)
    }

    fn import_proxy_logs(
        &self,
        connection: &Connection,
        providers: &HashMap<(String, String), CcProvider>,
        cursor: Option<&ImportCursor>,
        batch: &mut ImportBatch,
    ) -> Result<(), CcSwitchAdapterError> {
        if !table_exists(connection, "proxy_request_logs")? {
            return Ok(());
        }
        let columns = column_set(connection, "proxy_request_logs")?;
        // `created_at` and `request_id` are the two fields we cannot do without.
        if !columns.contains("created_at") || !columns.contains("request_id") {
            return Ok(());
        }
        let since = cursor.map_or(0, |value| value.byte_offset.max(0));

        // Only import rows CC-Switch measured through its own proxy. When the
        // `data_source` column is absent (older CC-Switch), every row predates
        // session-log ingestion and is genuinely proxy-measured, so import all.
        let mut sql = String::from("SELECT * FROM proxy_request_logs WHERE created_at >= ?1");
        if columns.contains("data_source") {
            sql.push_str(" AND data_source = 'proxy'");
        }
        sql.push_str(" ORDER BY created_at ASC");

        let mut statement = connection.prepare(&sql)?;
        let names = column_names(&statement);
        let mut sessions = BTreeMap::<String, SessionRecord>::new();
        let mut referenced_providers = HashSet::<(String, String)>::new();
        let mut max_created_at = since;
        let mut skipped = 0_usize;

        let mut rows = statement.query([since])?;
        while let Some(row) = rows.next()? {
            let Some(request_id) = string_col(row, &names, "request_id") else {
                skipped += 1;
                continue;
            };
            let created_at = int_col(row, &names, "created_at").unwrap_or(0);
            let Some(occurred_at) = epoch_to_utc(created_at) else {
                skipped += 1;
                continue;
            };
            max_created_at = max_created_at.max(created_at);

            let app_type = string_col(row, &names, "app_type").unwrap_or_default();
            let app = app_kind(&app_type);
            let provider_key = (
                string_col(row, &names, "provider_id").unwrap_or_default(),
                app_type.clone(),
            );
            referenced_providers.insert(provider_key.clone());
            let provider_id = provider_domain_id(&provider_key);

            let external_session_id =
                string_col(row, &names, "session_id").filter(|value| !value.is_empty());
            let session_id = external_session_id.as_deref().map(session_domain_id);
            if let (Some(external), Some(id)) = (&external_session_id, &session_id) {
                upsert_session(&mut sessions, id, external, app, occurred_at);
            }

            let usage = NormalizedUsage {
                input_tokens_total: int_col(row, &names, "input_tokens").map(cast_u64),
                input_tokens_uncached: uncached_input(row, &names),
                cache_read_tokens: int_col(row, &names, "cache_read_tokens").map(cast_u64),
                cache_write_tokens: int_col(row, &names, "cache_creation_tokens").map(cast_u64),
                output_tokens_total: int_col(row, &names, "output_tokens").map(cast_u64),
                reasoning_tokens: None,
                visible_output_tokens: None,
            };
            let model = string_col(row, &names, "model")
                .or_else(|| string_col(row, &names, "request_model"))
                .filter(|value| !value.is_empty());
            let status_code = int_col(row, &names, "status_code");
            let cost =
                string_col(row, &names, "total_cost_usd").and_then(|value| parse_cost(&value));
            let latency =
                int_col(row, &names, "latency_ms").or_else(|| int_col(row, &names, "duration_ms"));
            // A stable request id makes the hash immune to re-reads.
            let raw_event_hash = hash_parts([SOURCE_ID, "identity", request_id.as_str()]);
            let session_present = session_id.is_some();

            batch.usage_events.push(UsageEvent {
                id: raw_event_hash.clone(),
                occurred_at,
                app,
                launcher: LauncherKind::CCSwitch,
                ingest_source: IngestSource::Proxy,
                source_id: SOURCE_ID.to_owned(),
                provider_id: Some(provider_id),
                account_id: None,
                session_id,
                parent_session_id: None,
                request_id: Some(request_id),
                response_id: None,
                model,
                query_source: Some("cc_switch_proxy".to_owned()),
                usage,
                provider_reported_cost: cost,
                estimated_cost: None,
                currency: cost.map(|_| "USD".to_owned()),
                http_status: status_code,
                latency_ms: latency,
                success: status_code.map(|code| (200..400).contains(&code)),
                // The proxy measured the real request end to end.
                precision_token: PrecisionLevel::Verified,
                precision_session: if session_present {
                    PrecisionLevel::ExactSession
                } else {
                    PrecisionLevel::Correlated
                },
                precision_provider: PrecisionLevel::Verified,
                precision_account: PrecisionLevel::Unavailable,
                raw_event_hash,
                raw_usage_json: Some(raw_usage_json(row, &names)),
            });
        }

        // Emit a provider record for every provider the imported events refer to
        // so the Providers view resolves real names/URLs (and never a dangling id).
        for key in referenced_providers {
            let provider = providers.get(&key);
            batch.providers.push(ProviderRecord {
                id: provider_domain_id(&key),
                provider_family: provider
                    .map_or_else(|| key.1.clone(), |value| value.family.clone()),
                display_name: provider
                    .map_or_else(|| key.0.clone(), |value| value.display_name.clone()),
                upstream_url: provider.and_then(|value| value.upstream_url.clone()),
                launcher: Some(LauncherKind::CCSwitch),
                source_id: Some(SOURCE_ID.to_owned()),
            });
        }

        batch.sessions.extend(sessions.into_values());
        batch.skipped_records += skipped;
        batch.cursors.push(ImportCursor {
            source_id: SOURCE_ID.to_owned(),
            resource_id: LOGS_RESOURCE_ID.to_owned(),
            file_size: None,
            modified_at: Some(now()),
            byte_offset: max_created_at,
            content_hash: None,
            last_cumulative_usage: None,
            snapshot_generation: 0,
            last_session_id: None,
            updated_at: now(),
        });
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct CcProvider {
    display_name: String,
    upstream_url: Option<String>,
    family: String,
}

fn resolve_db_path(path: PathBuf) -> PathBuf {
    if path.is_dir() {
        path.join(DB_FILENAME)
    } else {
        path
    }
}

pub fn default_cc_switch_db() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.map(|home| PathBuf::from(home).join(".cc-switch").join(DB_FILENAME))
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, CcSwitchAdapterError> {
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
) -> Result<HashSet<String>, CcSwitchAdapterError> {
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

fn uncached_input(row: &Row<'_>, names: &HashMap<String, usize>) -> Option<u64> {
    // CC-Switch's `input_tokens` is the total prompt size; the uncached portion
    // is the remainder after cache reads, when both are known and consistent.
    let input = int_col(row, names, "input_tokens")?;
    let cache_read = int_col(row, names, "cache_read_tokens").unwrap_or(0);
    (input >= cache_read).then(|| cast_u64(input - cache_read))
}

fn cast_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn parse_cost(value: &str) -> Option<f64> {
    let cost = value.trim().parse::<f64>().ok()?;
    (cost.is_finite() && cost >= 0.0).then_some(cost)
}

fn epoch_to_utc(value: i64) -> Option<DateTime<Utc>> {
    if value <= 0 {
        return None;
    }
    // CC-Switch stores whole seconds; tolerate a millisecond source too.
    if value > 100_000_000_000 {
        Utc.timestamp_millis_opt(value).single()
    } else {
        Utc.timestamp_opt(value, 0).single()
    }
}

fn app_kind(app_type: &str) -> AppKind {
    match app_type {
        "codex" => AppKind::Codex,
        "claude" | "claude-desktop" => AppKind::ClaudeCode,
        _ => AppKind::Unknown,
    }
}

fn provider_domain_id(key: &(String, String)) -> String {
    format!("{SOURCE_ID}:{}:{}", key.1, key.0)
}

fn session_domain_id(external_session_id: &str) -> String {
    format!("{SOURCE_ID}:{}", short_hash(external_session_id))
}

fn upsert_session(
    sessions: &mut BTreeMap<String, SessionRecord>,
    id: &str,
    external_session_id: &str,
    app: AppKind,
    occurred_at: DateTime<Utc>,
) {
    sessions
        .entry(id.to_owned())
        .and_modify(|session| {
            session.started_at = Some(
                session
                    .started_at
                    .map_or(occurred_at, |current| current.min(occurred_at)),
            );
            session.ended_at = Some(
                session
                    .ended_at
                    .map_or(occurred_at, |current| current.max(occurred_at)),
            );
            session.updated_at = now();
        })
        .or_insert_with(|| SessionRecord {
            id: id.to_owned(),
            external_session_id: Some(external_session_id.to_owned()),
            parent_session_id: None,
            app,
            launcher: Some(LauncherKind::CCSwitch),
            project_path: None,
            title: None,
            started_at: Some(occurred_at),
            ended_at: Some(occurred_at),
            source_id: Some(SOURCE_ID.to_owned()),
            created_at: now(),
            updated_at: now(),
        });
}

fn raw_usage_json(row: &Row<'_>, names: &HashMap<String, usize>) -> serde_json::Value {
    // Only non-sensitive accounting fields; never prompts or credentials.
    serde_json::json!({
        "input_tokens": int_col(row, names, "input_tokens"),
        "output_tokens": int_col(row, names, "output_tokens"),
        "cache_read_tokens": int_col(row, names, "cache_read_tokens"),
        "cache_creation_tokens": int_col(row, names, "cache_creation_tokens"),
        "total_cost_usd": string_col(row, names, "total_cost_usd"),
        "model": string_col(row, names, "model"),
        "status_code": int_col(row, names, "status_code"),
        "data_source": "proxy",
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

fn short_hash(value: &str) -> String {
    hash_parts([value]).chars().take(16).collect()
}

fn now() -> DateTime<Utc> {
    DateTime::<Utc>::from(SystemTime::now())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rusqlite::Connection;
    use tokenbuddy_domain::{AppKind, IngestSource, LauncherKind, PrecisionLevel};

    use super::{CcSwitchAdapter, LOGS_RESOURCE_ID};

    fn write_fixture(with_data_source: bool) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("cc-switch.db");
        let connection = Connection::open(&path).expect("open fixture");
        connection
            .execute_batch(
                "CREATE TABLE providers (
                     id TEXT NOT NULL, app_type TEXT NOT NULL, name TEXT NOT NULL,
                     website_url TEXT, provider_type TEXT, is_current INTEGER DEFAULT 0,
                     PRIMARY KEY (id, app_type));
                 CREATE TABLE provider_endpoints (
                     id INTEGER PRIMARY KEY AUTOINCREMENT, provider_id TEXT NOT NULL,
                     app_type TEXT NOT NULL, url TEXT NOT NULL);
                 CREATE TABLE proxy_request_logs (
                     request_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, app_type TEXT NOT NULL,
                     model TEXT NOT NULL, input_tokens INTEGER NOT NULL DEFAULT 0,
                     output_tokens INTEGER NOT NULL DEFAULT 0, cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                     cache_creation_tokens INTEGER NOT NULL DEFAULT 0, total_cost_usd TEXT NOT NULL DEFAULT '0',
                     latency_ms INTEGER NOT NULL DEFAULT 0, status_code INTEGER NOT NULL DEFAULT 200,
                     session_id TEXT, created_at INTEGER NOT NULL, data_source TEXT NOT NULL DEFAULT 'proxy');
                 INSERT INTO providers (id, app_type, name, website_url, provider_type) VALUES
                     ('prov-1', 'codex', 'DeepSeek', 'https://platform.deepseek.com', 'codex');
                 INSERT INTO provider_endpoints (provider_id, app_type, url) VALUES
                     ('prov-1', 'codex', 'https://api.deepseek.com/anthropic');
                 INSERT INTO proxy_request_logs
                     (request_id, provider_id, app_type, model, input_tokens, output_tokens,
                      cache_read_tokens, cache_creation_tokens, total_cost_usd, latency_ms,
                      status_code, session_id, created_at, data_source) VALUES
                     ('req-1', 'prov-1', 'codex', 'gpt-5-codex', 1000, 200, 300, 50, '0.0123', 420, 200, 'sess-1', 1785000000, 'proxy'),
                     ('req-2', 'prov-1', 'codex', 'gpt-5-codex', 2000, 400, 600, 0, '0.0456', 610, 200, 'sess-1', 1785000100, 'proxy'),
                     ('req-3', 'prov-1', 'codex', 'gpt-5-codex', 9999, 999, 0, 0, '0.9', 100, 200, 'sess-x', 1785000200, 'codex_session');",
            )
            .expect("seed fixture");
        if !with_data_source {
            // Simulate an older CC-Switch schema without a data_source column by
            // rebuilding the table without it.
            connection
                .execute_batch("ALTER TABLE proxy_request_logs DROP COLUMN data_source;")
                .expect("drop data_source");
        }
        drop(connection);
        dir
    }

    #[test]
    fn imports_only_proxy_measured_rows_with_real_provider_context() {
        let dir = write_fixture(true);
        let adapter = CcSwitchAdapter::new(dir.path().join("cc-switch.db"));
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import");

        // The codex_session row is skipped to avoid double-counting the native
        // Codex adapter; only the two proxy rows become events.
        assert_eq!(batch.usage_events.len(), 2);
        let event = &batch.usage_events[0];
        assert_eq!(event.app, AppKind::Codex);
        assert_eq!(event.launcher, LauncherKind::CCSwitch);
        assert_eq!(event.ingest_source, IngestSource::Proxy);
        assert_eq!(event.provider_id.as_deref(), Some("cc-switch:codex:prov-1"));
        assert_eq!(event.request_id.as_deref(), Some("req-1"));
        assert_eq!(event.usage.input_tokens_total, Some(1000));
        assert_eq!(event.usage.cache_read_tokens, Some(300));
        assert_eq!(event.usage.input_tokens_uncached, Some(700));
        assert_eq!(event.provider_reported_cost, Some(0.0123));
        assert_eq!(event.currency.as_deref(), Some("USD"));
        assert_eq!(event.http_status, Some(200));
        assert_eq!(event.precision_token, PrecisionLevel::Verified);

        // Provider context carries the real name + upstream endpoint URL.
        let provider = batch
            .providers
            .iter()
            .find(|provider| provider.id == "cc-switch:codex:prov-1")
            .expect("provider record");
        assert_eq!(provider.display_name, "DeepSeek");
        assert_eq!(
            provider.upstream_url.as_deref(),
            Some("https://api.deepseek.com/anthropic")
        );
        assert_eq!(batch.sessions.len(), 1);
    }

    #[test]
    fn incremental_cursor_skips_already_imported_rows() {
        let dir = write_fixture(true);
        let adapter = CcSwitchAdapter::new(dir.path().join("cc-switch.db"));
        let first = adapter
            .import_history_sync(&HashMap::new())
            .expect("first import");
        let cursors: HashMap<_, _> = first
            .cursors
            .iter()
            .map(|cursor| (cursor.resource_id.clone(), cursor.clone()))
            .collect();
        let cursor = cursors.get(LOGS_RESOURCE_ID).expect("logs cursor");
        assert_eq!(cursor.byte_offset, 1785000100);

        // Re-importing from the cursor only revisits the boundary row, which the
        // request-id hash dedupes downstream.
        let second = adapter
            .import_history_sync(&cursors)
            .expect("second import");
        assert!(second.usage_events.len() <= 1);
    }

    #[test]
    fn older_schema_without_data_source_imports_every_row() {
        let dir = write_fixture(false);
        let adapter = CcSwitchAdapter::new(dir.path().join("cc-switch.db"));
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import");
        // Without the data_source column every row predates session ingestion.
        assert_eq!(batch.usage_events.len(), 3);
    }

    #[test]
    fn missing_database_reports_not_found_without_error() {
        let adapter = CcSwitchAdapter::new("/nonexistent/cc-switch.db");
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import");
        assert!(batch.usage_events.is_empty());
        assert_eq!(
            batch.source.as_ref().unwrap().health_status.as_deref(),
            Some("not_found")
        );
    }

    // Opt-in smoke test against a real CC-Switch database. Run with:
    //   CC_SWITCH_REAL_DB=~/.cc-switch/cc-switch.db cargo test -p tokenbuddy-cc-switch \
    //     -- --ignored --nocapture real_database
    #[test]
    #[ignore]
    fn real_database_parses_without_panicking() {
        let Some(path) = std::env::var_os("CC_SWITCH_REAL_DB") else {
            return;
        };
        let adapter = CcSwitchAdapter::new(path);
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("real import");
        println!(
            "real CC-Switch import: {} proxy events, {} providers, {} sessions, {} skipped",
            batch.usage_events.len(),
            batch.providers.len(),
            batch.sessions.len(),
            batch.skipped_records
        );
        let with_cost = batch
            .usage_events
            .iter()
            .filter(|event| event.provider_reported_cost.is_some())
            .count();
        println!("events with provider-reported cost: {with_cost}");
        assert!(batch.usage_events.iter().all(|event| {
            event.request_id.is_some() && event.launcher == LauncherKind::CCSwitch
        }));
    }
}
