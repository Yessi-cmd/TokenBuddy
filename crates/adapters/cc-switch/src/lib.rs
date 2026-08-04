//! Read-only CC-Switch adapter — a *provider attribution* source, not a token
//! source.
//!
//! CC-Switch keeps a SQLite database at `~/.cc-switch/cc-switch.db`. TokenBuddy
//! opens it read-only, probes `sqlite_master` before touching any table, and
//! maps two things into the shared domain model:
//!
//! - `providers` + `provider_endpoints` → real provider names and upstream URLs.
//! - `proxy_request_logs` → which provider actually served each session.
//!
//! It deliberately emits **no usage events**. CC-Switch proxies the very
//! requests that Codex/Claude Code also record in their own transcripts, so
//! importing its rows as events would count the same API call twice (verified on
//! real data: every proxied session_id resolves to an existing
//! `~/.claude/projects/*.jsonl` transcript). Spec §6.1 ranks session logs above
//! proxy logs as the token source, and §10.1 forbids treating CC-Switch as the
//! sole source — so its unique contribution is telling us *who served the
//! request*, which a session log never records. That fixes attribution the model
//! name cannot: `deepseek-v4-pro` reached through a Claude-compatible relay is
//! DeepSeek, not Anthropic.
#![warn(missing_docs)]

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    time::SystemTime,
};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokenbuddy_domain::{
    AdapterCapabilities, AdapterDescriptor, DetectionResult, ImportBatch, ImportCursor,
    LauncherKind, ProviderRecord, SessionProviderAttribution, SourceHealth, SourceRecord,
};
use tokenbuddy_sqlite_source::{
    column_names, column_set, epoch_to_utc, int_col, open_read_only, string_col, table_exists,
};

/// Stable id of this source.
pub const SOURCE_ID: &str = "cc-switch";
/// Adapter kind recorded on the source row.
pub const ADAPTER_TYPE: &str = "cc_switch";
/// Name shown in the UI.
pub const DISPLAY_NAME: &str = "CC-Switch";
/// Static capabilities advertised to the Core registry.
pub const DESCRIPTOR: AdapterDescriptor = AdapterDescriptor {
    id: SOURCE_ID,
    adapter_type: ADAPTER_TYPE,
    display_name: DISPLAY_NAME,
    capabilities: AdapterCapabilities {
        usage_events: false,
        provider_context: true,
        quota_snapshots: false,
        file_watch: false,
    },
    read_only: true,
};
/// CC-Switch's database file name.
pub const DB_FILENAME: &str = "cc-switch.db";
const LOGS_RESOURCE_ID: &str = "proxy_request_logs";

/// Why reading the CC-Switch database failed.
#[derive(Debug, Error)]
pub enum CcSwitchAdapterError {
    /// The database could not be opened or queried.
    #[error("failed to read CC-Switch database: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Debug, Clone)]
/// Reads CC-Switch's database for provider identity and session attribution.
///
/// Emits no usage events: CC-Switch proxies requests the session logs already
/// record, and those rank higher as a token source.
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

    /// The database this adapter reads.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Whether the configured database is present.
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

    /// Current health of this source.
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

