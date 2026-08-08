//! Read-only OpenCode adapter — a token source backed by OpenCode's SQLite
//! database.
//!
//! OpenCode (https://github.com/anomalyco/opencode) keeps its transcript in
//! `opencode.db` (default `~/.local/share/opencode/opencode.db` on macOS/Linux,
//! `%LOCALAPPDATA%\opencode\opencode.db` on Windows). TokenBuddy opens it
//! strictly read-only, probes `sqlite_master` before touching a table, and
//! maps:
//!
//! - `session` rows → sessions (title, project directory, model, parent).
//! - `part` rows of type `step-finish` → one usage event per model call. Every
//!   step-finish carries the request's own token counts
//!   `{input, output, reasoning, cache: {read, write}}` and OpenCode's computed
//!   `cost`. Verified against real data: the session table's cumulative
//!   counters are exactly the sum of its step-finish parts, so importing parts
//!   never double-counts and there is no separate cumulative snapshot to
//!   difference.
//!
//! Prompt text, tool inputs, reasoning text, and completions live in the same
//! rows but are never read into the domain model. The `model` field is taken
//! from the enclosing message or the session, never from provider data, and no
//! provider or account record is minted: OpenCode's `providerID` names the
//! configured provider plugin, which is not a statement about the real
//! upstream, so provider attribution stays `Unavailable`.
#![warn(missing_docs)]

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    time::SystemTime,
};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokenbuddy_domain::{
    AdapterCapabilities, AdapterDescriptor, AdapterError, AppKind, DetectionResult, ImportBatch,
    ImportCursor, IngestSource, LauncherKind, NormalizedUsage, PrecisionLevel, SessionRecord,
    SourceHealth, SourceRecord, UsageAdapter, UsageEvent,
};
use tokenbuddy_sqlite_source::{
    column_names, epoch_to_utc, int_col, open_read_only, string_col, table_exists,
};

/// Stable id of this source.
pub const SOURCE_ID: &str = "opencode";
/// Adapter kind recorded on the source row.
pub const ADAPTER_TYPE: &str = "opencode";
/// Name shown in the UI.
pub const DISPLAY_NAME: &str = "OpenCode";
/// Static capabilities advertised to the Core registry.
pub const DESCRIPTOR: AdapterDescriptor = AdapterDescriptor {
    id: SOURCE_ID,
    adapter_type: ADAPTER_TYPE,
    display_name: DISPLAY_NAME,
    capabilities: AdapterCapabilities {
        usage_events: true,
        provider_context: false,
        quota_snapshots: false,
        file_watch: false,
    },
    read_only: true,
};
/// OpenCode's database file name.
pub const DB_FILENAME: &str = "opencode.db";
const PARTS_RESOURCE_ID: &str = "parts";

/// Why reading the OpenCode database failed.
#[derive(Debug, Error)]
pub enum OpenCodeAdapterError {
    /// The database could not be opened or queried.
    #[error("failed to read OpenCode database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The file exists but its schema is not a schema TokenBuddy understands.
    #[error("OpenCode 数据库缺少 {0} 表；该数据库不是受支持的 OpenCode 数据目录")]
    SchemaUnsupported(String),
}

#[derive(Debug, Clone)]
/// Reads OpenCode's database for session transcripts and request-level usage.
pub struct OpenCodeAdapter {
    db_path: PathBuf,
}

/// Sessions read from the `session` table, keyed by their domain id, with the
/// model each session runs under (from the session's `model` column).
type SessionMap = BTreeMap<String, (SessionRecord, Option<String>)>;

