//! Read-only Cockpit Tools adapter — a *provider and account context* source,
//! not a token source.
//!
//! Cockpit Tools (the open-source `jlcodes99/cockpit-tools`) records the Codex
//! requests it routes through its local access proxy into a SQLite database at
//! `~/.antigravity_cockpit/codex_local_access_logs.sqlite`, table `request_logs`.
//! That table is the spec's "read-only database" fallback (§11.1) and is the
//! only non-sensitive, request-level surface — the WebSocket exposes account
//! aliases but no usage, and the `/report` quota API is off by default.
//!
//! Those requests are the same ones Codex records in its own rollout logs, which
//! TokenBuddy imports directly and which spec §6.1 ranks above proxy logs. So
//! this adapter emits **no usage events** — counting them here would double the
//! tokens. Unlike CC-Switch, `request_logs` carries no session id, so per-session
//! attribution is impossible; per spec §11.3 Cockpit therefore contributes
//! provider and account context only. Credentials and the account store are
//! never touched.
#![warn(missing_docs)]

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    time::SystemTime,
};

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use thiserror::Error;
use tokenbuddy_domain::{
    AccountActivityWindow, AccountRecord, AdapterCapabilities, AdapterDescriptor, AppKind,
    DetectionResult, ImportBatch, ImportCursor, LauncherKind, ProviderRecord, SourceHealth,
    SourceRecord, account_fingerprint,
};
use tokenbuddy_sqlite_source::{
    column_names, column_set, epoch_to_utc, int_col, open_read_only, string_col, table_exists,
};

/// Stable id of this source.
pub const SOURCE_ID: &str = "cockpit";
/// Adapter kind recorded on the source row.
pub const ADAPTER_TYPE: &str = "cockpit";
/// Name shown in the UI.
pub const DISPLAY_NAME: &str = "Cockpit Tools";
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
/// Cockpit's request-log database file name.
pub const DB_FILENAME: &str = "codex_local_access_logs.sqlite";
/// Cockpit's data directory inside the user's home.
pub const HOME_DIRNAME: &str = ".antigravity_cockpit";
const LOGS_RESOURCE_ID: &str = "request_logs";
/// Cockpit's accounts are ChatGPT accounts; the quota and the plan belong to
/// OpenAI, not to the launcher that routed the request.
const UPSTREAM_PROVIDER_ID: &str = "openai";
const UPSTREAM_PROVIDER_DISPLAY_NAME: &str = "OpenAI";
const AUTH_MODE: &str = "cockpit";
/// A silence longer than this ends an activity window: Cockpit may have
/// switched accounts, and a window spanning the switch would claim requests the
/// other account served.
const WINDOW_GAP_SECONDS: i64 = 30 * 60;
/// The proxy and the Codex rollout log timestamp the same request a moment
/// apart, so windows are padded before matching.
const WINDOW_PADDING_SECONDS: i64 = 60;