    /// Read providers, endpoints, and proxied requests since `cursors`.
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
        Ok(open_read_only(&self.db_path)?)
    }

    fn source_record(&self, status: &str) -> SourceRecord {
        let timestamp = now();
        SourceRecord {
            id: DESCRIPTOR.id.to_owned(),
            adapter_type: DESCRIPTOR.adapter_type.to_owned(),
            display_name: DESCRIPTOR.display_name.to_owned(),
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
        let mut referenced_providers = HashSet::<(String, String)>::new();
        let mut attributions = BTreeMap::<String, String>::new();
        let mut max_created_at = since;
        let mut skipped = 0_usize;

        let mut rows = statement.query([since])?;
        while let Some(row) = rows.next()? {
            let created_at = int_col(row, &names, "created_at").unwrap_or(0);
            if epoch_to_utc(created_at).is_none() {
                skipped += 1;
                continue;
            }
            max_created_at = max_created_at.max(created_at);

            let app_type = string_col(row, &names, "app_type").unwrap_or_default();
            let provider_key = (
                string_col(row, &names, "provider_id").unwrap_or_default(),
                app_type.clone(),
            );

            // Correlate to the session the *native* adapter already imported. The
            // proxy row's session_id is the Codex/Claude session UUID, so hashing
            // it the way that adapter does lands the attribution on its events.
            let Some(external_session_id) =
                string_col(row, &names, "session_id").filter(|value| !value.is_empty())
            else {
                continue;
            };
            let Some(session_id) = native_session_domain_id(&app_type, &external_session_id) else {
                continue;
            };

            referenced_providers.insert(provider_key.clone());
            // Last write wins: the provider in force at the end of the window is
            // the one the session is attributed to.
            attributions.insert(session_id, provider_domain_id(&provider_key));
        }

        // Emit a provider record for every provider referenced by an attribution
        // so the Providers view resolves real names/URLs (never a dangling id).
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

        for (session_id, provider_id) in attributions {
            batch.attributions.push(SessionProviderAttribution {
                session_id,
                provider_id,
                account_id: None,
                source_id: SOURCE_ID.to_owned(),
            });
        }
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
            last_model: None,
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

/// The platform's default CC-Switch database location.
pub fn default_cc_switch_db() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.map(|home| PathBuf::from(home).join(".cc-switch").join(DB_FILENAME))
}

fn provider_domain_id(key: &(String, String)) -> String {
    format!("{SOURCE_ID}:{}:{}", key.1, key.0)
}

/// Mint the session id exactly as the native session adapter does, so an
/// attribution lands on the rows that adapter imported. Both adapters use
/// `"{SOURCE_ID}:{short_hash(external_session_id)}"`; the app type selects which
/// source id to use. Returns `None` for app types TokenBuddy has no native
/// adapter for, since nothing could be attributed.
fn native_session_domain_id(app_type: &str, external_session_id: &str) -> Option<String> {
    let source_id = match app_type {
        "claude" => "claude-code-session",
        "codex" => "codex-session",
        _ => return None,
    };
    Some(format!("{source_id}:{}", short_hash(external_session_id)))
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
    use tokenbuddy_domain::LauncherKind;

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
    fn never_emits_usage_events_so_proxied_calls_are_not_double_counted() {
        let dir = write_fixture(true);
        let adapter = CcSwitchAdapter::new(dir.path().join("cc-switch.db"));
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import");

        // The proxied requests are the same API calls the native Codex/Claude
        // adapters already import from their transcripts. CC-Switch contributes
        // attribution only — never tokens.
        assert!(batch.usage_events.is_empty());
        assert!(batch.sessions.is_empty());
    }

    #[test]
    fn attributes_sessions_to_the_real_provider_using_native_session_ids() {
        let dir = write_fixture(true);
        let adapter = CcSwitchAdapter::new(dir.path().join("cc-switch.db"));
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import");

        // Only the proxied session is attributed; the codex_session row is
        // CC-Switch re-reading a transcript and carries no routing truth.
        assert_eq!(batch.attributions.len(), 1);
        let attribution = &batch.attributions[0];
        assert_eq!(attribution.provider_id, "cc-switch:codex:prov-1");
        assert_eq!(attribution.source_id, "cc-switch");
        // The id must match what the native Codex adapter mints for "sess-1".
        assert_eq!(
            attribution.session_id,
            format!("codex-session:{}", super::short_hash("sess-1"))
        );

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
        assert_eq!(provider.launcher, Some(LauncherKind::CCSwitch));
    }

    #[test]
    fn native_session_ids_match_each_adapters_scheme() {
        assert_eq!(
            super::native_session_domain_id("claude", "abc").as_deref(),
            Some(format!("claude-code-session:{}", super::short_hash("abc")).as_str())
        );
        assert_eq!(
            super::native_session_domain_id("codex", "abc").as_deref(),
            Some(format!("codex-session:{}", super::short_hash("abc")).as_str())
        );
        // No native adapter exists for these, so nothing can be attributed.
        assert!(super::native_session_domain_id("gemini", "abc").is_none());
        assert!(super::native_session_domain_id("claude-desktop", "abc").is_none());
    }

    #[test]
    fn incremental_cursor_advances_past_imported_rows() {
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

        // Re-attributing is idempotent: the attribution upsert is keyed by
        // session, so revisiting the boundary row changes nothing.
        let second = adapter
            .import_history_sync(&cursors)
            .expect("second import");
        assert!(second.usage_events.is_empty());
    }

    #[test]
    fn older_schema_without_data_source_still_attributes() {
        let dir = write_fixture(false);
        let adapter = CcSwitchAdapter::new(dir.path().join("cc-switch.db"));
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import");
        assert!(batch.usage_events.is_empty());
        // Without the column every row is treated as proxy-measured, so both
        // sessions get attributed.
        assert_eq!(batch.attributions.len(), 2);
    }

    #[test]
    fn missing_database_reports_not_found_without_error() {
        let adapter = CcSwitchAdapter::new("/nonexistent/cc-switch.db");
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import");
        assert!(batch.usage_events.is_empty());
        assert!(batch.attributions.is_empty());
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
            "real CC-Switch import: {} usage events, {} attributions, {} providers, {} skipped",
            batch.usage_events.len(),
            batch.attributions.len(),
            batch.providers.len(),
            batch.skipped_records
        );
        for attribution in &batch.attributions {
            println!(
                "  attribute {} -> {}",
                attribution.session_id, attribution.provider_id
            );
        }
        // The whole point of the fix: no tokens come from CC-Switch.
        assert!(batch.usage_events.is_empty());
    }
}