impl OpenCodeAdapter {
    /// `path` may be the `opencode.db` file itself or the OpenCode data
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
    pub fn detect_sync(&self) -> Result<DetectionResult, OpenCodeAdapterError> {
        let detected = self.db_path.is_file();
        Ok(DetectionResult {
            source_id: SOURCE_ID.to_owned(),
            detected,
            path_or_endpoint: Some(self.db_path.to_string_lossy().into_owned()),
            detected_version: detected.then(|| "sqlite".to_owned()),
            message: Some(if detected {
                "OpenCode 数据库已检测到".to_owned()
            } else {
                "OpenCode 数据库未找到".to_owned()
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

    /// Read sessions and every new request-level usage record since `cursors`.
    ///
    /// The cursor is the high-water `time_created` (millisecond epoch) of the
    /// imported parts. New parts are always appended with a fresh timestamp, so
    /// a repeated call over unchanged input yields no events; the same-ms
    /// bucket is re-read deliberately and the storage layer's stable event
    /// hash makes that idempotent.
    pub fn import_history_sync(
        &self,
        cursors: &HashMap<String, ImportCursor>,
    ) -> Result<ImportBatch, OpenCodeAdapterError> {
        if !self.db_path.is_file() {
            return Ok(ImportBatch {
                source: Some(self.source_record("not_found")),
                ..ImportBatch::default()
            });
        }
        let connection = self.open_readonly()?;
        for table in ["session", "message", "part"] {
            if !table_exists(&connection, table)? {
                return Err(OpenCodeAdapterError::SchemaUnsupported(table.to_owned()));
            }
        }

        let mut batch = ImportBatch {
            source: Some(self.source_record("healthy")),
            ..ImportBatch::default()
        };
        let sessions = self.read_sessions(&connection)?;
        batch
            .sessions
            .extend(sessions.values().map(|(session, _)| session.clone()));
        self.import_parts(
            &connection,
            &sessions,
            cursors.get(PARTS_RESOURCE_ID),
            &mut batch,
        )?;
        Ok(batch)
    }

    fn open_readonly(&self) -> Result<Connection, OpenCodeAdapterError> {
        // Strictly read-only so a running OpenCode TUI is never disturbed.
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

    fn read_sessions(&self, connection: &Connection) -> Result<SessionMap, OpenCodeAdapterError> {
        let mut statement = connection.prepare("SELECT * FROM session")?;
        let names = column_names(&statement);
        let mut sessions = SessionMap::new();
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let Some(external_session_id) = string_col(row, &names, "id") else {
                continue;
            };
            let parent = string_col(row, &names, "parent_id")
                .filter(|value| !value.is_empty())
                .map(|value| self.session_id(&value));
            let started_at = int_col(row, &names, "time_created").and_then(epoch_to_utc);
            let ended_at = int_col(row, &names, "time_updated").and_then(epoch_to_utc);
            let model = string_col(row, &names, "model").and_then(|value| {
                serde_json::from_str::<Value>(&value)
                    .ok()
                    .and_then(|model| model_id(&model))
            });
            let id = self.session_id(&external_session_id);
            let record = SessionRecord {
                id: id.clone(),
                external_session_id: Some(external_session_id),
                parent_session_id: parent,
                app: AppKind::OpenCode,
                launcher: Some(LauncherKind::Direct),
                project_path: string_col(row, &names, "directory")
                    .filter(|value| !value.is_empty()),
                title: string_col(row, &names, "title").filter(|value| !value.is_empty()),
                started_at,
                ended_at,
                source_id: Some(SOURCE_ID.to_owned()),
                created_at: now(),
                updated_at: now(),
            };
            sessions.insert(id, (record, model));
        }
        Ok(sessions)
    }

    fn import_parts(
        &self,
        connection: &Connection,
        sessions: &SessionMap,
        cursor: Option<&ImportCursor>,
        batch: &mut ImportBatch,
    ) -> Result<(), OpenCodeAdapterError> {
        let since = cursor.map_or(0, |value| value.byte_offset.max(0));
        let mut statement = connection.prepare(
            "SELECT p.id, p.session_id, p.time_created, p.data, m.data AS message_data
               FROM part p
               LEFT JOIN message m ON m.id = p.message_id
              WHERE p.time_created >= ?1
              ORDER BY p.time_created ASC",
        )?;
        let names = column_names(&statement);
        let mut rows = statement.query([since])?;
        let mut max_time_created = since;
        let mut skipped = 0_usize;

        while let Some(row) = rows.next()? {
            let time_created = int_col(row, &names, "time_created").unwrap_or(0);
            max_time_created = max_time_created.max(time_created);
            let Some(part_id) = string_col(row, &names, "id") else {
                skipped += 1;
                continue;
            };
            let Some(occurred_at) = epoch_to_utc(time_created) else {
                skipped += 1;
                continue;
            };
            let Some(data) = string_col(row, &names, "data") else {
                skipped += 1;
                continue;
            };
            let Ok(data) = serde_json::from_str::<Value>(&data) else {
                skipped += 1;
                continue;
            };
            if data.get("type").and_then(Value::as_str) != Some("step-finish") {
                continue;
            }
            let Some(tokens) = data.get("tokens") else {
                // A step-finish without a tokens block is not a usage record;
                // it is OpenCode's normal shape for steps the provider never
                // metered, so it is not a skipped record either.
                continue;
            };
            let Some(usage) = step_tokens_to_usage(tokens) else {
                skipped += 1;
                continue;
            };

            let Some(external_session_id) =
                string_col(row, &names, "session_id").filter(|value| !value.is_empty())
            else {
                // A part without a session cannot be attributed; keep the event
                // out of the database rather than invent a session.
                skipped += 1;
                continue;
            };
            let session_id = self.session_id(&external_session_id);
            let session = sessions
                .get(&session_id)
                .map(|(record, model)| (record, model.as_deref()));
            let message_data = string_col(row, &names, "message_data")
                .and_then(|value| serde_json::from_str::<Value>(&value).ok());
            let model = message_data
                .as_ref()
                .and_then(|value| value.get("model"))
                .and_then(model_id)
                .or_else(|| session.and_then(|(_, model)| model.map(str::to_owned)));

            let cost = data.get("cost").and_then(Value::as_f64);
            let latency_ms = step_latency_ms(&data);
            let raw_event_hash = self.event_hash(&part_id);
            let raw_usage_json = Some(serde_json::json!({
                "tokens": tokens,
                "cost": cost,
            }));
            batch.usage_events.push(UsageEvent {
                id: raw_event_hash.clone(),
                occurred_at,
                app: AppKind::OpenCode,
                launcher: LauncherKind::Direct,
                ingest_source: IngestSource::ImportedDatabase,
                source_id: SOURCE_ID.to_owned(),
                provider_id: None,
                account_id: None,
                session_id: Some(session_id.clone()),
                parent_session_id: session.and_then(|(record, _)| record.parent_session_id.clone()),
                request_id: None,
                response_id: None,
                model,
                query_source: None,
                usage,
                // OpenCode computes `cost` from its own model pricing tables;
                // it is an estimate, never a provider-stated bill, so it goes
                // into estimated_cost and the UI marks it as such (spec §18).
                provider_reported_cost: None,
                estimated_cost: cost,
                currency: cost.map(|_| "USD".to_owned()),
                http_status: None,
                latency_ms,
                success: None,
                // Numbers come verbatim from the request OpenCode recorded and
                // the session ownership is unambiguous, matching the precision
                // the native session adapters claim for their transcripts.
                precision_token: PrecisionLevel::ExactSession,
                precision_session: PrecisionLevel::ExactSession,
                precision_provider: PrecisionLevel::Unavailable,
                precision_account: PrecisionLevel::Unavailable,
                raw_event_hash,
                raw_usage_json,
            });
        }

        batch.skipped_records += skipped;
        batch.cursors.push(ImportCursor {
            source_id: SOURCE_ID.to_owned(),
            resource_id: PARTS_RESOURCE_ID.to_owned(),
            file_size: None,
            modified_at: Some(now()),
            byte_offset: max_time_created,
            content_hash: None,
            last_cumulative_usage: None,
            snapshot_generation: 0,
            last_session_id: None,
            last_model: None,
            updated_at: now(),
        });
        Ok(())
    }

    fn session_id(&self, external_session_id: &str) -> String {
        format!("{SOURCE_ID}:{}", short_hash(external_session_id))
    }

    fn event_hash(&self, part_id: &str) -> String {
        hash_strings([SOURCE_ID, "part", part_id])
    }
}

impl UsageAdapter for OpenCodeAdapter {
    fn id(&self) -> &'static str {
        SOURCE_ID
    }

    fn display_name(&self) -> &'static str {
        DESCRIPTOR.display_name
    }

    fn descriptor(&self) -> AdapterDescriptor {
        DESCRIPTOR
    }

    async fn detect(&self) -> Result<DetectionResult, AdapterError> {
        self.detect_sync().map_err(adapter_error)
    }

    async fn import_history(
        &self,
        cursor: Option<ImportCursor>,
    ) -> Result<ImportBatch, AdapterError> {
        let cursors = cursor
            .map(|value| HashMap::from([(value.resource_id.clone(), value)]))
            .unwrap_or_default();
        self.import_history_sync(&cursors).map_err(adapter_error)
    }

    async fn health(&self) -> Result<SourceHealth, AdapterError> {
        Ok(self.health_sync())
    }
}

/// Normalize one step-finish `tokens` object onto the shared vocabulary.
///
/// OpenCode's accounting matches the Anthropic-style separation: `input` is
/// the uncached portion and cache reads/writes are reported alongside it.
/// `total` is merely their sum and carries no information of its own, so it is
/// not stored. Every field OpenCode does not report stays `None`.
fn step_tokens_to_usage(tokens: &Value) -> Option<NormalizedUsage> {
    let input = u64_value(tokens.get("input"))?;
    let output = u64_value(tokens.get("output"));
    let reasoning = u64_value(tokens.get("reasoning"));
    let cache_read = u64_value(tokens.get("cache").and_then(|value| value.get("read")))?;
    let cache_write = u64_value(tokens.get("cache").and_then(|value| value.get("write")))?;
    let input_tokens_total = input.checked_add(cache_read)?.checked_add(cache_write)?;
    let visible_output_tokens = output
        .zip(reasoning)
        .and_then(|(output, reasoning)| output.checked_sub(reasoning));
    Some(NormalizedUsage {
        input_tokens_total: Some(input_tokens_total),
        input_tokens_uncached: Some(input),
        cache_read_tokens: Some(cache_read),
        cache_write_tokens: Some(cache_write),
        output_tokens_total: output,
        reasoning_tokens: reasoning,
        visible_output_tokens,
    })
}

fn u64_value(value: Option<&Value>) -> Option<u64> {
    value.and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
    })
}

/// Extract the model id from OpenCode's model JSON (`{id, modelID, ...}`) or a
/// plain string.
fn model_id(model: &Value) -> Option<String> {
    if let Some(id) = model.get("modelID").and_then(Value::as_str) {
        return Some(id.to_owned());
    }
    if let Some(id) = model.get("id").and_then(Value::as_str) {
        return Some(id.to_owned());
    }
    model.as_str().map(str::to_owned)
}

/// The step's wall-clock duration in milliseconds, when the source records it.
fn step_latency_ms(data: &Value) -> Option<i64> {
    let start = data.get("time")?.get("start")?.as_i64()?;
    let end = data.get("time")?.get("end")?.as_i64()?;
    (end >= start && start > 0).then_some(end - start)
}

fn resolve_db_path(path: PathBuf) -> PathBuf {
    if path.is_dir() {
        path.join(DB_FILENAME)
    } else {
        path
    }
}

/// The platform's default OpenCode database location.
///
/// OpenCode derives it from the XDG data dir: `~/.local/share/opencode` on
/// macOS/Linux and `%LOCALAPPDATA%\opencode` on Windows.
pub fn default_opencode_db() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"));
    base.map(|base| base.join("opencode").join(DB_FILENAME))
}