/// Why reading the Cockpit database failed.
#[derive(Debug, Error)]
pub enum CockpitAdapterError {
    /// The database could not be opened or queried.
    #[error("failed to read Cockpit database: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Debug, Clone)]
/// Reads Cockpit's request log for account identity and activity windows.
///
/// Emits no usage events: Codex already logged these same requests, and its own
/// log ranks higher as a token source.
pub struct CockpitAdapter {
    db_path: PathBuf,
    fingerprint_salt: Option<String>,
}

impl CockpitAdapter {
    /// `path` may be the request-log SQLite file, the `~/.antigravity_cockpit`
    /// directory, or its parent — whatever the user pointed the setting at.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            db_path: resolve_db_path(path.into()),
            fingerprint_salt: None,
        }
    }

    /// Supply the per-install salt used to fingerprint account ids (spec §20.2).
    /// Without it the adapter reports no accounts — Cockpit's account ids are
    /// stored hashed or not at all.
    #[must_use]
    pub fn with_fingerprint_salt(mut self, salt: impl Into<String>) -> Self {
        self.fingerprint_salt = Some(salt.into());
        self
    }

    /// The database this adapter reads.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Whether the configured database is present.
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

    /// Read request-log rows since `cursors`, producing accounts and windows.
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
        // Read-only: a running Cockpit must never see TokenBuddy in its data.
        let connection = open_read_only(&self.db_path)?;

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

    /// Turn each account's request instants into one account row plus the
    /// windows it was active in.
    ///
    /// Requests closer together than [`WINDOW_GAP_SECONDS`] belong to the same
    /// window; a longer silence ends it, because Cockpit may have switched
    /// accounts in the meantime and a window that spans the switch would claim
    /// another account's requests. Each window is padded by
    /// [`WINDOW_PADDING_SECONDS`] so the Codex log's own timestamp — written at
    /// a slightly different moment than the proxy's — still falls inside.
    fn push_accounts_and_windows(
        &self,
        account_activity: BTreeMap<String, AccountActivity>,
        batch: &mut ImportBatch,
    ) {
        let Some(salt) = self.fingerprint_salt.as_deref() else {
            return;
        };
        if account_activity.is_empty() {
            return;
        }

        batch.providers.push(ProviderRecord {
            id: UPSTREAM_PROVIDER_ID.to_owned(),
            provider_family: UPSTREAM_PROVIDER_ID.to_owned(),
            display_name: UPSTREAM_PROVIDER_DISPLAY_NAME.to_owned(),
            upstream_url: None,
            launcher: Some(LauncherKind::Cockpit),
            source_id: Some(SOURCE_ID.to_owned()),
        });

        for (account_key, mut activity) in account_activity {
            let fingerprint = account_fingerprint(salt, &account_key);
            let account_id = format!("{SOURCE_ID}:{}", &fingerprint[..16]);
            batch.accounts.push(AccountRecord {
                id: account_id.clone(),
                provider_id: UPSTREAM_PROVIDER_ID.to_owned(),
                display_name: Some(
                    activity
                        .label
                        .clone()
                        .unwrap_or_else(|| format!("Cockpit 账号 · {}", &fingerprint[..8])),
                ),
                account_fingerprint: fingerprint,
                auth_mode: AUTH_MODE.to_owned(),
                // Cockpit's request log does not state a subscription plan.
                plan: None,
            });

            activity.instants.sort_unstable();
            let mut window: Option<(DateTime<Utc>, DateTime<Utc>)> = None;
            for instant in activity.instants {
                window = match window {
                    Some((start, end))
                        if instant.signed_duration_since(end).num_seconds()
                            <= WINDOW_GAP_SECONDS =>
                    {
                        Some((start, instant))
                    }
                    Some((start, end)) => {
                        push_window(batch, &account_id, start, end);
                        Some((instant, instant))
                    }
                    None => Some((instant, instant)),
                };
            }
            if let Some((start, end)) = window {
                push_window(batch, &account_id, start, end);
            }
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

        // Per account: the label to show and every instant it served a request.
        // Those instants become the activity windows that let a Codex usage
        // event find its account by time.
        let mut account_activity = BTreeMap::<String, AccountActivity>::new();

        let mut rows = statement.query([since])?;
        while let Some(row) = rows.next()? {
            if string_col(row, &names, "event_key").is_none() {
                skipped += 1;
                continue;
            }
            let timestamp = int_col(row, &names, "timestamp").unwrap_or(0);
            let Some(occurred_at) = epoch_to_utc(timestamp) else {
                skipped += 1;
                continue;
            };
            max_timestamp = max_timestamp.max(timestamp);

            // Record which gateways served traffic. Tokens deliberately stay out
            // of the batch — Codex's own rollout log already counted them.
            let gateway_mode = string_col(row, &names, "gateway_mode")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "cockpit".to_owned());
            referenced_providers.insert(gateway_mode);

            // Cockpit rotates several ChatGPT accounts through one Codex Home,
            // so which account served a request is knowable only here.
            if let Some(account_key) = string_col(row, &names, "account_id")
                .filter(|value| !value.is_empty())
                .or_else(|| string_col(row, &names, "email").filter(|value| !value.is_empty()))
            {
                let activity = account_activity.entry(account_key).or_default();
                if activity.label.is_none() {
                    activity.label =
                        string_col(row, &names, "email").filter(|value| !value.is_empty());
                }
                activity.instants.push(occurred_at);
            }
        }

        self.push_accounts_and_windows(account_activity, batch);

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
            last_model: None,
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

/// The platform's default Cockpit database location.
pub fn default_cockpit_db() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.map(|home| PathBuf::from(home).join(HOME_DIRNAME).join(DB_FILENAME))
}

#[derive(Debug, Default)]
struct AccountActivity {
    label: Option<String>,
    instants: Vec<DateTime<Utc>>,
}

fn push_window(
    batch: &mut ImportBatch,
    account_id: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) {
    let padding = Duration::seconds(WINDOW_PADDING_SECONDS);
    batch.account_windows.push(AccountActivityWindow {
        account_id: account_id.to_owned(),
        source_id: SOURCE_ID.to_owned(),
        // Cockpit routes Codex; it never sits in front of Claude Code.
        app: AppKind::Codex,
        started_at: start - padding,
        ended_at: end + padding,
    });
}

fn now() -> DateTime<Utc> {
    DateTime::<Utc>::from(SystemTime::now())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Duration;
    use rusqlite::Connection;
    use tokenbuddy_domain::{AppKind, LauncherKind};

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
    fn contributes_provider_context_without_emitting_tokens() {
        let dir = write_fixture(true);
        let adapter = CockpitAdapter::new(dir.path().join(super::DB_FILENAME));
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import");

        // Codex already recorded these very requests in its rollout log, so
        // emitting them here would double the tokens.
        assert!(batch.usage_events.is_empty());
        // request_logs has no session id, so per-session attribution is
        // impossible — Cockpit supplies provider context only (spec §11.3).
        assert!(batch.attributions.is_empty());

        let provider = batch
            .providers
            .iter()
            .find(|provider| provider.id == "cockpit:proxy")
            .expect("provider record");
        assert_eq!(provider.display_name, "Cockpit · proxy");
        assert_eq!(provider.launcher, Some(LauncherKind::Cockpit));
    }

    /// Two accounts, each serving a burst of requests, with a long silence
    /// between them — the shape Cockpit produces when it rotates accounts.
    fn write_multi_account_fixture() -> tempfile::TempDir {
        let dir = write_fixture(false);
        let connection =
            Connection::open(dir.path().join(super::DB_FILENAME)).expect("open fixture");
        connection
            .execute_batch(
                "INSERT INTO request_logs
                     (event_key, timestamp, request_id, account_id, email, model_id,
                      gateway_mode, service_tier, success, http_status, latency_ms,
                      input_tokens, output_tokens, total_tokens, cached_tokens,
                      reasoning_tokens, estimated_cost_usd) VALUES
                     ('a-1', 1785000000, 'r1', 'codex_plus', 'plus@example.com', 'gpt-5-codex',
                      'proxy', 'default', 1, 200, 300, 10, 5, 15, 0, 0, 0.01),
                     ('a-2', 1785000300, 'r2', 'codex_plus', 'plus@example.com', 'gpt-5-codex',
                      'proxy', 'default', 1, 200, 300, 10, 5, 15, 0, 0, 0.01),
                     ('b-1', 1785100000, 'r3', 'codex_team', 'team@example.com', 'gpt-5-codex',
                      'proxy', 'default', 1, 200, 300, 10, 5, 15, 0, 0, 0.01);",
            )
            .expect("seed rows");
        dir
    }

    #[test]
    fn rotating_accounts_become_separate_accounts_with_their_own_activity_windows() {
        let dir = write_multi_account_fixture();
        let adapter = CockpitAdapter::new(dir.path().join(super::DB_FILENAME))
            .with_fingerprint_salt("fixture-salt");
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import");

        let mut labels = batch
            .accounts
            .iter()
            .map(|account| account.display_name.clone().expect("label"))
            .collect::<Vec<_>>();
        labels.sort();
        assert_eq!(labels, vec!["plus@example.com", "team@example.com"]);
        for account in &batch.accounts {
            assert_eq!(account.auth_mode, "cockpit");
            assert_eq!(account.provider_id, "openai");
            // Cockpit's raw account id never leaves the adapter.
            assert!(!account.account_fingerprint.contains("codex_"));
            assert!(!account.id.contains("codex_"));
        }

        // Two bursts 300s apart stay one window; the 100_000s gap starts a new
        // one for the other account.
        assert_eq!(batch.account_windows.len(), 2);
        for window in &batch.account_windows {
            assert_eq!(window.app, AppKind::Codex);
            assert_eq!(window.source_id, "cockpit");
            assert!(window.started_at < window.ended_at);
        }
        let plus_account = batch
            .accounts
            .iter()
            .find(|account| account.display_name.as_deref() == Some("plus@example.com"))
            .expect("plus account");
        let plus_window = batch
            .account_windows
            .iter()
            .find(|window| window.account_id == plus_account.id)
            .expect("plus window");
        assert_eq!(
            plus_window.ended_at - plus_window.started_at,
            Duration::seconds(300 + 2 * super::WINDOW_PADDING_SECONDS)
        );
    }

    #[test]
    fn accounts_stay_unavailable_without_a_fingerprint_salt() {
        let dir = write_multi_account_fixture();
        let batch = CockpitAdapter::new(dir.path().join(super::DB_FILENAME))
            .import_history_sync(&HashMap::new())
            .expect("import");

        assert!(batch.accounts.is_empty());
        assert!(batch.account_windows.is_empty());
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
        assert!(second.usage_events.is_empty());
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
        // Cockpit never contributes tokens; Codex's rollout log owns those.
        assert!(batch.usage_events.is_empty());
    }
}