fn hash_strings<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

fn short_hash(value: &str) -> String {
    hash_strings([value]).chars().take(16).collect()
}

fn now() -> DateTime<Utc> {
    DateTime::<Utc>::from(SystemTime::now())
}

fn adapter_error(error: OpenCodeAdapterError) -> AdapterError {
    AdapterError {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use rusqlite::Connection;
    use tokenbuddy_domain::{AppKind, IngestSource, PrecisionLevel};

    use super::{
        OpenCodeAdapter, PARTS_RESOURCE_ID, SOURCE_ID, default_opencode_db, step_tokens_to_usage,
    };

    /// Sanitized millisecond epoch used by every fixture timestamp.
    const MS: i64 = 1_786_114_981_086;

    /// Build a sanitized OpenCode database with the classic schema: one session
    /// with two step-finish parts, one subagent session, and noise rows that
    /// must not produce events.
    fn write_fixture(dir: &tempfile::TempDir) -> PathBuf {
        let path = dir.path().join(super::DB_FILENAME);
        let connection = Connection::open(&path).expect("open fixture");
        connection
            .execute_batch(
                &format!(
                    "CREATE TABLE session (
                         id TEXT PRIMARY KEY,
                         project_id TEXT NOT NULL,
                         parent_id TEXT,
                         directory TEXT NOT NULL,
                         title TEXT NOT NULL,
                         model TEXT,
                         agent TEXT,
                         cost REAL NOT NULL DEFAULT 0,
                         tokens_input INTEGER NOT NULL DEFAULT 0,
                         tokens_output INTEGER NOT NULL DEFAULT 0,
                         time_created INTEGER NOT NULL,
                         time_updated INTEGER NOT NULL
                     );
                     CREATE TABLE message (
                         id TEXT PRIMARY KEY,
                         session_id TEXT NOT NULL,
                         time_created INTEGER NOT NULL,
                         time_updated INTEGER NOT NULL,
                         data TEXT NOT NULL
                     );
                     CREATE TABLE part (
                         id TEXT PRIMARY KEY,
                         message_id TEXT NOT NULL,
                         session_id TEXT NOT NULL,
                         time_created INTEGER NOT NULL,
                         time_updated INTEGER NOT NULL,
                         data TEXT NOT NULL
                     );
                     INSERT INTO session VALUES
                         ('ses_main', 'proj', NULL, '/work/demo', '为项目添加OpenCode支持',
                          '{{\"id\":\"deepseek-v4-flash\",\"providerID\":\"opencode-go\"}}',
                          'build', 0.0022799308, 29357, 528, {main_start}, {main_end}),
                         ('ses_sub', 'proj', 'ses_main', '/work/demo', 'subagent task',
                          '{{\"id\":\"claude-opus-5\",\"providerID\":\"anthropic\"}}',
                          'build', 0.0, 0, 0, {sub_start}, {sub_end});
                     INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES
                         ('msg_1', 'ses_main', {main_start}, {main_start},
                          '{{\"role\":\"assistant\",\"model\":{{\"modelID\":\"deepseek-v4-flash\",\"providerID\":\"opencode-go\"}}}}'),
                         ('msg_2', 'ses_main', {msg2}, {msg2},
                          '{{\"role\":\"assistant\"}}'),
                         ('msg_3', 'ses_sub', {sub_start}, {sub_start},
                          '{{\"role\":\"assistant\",\"model\":{{\"modelID\":\"claude-opus-5\"}}}}');
                     INSERT INTO part VALUES
                         ('prt_1', 'msg_1', 'ses_main', {main_start}, {main_start},
                          '{{\"type\":\"step-finish\",\"reason\":\"tool-calls\",
                            \"tokens\":{{\"total\":31944,\"input\":1425,\"output\":224,
                                        \"reasoning\":215,\"cache\":{{\"write\":0,\"read\":30080}}}},
                            \"cost\":0.000203322,
                            \"time\":{{\"start\":{main_start},\"end\":{end1}}}}}'),
                         ('prt_2', 'msg_2', 'ses_main', {msg2}, {msg2},
                          '{{\"type\":\"step-finish\",\"reason\":\"tool-calls\",
                            \"tokens\":{{\"total\":32569,\"input\":596,\"output\":83,
                                        \"reasoning\":18,\"cache\":{{\"write\":0,\"read\":31872}}}},
                            \"cost\":0.0001004808}}'),
                         ('prt_3', 'msg_3', 'ses_sub', {sub_start}, {sub_start},
                          '{{\"type\":\"step-finish\",
                            \"tokens\":{{\"total\":1000,\"input\":500,\"output\":100,
                                        \"reasoning\":0,\"cache\":{{\"write\":200,\"read\":200}}}},
                            \"cost\":null}}'),
                         ('prt_noise_text', 'msg_1', 'ses_main', {noise}, {noise},
                          '{{\"type\":\"text\",\"text\":\"not a usage record\"}}'),
                         ('prt_no_tokens', 'msg_1', 'ses_main', {noise} + 1000, {noise} + 1000,
                          '{{\"type\":\"step-finish\",\"reason\":\"error\"}}'),
                         ('prt_broken_json', 'msg_1', 'ses_main', {noise} + 2000, {noise} + 2000,
                          '{{not json at all');
                    ",
                    main_start = MS,
                    main_end = MS + 60_000,
                    sub_start = MS + 70_000,
                    sub_end = MS + 80_000,
                    msg2 = MS + 1000,
                    noise = MS + 2000,
                    end1 = MS + 900,
                ),
            )
            .expect("seed fixture");
        path
    }

    #[test]
    fn step_finish_tokens_map_onto_the_shared_vocabulary() {
        let tokens = serde_json::json!({
            "total": 31944,
            "input": 1425,
            "output": 224,
            "reasoning": 215,
            "cache": { "write": 0, "read": 30080 }
        });
        let usage = step_tokens_to_usage(&tokens).expect("normalized");
        assert_eq!(usage.input_tokens_total, Some(31505));
        assert_eq!(usage.input_tokens_uncached, Some(1425));
        assert_eq!(usage.cache_read_tokens, Some(30080));
        assert_eq!(usage.cache_write_tokens, Some(0));
        assert_eq!(usage.output_tokens_total, Some(224));
        assert_eq!(usage.reasoning_tokens, Some(215));
        assert_eq!(usage.visible_output_tokens, Some(9));
        assert_eq!(
            usage.cache_hit_rate_percent(),
            Some(30080.0 / 31505.0 * 100.0)
        );
    }

    #[test]
    fn import_reads_sessions_and_one_event_per_step_finish() {
        let dir = tempfile::tempdir().expect("temp dir");
        let adapter = OpenCodeAdapter::new(write_fixture(&dir));

        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import");

        assert_eq!(batch.sessions.len(), 2);
        let main = batch
            .sessions
            .iter()
            .find(|session| session.external_session_id.as_deref() == Some("ses_main"))
            .expect("main session");
        assert_eq!(main.app, AppKind::OpenCode);
        assert_eq!(main.project_path.as_deref(), Some("/work/demo"));
        assert_eq!(main.title.as_deref(), Some("为项目添加OpenCode支持"));
        assert_eq!(main.parent_session_id, None);
        let sub = batch
            .sessions
            .iter()
            .find(|session| session.external_session_id.as_deref() == Some("ses_sub"))
            .expect("subagent session");
        assert_eq!(sub.parent_session_id.as_deref(), Some(main.id.as_str()));

        assert_eq!(batch.usage_events.len(), 3);
        let first = &batch.usage_events[0];
        assert_eq!(first.app, AppKind::OpenCode);
        assert_eq!(first.ingest_source, IngestSource::ImportedDatabase);
        assert_eq!(first.model.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(first.usage.input_tokens_total, Some(31505));
        assert_eq!(first.usage.cache_read_tokens, Some(30080));
        assert_eq!(first.estimated_cost, Some(0.000203322));
        assert_eq!(first.provider_reported_cost, None);
        assert_eq!(first.latency_ms, Some(900));
        assert_eq!(first.precision_token, PrecisionLevel::ExactSession);
        assert_eq!(first.precision_session, PrecisionLevel::ExactSession);
        assert_eq!(first.precision_provider, PrecisionLevel::Unavailable);
        assert_eq!(first.precision_account, PrecisionLevel::Unavailable);
        assert_eq!(first.session_id.as_deref(), Some(main.id.as_str()));
        assert_eq!(first.raw_event_hash, first.id);
        // The model falls back to the session when the message names none.
        assert_eq!(
            batch.usage_events[1].model.as_deref(),
            Some("deepseek-v4-flash")
        );
        // The subagent event inherits the parent session chain.
        assert_eq!(
            batch.usage_events[2].parent_session_id.as_deref(),
            Some(main.id.as_str())
        );

        // Exactly one skipped record: the malformed JSON line. A step-finish
        // without a tokens block and text/tool parts are legitimate records,
        // not failures.
        assert_eq!(batch.skipped_records, 1);

        let cursor = batch
            .cursors
            .iter()
            .find(|cursor| cursor.resource_id == PARTS_RESOURCE_ID)
            .expect("part cursor");
        assert_eq!(cursor.byte_offset, MS + 70_000);
        assert_eq!(cursor.source_id, SOURCE_ID);
    }

    #[test]
    fn repeated_import_is_idempotent_and_incremental() {
        let dir = tempfile::tempdir().expect("temp dir");
        let adapter = OpenCodeAdapter::new(write_fixture(&dir));

        let first = adapter
            .import_history_sync(&HashMap::new())
            .expect("first import");
        let first_hashes: Vec<_> = first
            .usage_events
            .iter()
            .map(|event| event.raw_event_hash.clone())
            .collect();
        let mut cursors: HashMap<_, _> = first
            .cursors
            .iter()
            .map(|cursor| (cursor.resource_id.clone(), cursor.clone()))
            .collect();

        // The query is `time_created >= high_water`, so the same-ms bucket is
        // re-read on purpose: a part appended in the same millisecond as the
        // previous max must not be skipped. Re-reads carry the same stable
        // hash, which the storage layer deduplicates.
        let second = adapter
            .import_history_sync(&cursors)
            .expect("second import");
        assert_eq!(second.usage_events.len(), 1, "only the same-ms bucket");
        assert!(
            first_hashes.contains(&second.usage_events[0].raw_event_hash),
            "re-read event must carry a hash already imported"
        );

        // Appending one part after the cursor imports it alongside the bucket.
        let connection = Connection::open(adapter.db_path()).expect("open");
        connection
            .execute(
                "INSERT INTO part VALUES ('prt_4', 'msg_1', 'ses_main', ?1, ?1,
                  '{\"type\":\"step-finish\",
                    \"tokens\":{\"total\":10,\"input\":5,\"output\":3,
                                \"reasoning\":1,\"cache\":{\"write\":1,\"read\":0}}}')",
                [MS + 90_000],
            )
            .expect("append part");
        drop(connection);
        cursors = second
            .cursors
            .iter()
            .map(|cursor| (cursor.resource_id.clone(), cursor.clone()))
            .collect();
        let third = adapter.import_history_sync(&cursors).expect("third import");
        let new_event = third
            .usage_events
            .iter()
            .find(|event| !first_hashes.contains(&event.raw_event_hash))
            .expect("the appended part is the only new event");
        assert_eq!(new_event.occurred_at.timestamp_millis(), MS + 90_000);
        assert_eq!(new_event.usage.input_tokens_total, Some(6));
    }

    #[test]
    fn a_step_finish_with_invalid_numbers_is_skipped_not_fabricated() {
        let dir = tempfile::tempdir().expect("temp dir");
        let adapter = OpenCodeAdapter::new(write_fixture(&dir));
        let connection = Connection::open(adapter.db_path()).expect("open");
        connection
            .execute(
                "INSERT INTO part VALUES ('prt_bad_tokens', 'msg_1', 'ses_main', ?1, ?1,
                  '{\"type\":\"step-finish\",\"tokens\":{\"input\":\"many\"}}')",
                [MS + 95_000],
            )
            .expect("insert");
        drop(connection);

        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import");
        assert_eq!(
            batch.usage_events.len(),
            3,
            "bad-token part is not an event"
        );
        assert_eq!(batch.skipped_records, 2, "malformed JSON + unusable tokens");
    }

    #[test]
    fn a_directory_path_resolves_to_the_database_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = write_fixture(&dir);
        let via_dir = OpenCodeAdapter::new(dir.path());
        let via_file = OpenCodeAdapter::new(file);
        assert_eq!(via_dir.db_path(), via_file.db_path());
    }

    #[test]
    fn detection_and_health_follow_the_database() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = write_fixture(&dir);
        let adapter = OpenCodeAdapter::new(file.clone());
        let detection = adapter.detect_sync().expect("detect");
        assert!(detection.detected);
        assert_eq!(detection.detected_version.as_deref(), Some("sqlite"));
        assert_eq!(adapter.health_sync().status, "healthy");

        let missing = OpenCodeAdapter::new(dir.path().join("absent.db"));
        let detection = missing.detect_sync().expect("detect");
        assert!(!detection.detected);
        assert_eq!(missing.health_sync().status, "not_found");
        let batch = missing
            .import_history_sync(&HashMap::new())
            .expect("import");
        assert_eq!(
            batch
                .source
                .as_ref()
                .and_then(|source| source.health_status.as_deref()),
            Some("not_found")
        );
    }

    #[test]
    fn a_database_without_the_expected_tables_is_schema_unsupported() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(super::DB_FILENAME);
        let connection = Connection::open(&path).expect("open");
        connection
            .execute_batch("CREATE TABLE unrelated (id INTEGER);")
            .expect("seed");
        drop(connection);

        let adapter = OpenCodeAdapter::new(path);
        let error = adapter
            .import_history_sync(&HashMap::new())
            .expect_err("unsupported schema");
        assert!(
            error.to_string().contains("session"),
            "unexpected message: {error}"
        );
    }

    #[test]
    fn default_paths_follow_each_platforms_data_directory() {
        if let Some(path) = default_opencode_db() {
            assert_eq!(
                path.file_name().and_then(|name| name.to_str()),
                Some("opencode.db")
            );
        }
    }
}
