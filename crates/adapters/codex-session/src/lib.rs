//! Read-only Codex session JSONL adapter.
//!
//! The parser works on sanitized fixtures and never stores prompt, completion,
//! or source-code bodies. Only stable session metadata and usage fields are
//! normalized into the shared domain model.

use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsStr,
    fs::{self, File},
    io::{self, BufRead, BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::SystemTime,
};

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokenbuddy_domain::{
    AdapterError, AppKind, DetectionResult, EventSink, ImportBatch, ImportCursor, IngestSource,
    LauncherKind, NormalizedUsage, PrecisionLevel, SessionRecord, SourceHealth, SourceRecord,
    UsageAdapter, UsageEvent, WatcherHandle,
};

pub const SOURCE_ID: &str = "codex-session";
pub const ADAPTER_TYPE: &str = "codex_session";
pub const DISPLAY_NAME: &str = "Codex Sessions";
const SESSION_INDEX_RESOURCE_ID: &str = "session_index.jsonl";

#[derive(Debug, Error)]
pub enum CodexAdapterError {
    #[error("failed to read Codex session files: {0}")]
    Io(#[from] io::Error),
    #[error("Codex session home is not a directory: {0}")]
    InvalidHome(PathBuf),
}

#[derive(Debug, Clone)]
pub struct CodexSessionAdapter {
    codex_home: PathBuf,
}

impl CodexSessionAdapter {
    pub fn new(codex_home: impl Into<PathBuf>) -> Self {
        Self {
            codex_home: codex_home.into(),
        }
    }

    pub fn codex_home(&self) -> &Path {
        &self.codex_home
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.codex_home.join("sessions")
    }

    pub fn detect_sync(&self) -> Result<DetectionResult, CodexAdapterError> {
        let sessions_dir = self.sessions_dir();
        let detected = sessions_dir.is_dir();
        Ok(DetectionResult {
            source_id: SOURCE_ID.to_owned(),
            detected,
            path_or_endpoint: Some(self.codex_home.to_string_lossy().into_owned()),
            detected_version: detected.then(|| "jsonl".to_owned()),
            message: if detected {
                Some("Codex session directory detected".to_owned())
            } else {
                Some("Codex session directory was not found".to_owned())
            },
        })
    }

    pub fn import_history_sync(
        &self,
        cursors: &HashMap<String, ImportCursor>,
    ) -> Result<ImportBatch, CodexAdapterError> {
        let sessions_dir = self.sessions_dir();
        if !sessions_dir.exists() {
            return Ok(ImportBatch {
                source: Some(self.source_record("not_found")),
                ..ImportBatch::default()
            });
        }
        if !sessions_dir.is_dir() {
            return Err(CodexAdapterError::InvalidHome(sessions_dir));
        }

        let index_snapshot = read_session_index(
            &self.codex_home.join(SESSION_INDEX_RESOURCE_ID),
            cursors.get(SESSION_INDEX_RESOURCE_ID),
        );
        let session_titles = index_snapshot
            .as_ref()
            .map(|snapshot| snapshot.titles.clone())
            .unwrap_or_default();
        let session_index_changed = index_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.changed);
        let mut files = Vec::new();
        collect_jsonl_files(&sessions_dir, &mut files)?;
        files.sort();

        let mut batch = ImportBatch {
            source: Some(self.source_record("healthy")),
            ..ImportBatch::default()
        };
        if let Some(snapshot) = index_snapshot {
            batch.cursors.push(snapshot.cursor);
        }

        for path in files {
            let resource_id = resource_id(&self.codex_home, &path);
            let cursor = cursors.get(&resource_id);
            let parsed = self.import_file(
                &path,
                &resource_id,
                cursor,
                &session_titles,
                session_index_changed,
            )?;
            batch.sessions.extend(parsed.sessions);
            batch.usage_events.extend(parsed.usage_events);
            batch.cursors.push(parsed.cursor);
            batch.skipped_records += parsed.skipped_records;
        }

        Ok(batch)
    }

    pub fn health_sync(&self) -> SourceHealth {
        let detected = self.sessions_dir().is_dir();
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

    fn source_record(&self, status: &str) -> SourceRecord {
        let timestamp = now();
        SourceRecord {
            id: SOURCE_ID.to_owned(),
            adapter_type: ADAPTER_TYPE.to_owned(),
            display_name: DISPLAY_NAME.to_owned(),
            path_or_endpoint: Some(self.codex_home.to_string_lossy().into_owned()),
            enabled: true,
            detected_version: Some("jsonl".to_owned()),
            health_status: Some(status.to_owned()),
            last_success_at: (status == "healthy").then_some(timestamp),
            last_error: None,
            created_at: timestamp,
            updated_at: timestamp,
        }
    }

    fn import_file(
        &self,
        path: &Path,
        resource_id: &str,
        cursor: Option<&ImportCursor>,
        session_titles: &HashMap<String, String>,
        session_index_changed: bool,
    ) -> Result<ParsedFile, CodexAdapterError> {
        let metadata = fs::metadata(path)?;
        let file_size = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
        let modified_at = metadata.modified().ok().map(DateTime::<Utc>::from);
        let current_content_hash = first_line_hash(path)?;
        let cursor_is_stale = cursor.is_some_and(|value| {
            value.byte_offset < 0
                || u64::try_from(value.byte_offset).map_or(true, |offset| offset > metadata.len())
                || value.file_size.is_some_and(|size| size > file_size)
                || (value.byte_offset > 0
                    && value.content_hash.is_some()
                    && value.content_hash != current_content_hash)
        });
        let start_offset = if cursor_is_stale {
            0
        } else {
            cursor.map_or(0, |value| value.byte_offset.max(0) as u64)
        };

        let mut state = ParseState {
            // Rollout headers appear once at the top of the file; restore the
            // session identity captured by the previous import so appended
            // `token_count` rows stay attached to the same session instead of
            // splitting off under the file-stem fallback.
            current_session_id: (!cursor_is_stale)
                .then(|| cursor.and_then(|value| value.last_session_id.clone()))
                .flatten(),
            last_cumulative_usage: (!cursor_is_stale)
                .then(|| cursor.and_then(|value| value.last_cumulative_usage.clone()))
                .flatten(),
            snapshot_generation: if cursor_is_stale {
                0
            } else {
                cursor.map_or(0, |value| value.snapshot_generation)
            },
        };
        let default_external_session_id = path
            .file_stem()
            .and_then(OsStr::to_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| resource_id.to_owned());
        let mut sessions = BTreeMap::<String, SessionRecord>::new();
        if start_offset > 0 && session_index_changed && !session_titles.is_empty() {
            self.import_session_metadata(
                path,
                &default_external_session_id,
                session_titles,
                &mut sessions,
            )?;
        }
        let mut usage_events = Vec::new();
        let mut skipped_records = 0;
        let mut offset = start_offset;
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        reader.seek(SeekFrom::Start(start_offset))?;
        let mut line = Vec::new();

        loop {
            line.clear();
            let bytes_read = reader.read_until(b'\n', &mut line)?;
            if bytes_read == 0 {
                break;
            }

            let line_offset = offset;
            offset += bytes_read as u64;
            let has_newline = line.ends_with(b"\n");
            let trimmed = trim_line_end(&line);
            if trimmed.is_empty() {
                continue;
            }

            let value: Value = match serde_json::from_slice(trimmed) {
                Ok(value) => value,
                Err(_) => {
                    // A writer can append a JSONL record in more than one
                    // write. Keep an incomplete final line out of the cursor
                    // so the next import can retry it after the newline and
                    // remaining bytes arrive.
                    if !has_newline {
                        offset = line_offset;
                        break;
                    }
                    skipped_records += 1;
                    continue;
                }
            };

            let mut context = parse_context(&value, &default_external_session_id, &state);
            apply_indexed_session_context(
                &mut context,
                session_titles,
                &default_external_session_id,
            );
            if let Some(external_session_id) = &context.session_id {
                state.current_session_id = Some(external_session_id.clone());
                update_session(
                    &mut sessions,
                    &self.session_record(external_session_id, &context),
                    context.timestamp,
                );
            }

            if context.inherited_history {
                continue;
            }

            let Some(candidate) = find_usage(&value) else {
                continue;
            };
            let Some(timestamp) = context.timestamp else {
                skipped_records += 1;
                continue;
            };
            let Some(normalized_usage) = normalize_usage(&candidate.value) else {
                skipped_records += 1;
                continue;
            };

            let usage = if candidate.cumulative {
                let current_snapshot = normalized_usage.clone();
                let (usage, generation) = match &state.last_cumulative_usage {
                    None => (Some(normalized_usage.clone()), state.snapshot_generation),
                    Some(previous) if previous == &normalized_usage => {
                        (None, state.snapshot_generation)
                    }
                    Some(previous) => match normalized_usage.checked_delta(previous) {
                        Some(delta) => (Some(delta), state.snapshot_generation),
                        None => {
                            let next_generation = state.snapshot_generation + 1;
                            (Some(normalized_usage.clone()), next_generation)
                        }
                    },
                };
                state.last_cumulative_usage = Some(current_snapshot);
                state.snapshot_generation = generation;
                usage
            } else {
                Some(normalized_usage)
            };

            let Some(usage) = usage else {
                continue;
            };

            let external_session_id = context
                .session_id
                .clone()
                .or_else(|| state.current_session_id.clone())
                .unwrap_or_else(|| default_external_session_id.clone());
            let session = self.session_record(&external_session_id, &context);
            update_session(&mut sessions, &session, Some(timestamp));
            let session_id = session.id.clone();
            let parent_session_id = context
                .parent_session_id
                .as_deref()
                .map(|value| self.session_id(value));
            let raw_usage_json = Some(candidate.value.clone());
            let raw_event_hash = event_hash(
                self.id(),
                resource_id,
                line_offset,
                &context,
                &raw_usage_json,
                candidate.cumulative.then_some(state.snapshot_generation),
            );
            usage_events.push(UsageEvent {
                id: raw_event_hash.clone(),
                occurred_at: timestamp,
                app: AppKind::Codex,
                launcher: LauncherKind::Direct,
                ingest_source: IngestSource::SessionLog,
                source_id: SOURCE_ID.to_owned(),
                provider_id: None,
                account_id: None,
                session_id: Some(session_id),
                parent_session_id,
                request_id: context.request_id.clone(),
                response_id: context.response_id.clone(),
                model: context.model.clone(),
                query_source: context.query_source.clone(),
                usage,
                provider_reported_cost: None,
                estimated_cost: None,
                currency: None,
                http_status: None,
                latency_ms: None,
                success: None,
                precision_token: PrecisionLevel::ExactSession,
                precision_session: PrecisionLevel::ExactSession,
                precision_provider: PrecisionLevel::Unavailable,
                precision_account: PrecisionLevel::Unavailable,
                raw_event_hash,
                raw_usage_json,
            });
        }

        let cursor = ImportCursor {
            source_id: SOURCE_ID.to_owned(),
            resource_id: resource_id.to_owned(),
            file_size: Some(file_size),
            modified_at,
            byte_offset: i64::try_from(offset).unwrap_or(i64::MAX),
            content_hash: current_content_hash,
            last_cumulative_usage: state.last_cumulative_usage,
            snapshot_generation: state.snapshot_generation,
            last_session_id: state.current_session_id,
            updated_at: now(),
        };

        Ok(ParsedFile {
            sessions: sessions.into_values().collect(),
            usage_events,
            cursor,
            skipped_records,
        })
    }

    fn import_session_metadata(
        &self,
        path: &Path,
        default_external_session_id: &str,
        session_titles: &HashMap<String, String>,
        sessions: &mut BTreeMap<String, SessionRecord>,
    ) -> Result<(), CodexAdapterError> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut state = ParseState::default();

        for line in reader.lines().map_while(Result::ok) {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };

            let mut context = parse_context(&value, default_external_session_id, &state);
            apply_indexed_session_context(
                &mut context,
                session_titles,
                default_external_session_id,
            );
            if let Some(external_session_id) = &context.session_id {
                state.current_session_id = Some(external_session_id.clone());
                update_session(
                    sessions,
                    &self.session_record(external_session_id, &context),
                    context.timestamp,
                );
            }
        }

        Ok(())
    }

    fn session_id(&self, external_session_id: &str) -> String {
        format!("{SOURCE_ID}:{}", short_hash(external_session_id))
    }

    fn session_record(&self, external_session_id: &str, context: &ParseContext) -> SessionRecord {
        let timestamp = context.timestamp;
        SessionRecord {
            id: self.session_id(external_session_id),
            external_session_id: Some(external_session_id.to_owned()),
            parent_session_id: context
                .parent_session_id
                .as_deref()
                .map(|value| self.session_id(value)),
            app: AppKind::Codex,
            launcher: Some(LauncherKind::Direct),
            project_path: context.project_path.clone(),
            title: context.title.clone(),
            started_at: timestamp,
            ended_at: timestamp,
            source_id: Some(SOURCE_ID.to_owned()),
            created_at: now(),
            updated_at: now(),
        }
    }
}

impl UsageAdapter for CodexSessionAdapter {
    fn id(&self) -> &'static str {
        SOURCE_ID
    }

    fn display_name(&self) -> &'static str {
        DISPLAY_NAME
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

    async fn start_watch(&self, _sink: EventSink) -> Result<WatcherHandle, AdapterError> {
        Err(AdapterError {
            message: "Codex file watcher is scheduled after the initial importer".to_owned(),
        })
    }

    async fn health(&self) -> Result<SourceHealth, AdapterError> {
        Ok(self.health_sync())
    }
}

#[derive(Debug)]
struct ParsedFile {
    sessions: Vec<SessionRecord>,
    usage_events: Vec<UsageEvent>,
    cursor: ImportCursor,
    skipped_records: usize,
}

#[derive(Debug, Default)]
struct ParseState {
    current_session_id: Option<String>,
    last_cumulative_usage: Option<NormalizedUsage>,
    snapshot_generation: i64,
}

#[derive(Debug, Default)]
struct ParseContext {
    session_id: Option<String>,
    parent_session_id: Option<String>,
    project_path: Option<String>,
    title: Option<String>,
    model: Option<String>,
    request_id: Option<String>,
    response_id: Option<String>,
    query_source: Option<String>,
    timestamp: Option<DateTime<Utc>>,
    inherited_history: bool,
}

#[derive(Debug, Clone)]
struct UsageCandidate {
    value: Value,
    cumulative: bool,
}

fn collect_jsonl_files(directory: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_jsonl_files(&path, files)?;
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
        {
            files.push(path);
        }
    }
    Ok(())
}

fn resource_id(codex_home: &Path, path: &Path) -> String {
    path.strip_prefix(codex_home)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[derive(Debug)]
struct SessionIndexSnapshot {
    titles: HashMap<String, String>,
    cursor: ImportCursor,
    changed: bool,
}

fn read_session_index(
    path: &Path,
    previous_cursor: Option<&ImportCursor>,
) -> Option<SessionIndexSnapshot> {
    let bytes = fs::read(path).ok()?;
    let metadata = fs::metadata(path).ok()?;
    let content_hash = hash_bytes(&bytes);
    let changed = previous_cursor.and_then(|cursor| cursor.content_hash.as_deref())
        != Some(content_hash.as_str());
    let mut titles = HashMap::new();

    for line in bytes.split(|byte| *byte == b'\n') {
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if let Some((id, title)) = session_index_entry(&value) {
            titles.insert(id, title);
        }
    }

    Some(SessionIndexSnapshot {
        titles,
        cursor: ImportCursor {
            source_id: SOURCE_ID.to_owned(),
            resource_id: SESSION_INDEX_RESOURCE_ID.to_owned(),
            file_size: Some(i64::try_from(metadata.len()).unwrap_or(i64::MAX)),
            modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
            byte_offset: i64::try_from(bytes.len()).unwrap_or(i64::MAX),
            content_hash: Some(content_hash),
            last_cumulative_usage: None,
            snapshot_generation: 0,
            last_session_id: None,
            updated_at: now(),
        },
        changed,
    })
}

fn session_index_entry(value: &Value) -> Option<(String, String)> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let title = value
        .get("thread_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some((id.to_owned(), title.to_owned()))
}

fn indexed_session_title(
    titles: &HashMap<String, String>,
    session_id: Option<&str>,
    default_external_session_id: &str,
) -> Option<String> {
    session_id
        .and_then(|value| titles.get(value))
        .or_else(|| titles.get(default_external_session_id))
        .map(String::as_str)
        .map(str::to_owned)
        .or_else(|| indexed_file_title(titles, default_external_session_id))
}

fn indexed_file_title(
    titles: &HashMap<String, String>,
    default_external_session_id: &str,
) -> Option<String> {
    titles.iter().find_map(|(indexed_id, title)| {
        (!indexed_id.is_empty() && default_external_session_id.contains(indexed_id))
            .then(|| title.clone())
    })
}

fn apply_indexed_session_context(
    context: &mut ParseContext,
    titles: &HashMap<String, String>,
    default_external_session_id: &str,
) {
    let file_title = indexed_file_title(titles, default_external_session_id);
    if let Some(title) = indexed_session_title(
        titles,
        context.session_id.as_deref(),
        default_external_session_id,
    )
    .or(file_title.clone())
    {
        context.title = Some(title);
    }

    // Existing imports used the stable JSONL filename as the external session
    // ID when Codex nested the real ID under `payload`. Keep that identity so
    // title backfill updates existing rows and does not duplicate usage events.
    if file_title.is_some() {
        context.session_id = Some(default_external_session_id.to_owned());
    }
}

fn parse_context(value: &Value, default_session_id: &str, state: &ParseState) -> ParseContext {
    let record_type = value_string(value, &["type", "event_type", "eventType"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    let session_id = payload_value_string(
        value,
        &[
            "session_id",
            "sessionId",
            "conversation_id",
            "conversationId",
            "thread_id",
            "threadId",
            "agent_thread_id",
            "agentThreadId",
        ],
    )
    .or_else(|| {
        (record_type.contains("session") || record_type.contains("subagent"))
            .then(|| payload_value_string(value, &["id"]))
            .flatten()
    })
    .or_else(|| state.current_session_id.clone())
    .or_else(|| Some(default_session_id.to_owned()));

    ParseContext {
        session_id,
        parent_session_id: payload_value_string(
            value,
            &[
                "parent_session_id",
                "parentSessionId",
                "parent_session",
                "parent_thread_id",
                "parentThreadId",
            ],
        ),
        project_path: payload_value_string(
            value,
            &["project_path", "projectPath", "cwd", "working_directory"],
        ),
        title: payload_value_string(value, &["title", "session_title", "sessionTitle"]),
        model: payload_value_string(value, &["model", "model_name", "modelName"]),
        request_id: payload_value_string(value, &["request_id", "requestId"]),
        response_id: payload_value_string(value, &["response_id", "responseId"]),
        query_source: payload_value_string(value, &["query_source", "querySource"]),
        timestamp: timestamp_value(value),
        inherited_history: payload_value_bool(value, &["inherited"]).unwrap_or(false)
            || record_type.contains("inherited")
            || record_type.contains("history_copy"),
    }
}

fn update_session(
    sessions: &mut BTreeMap<String, SessionRecord>,
    incoming: &SessionRecord,
    event_timestamp: Option<DateTime<Utc>>,
) {
    sessions
        .entry(incoming.id.clone())
        .and_modify(|session| {
            if session.project_path.is_none() {
                session.project_path = incoming.project_path.clone();
            }
            if session.title.is_none() {
                session.title = incoming.title.clone();
            }
            if session.parent_session_id.is_none() {
                session.parent_session_id = incoming.parent_session_id.clone();
            }
            if let Some(timestamp) = event_timestamp {
                session.started_at = Some(
                    session
                        .started_at
                        .map_or(timestamp, |current| current.min(timestamp)),
                );
                session.ended_at = Some(
                    session
                        .ended_at
                        .map_or(timestamp, |current| current.max(timestamp)),
                );
            }
            session.updated_at = now();
        })
        .or_insert_with(|| incoming.clone());
}

fn find_usage(value: &Value) -> Option<UsageCandidate> {
    let object = value.as_object()?;
    for key in ["total_token_usage", "totalTokenUsage", "cumulative_usage"] {
        if let Some(candidate) = object.get(key).filter(|value| value.is_object()) {
            return Some(UsageCandidate {
                value: candidate.clone(),
                cumulative: true,
            });
        }
    }
    for key in ["usage", "last_token_usage", "lastTokenUsage"] {
        if let Some(candidate) = object.get(key).filter(|value| value.is_object()) {
            return Some(UsageCandidate {
                value: candidate.clone(),
                cumulative: false,
            });
        }
    }
    if has_usage_field(object) {
        return Some(UsageCandidate {
            value: Value::Object(object.clone()),
            cumulative: false,
        });
    }
    object.values().find_map(find_usage)
}

fn has_usage_field(object: &Map<String, Value>) -> bool {
    [
        "input_tokens",
        "inputTokens",
        "cached_input_tokens",
        "cachedInputTokens",
        "output_tokens",
        "outputTokens",
        "reasoning_output_tokens",
        "reasoning_tokens",
        "total_tokens",
        "totalTokens",
    ]
    .iter()
    .any(|key| object.contains_key(*key))
}

fn normalize_usage(value: &Value) -> Option<NormalizedUsage> {
    let input_tokens_total = number(value, &["input_tokens", "inputTokens", "prompt_tokens"]);
    let cache_read_tokens = number(
        value,
        &[
            "cached_input_tokens",
            "cachedInputTokens",
            "cache_read_tokens",
            "cache_read_input_tokens",
        ],
    );
    let cache_write_tokens = number(
        value,
        &[
            "cache_write_tokens",
            "cache_creation_input_tokens",
            "cache_creation_tokens",
        ],
    );
    let input_tokens_uncached = input_tokens_total
        .zip(cache_read_tokens)
        .and_then(|(input, cached)| input.checked_sub(cached));
    let output_tokens_total = number(
        value,
        &["output_tokens", "outputTokens", "completion_tokens"],
    );
    let reasoning_tokens = number(
        value,
        &[
            "reasoning_output_tokens",
            "reasoning_tokens",
            "reasoningTokens",
        ],
    );
    let visible_output_tokens = output_tokens_total
        .zip(reasoning_tokens)
        .and_then(|(output, reasoning)| output.checked_sub(reasoning));
    let usage = NormalizedUsage {
        input_tokens_total,
        input_tokens_uncached,
        cache_read_tokens,
        cache_write_tokens,
        output_tokens_total,
        reasoning_tokens,
        visible_output_tokens,
    };
    (!usage.is_empty()).then_some(usage)
}

fn number(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|number| {
            number
                .as_u64()
                .or_else(|| number.as_i64().and_then(|value| u64::try_from(value).ok()))
        })
    })
}

fn value_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str).map(str::to_owned))
}

fn payload_value_string(value: &Value, keys: &[&str]) -> Option<String> {
    value_string(value, keys).or_else(|| {
        value
            .get("payload")
            .and_then(|payload| value_string(payload, keys))
    })
}

fn payload_value_bool(value: &Value, keys: &[&str]) -> Option<bool> {
    value
        .as_object()
        .and_then(|object| keys.iter().find_map(|key| object.get(*key)?.as_bool()))
        .or_else(|| {
            value
                .get("payload")
                .and_then(|payload| payload_value_bool(payload, keys))
        })
}

fn timestamp_value(value: &Value) -> Option<DateTime<Utc>> {
    let timestamp = [
        "timestamp",
        "occurred_at",
        "created_at",
        "createdAt",
        "time",
    ]
    .iter()
    .find_map(|key| value.get(*key))
    .or_else(|| {
        value.get("payload").and_then(|payload| {
            [
                "timestamp",
                "occurred_at",
                "created_at",
                "createdAt",
                "time",
            ]
            .iter()
            .find_map(|key| payload.get(*key))
        })
    });
    timestamp.and_then(parse_timestamp)
}

fn parse_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    if let Some(value) = value.as_str() {
        return DateTime::parse_from_rfc3339(value)
            .ok()
            .map(|date| date.with_timezone(&Utc));
    }
    let timestamp = value.as_i64()?;
    if timestamp > 100_000_000_000 {
        Utc.timestamp_millis_opt(timestamp).single()
    } else {
        Utc.timestamp_opt(timestamp, 0).single()
    }
}

fn event_hash(
    source_id: &str,
    resource_id: &str,
    offset: u64,
    context: &ParseContext,
    raw_usage_json: &Option<Value>,
    snapshot_generation: Option<i64>,
) -> String {
    // Prefer a stable request/response identity so a `response.completed` row is
    // counted once regardless of which session identity the parser resolved for
    // it. Crucially the fallback below no longer folds in `session_id`: the same
    // physical `token_count` line hashed the same way whether it was first seen
    // during an incremental tail (file-stem identity) or a later full re-scan
    // (session_meta identity), which previously let one row be counted twice.
    let response = context.response_id.as_deref().unwrap_or_default();
    let request = context.request_id.as_deref().unwrap_or_default();
    if snapshot_generation.is_none() && !(response.is_empty() && request.is_empty()) {
        return hash_strings([source_id, "identity", response, request]);
    }
    let raw_usage = raw_usage_json
        .as_ref()
        .map_or_else(String::new, |value| value.to_string());
    let parts = [
        source_id.to_owned(),
        format!("{resource_id}:{offset}"),
        context
            .timestamp
            .map_or_else(String::new, |value| value.to_rfc3339()),
        context.model.clone().unwrap_or_default(),
        raw_usage,
        snapshot_generation.map_or_else(String::new, |value| value.to_string()),
    ];
    hash_strings(parts.iter().map(String::as_str))
}

fn hash_strings<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn first_line_hash(path: &Path) -> io::Result<Option<String>> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    if reader.read_until(b'\n', &mut line)? == 0 {
        Ok(None)
    } else {
        Ok(Some(hash_bytes(&line)))
    }
}

fn short_hash(value: &str) -> String {
    hash_strings([value]).chars().take(16).collect()
}

fn trim_line_end(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\n")
        .unwrap_or(line)
        .strip_suffix(b"\r")
        .unwrap_or_else(|| line.strip_suffix(b"\n").unwrap_or(line))
}

fn now() -> DateTime<Utc> {
    DateTime::<Utc>::from(SystemTime::now())
}

fn adapter_error(error: CodexAdapterError) -> AdapterError {
    AdapterError {
        message: error.to_string(),
    }
}

pub fn default_codex_home() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let user_profile = std::env::var_os("USERPROFILE");
        let home_drive = std::env::var_os("HOMEDRIVE");
        let home_path = std::env::var_os("HOMEPATH");
        codex_home_from_env(
            user_profile.as_deref(),
            home_drive.as_deref(),
            home_path.as_deref(),
        )
    }

    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex"))
    }
}

#[cfg(windows)]
fn codex_home_from_env(
    user_profile: Option<&OsStr>,
    home_drive: Option<&OsStr>,
    home_path: Option<&OsStr>,
) -> Option<PathBuf> {
    user_profile
        .map(PathBuf::from)
        .or_else(|| match (home_drive, home_path) {
            (Some(drive), Some(path)) => {
                let mut home = std::ffi::OsString::from(drive);
                home.push(path);
                Some(PathBuf::from(home))
            }
            _ => None,
        })
        .map(|home| home.join(".codex"))
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, path::Path};

    use tempfile::TempDir;
    use tokenbuddy_domain::PrecisionLevel;

    use super::{CodexSessionAdapter, SESSION_INDEX_RESOURCE_ID, default_codex_home};

    fn fixture_home(fixture: &str) -> TempDir {
        let home = tempfile::tempdir().expect("temporary home");
        let sessions = home.path().join("sessions");
        fs::create_dir_all(&sessions).expect("sessions directory");
        let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/codex")
            .join(fixture);
        fs::copy(fixture_path, sessions.join(fixture)).expect("copy fixture");
        home
    }

    #[test]
    fn imports_simple_usage_and_restarts_idempotently() {
        let home = fixture_home("simple_session.jsonl");
        let adapter = CodexSessionAdapter::new(home.path());
        let first = adapter
            .import_history_sync(&HashMap::new())
            .expect("first import");
        assert_eq!(first.usage_events.len(), 2);
        assert_eq!(first.sessions.len(), 1);
        assert_eq!(
            first.usage_events[0].precision_token,
            PrecisionLevel::ExactSession
        );
        assert_eq!(first.usage_events[0].usage.cache_read_tokens, Some(20));

        let cursors = first
            .cursors
            .into_iter()
            .map(|cursor| (cursor.resource_id.clone(), cursor))
            .collect();
        let second = adapter
            .import_history_sync(&cursors)
            .expect("repeated import");
        assert!(second.usage_events.is_empty());
        assert_eq!(second.sessions.len(), 0);

        let session_path = home.path().join("sessions/simple_session.jsonl");
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(session_path)
            .expect("open fixture for append");
        use std::io::Write;
        file.write_all(
            br#"{"type":"response.completed","session_id":"simple-session","timestamp":"2026-07-25T08:00:03Z","request_id":"request-003","model":"gpt-5-codex","usage":{"input_tokens":20,"output_tokens":8}}"#,
        )
        .and_then(|_| file.write_all(b"\n"))
        .expect("append fixture record");
        file.flush().expect("flush appended fixture record");
        drop(file);
        let third = adapter
            .import_history_sync(&cursors)
            .expect("incremental import");
        assert_eq!(third.usage_events.len(), 1);
        assert_eq!(third.usage_events[0].usage.input_tokens_total, Some(20));
    }

    #[test]
    fn retries_an_incomplete_final_jsonl_line_after_append() {
        let home = fixture_home("simple_session.jsonl");
        let adapter = CodexSessionAdapter::new(home.path());
        let session_path = home.path().join("sessions/simple_session.jsonl");
        let baseline = adapter
            .import_history_sync(&HashMap::new())
            .expect("baseline import");
        let baseline_cursors = baseline
            .cursors
            .into_iter()
            .map(|cursor| (cursor.resource_id.clone(), cursor))
            .collect();

        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&session_path)
            .expect("open fixture for partial append");
        use std::io::Write;
        file.write_all(
            br#"{"type":"response.completed","session_id":"simple-session","timestamp":"2026-07-25T08:00:04Z","request_id":"partial-request","model":"gpt-5-codex","usage":{"input_tokens":20,"output_tokens":8}"#,
        )
        .expect("append incomplete record");
        file.flush().expect("flush incomplete record");
        drop(file);

        let first = adapter
            .import_history_sync(&baseline_cursors)
            .expect("partial import");
        assert!(first.usage_events.is_empty());
        let partial_cursor = first
            .cursors
            .iter()
            .find(|cursor| cursor.resource_id.ends_with("simple_session.jsonl"))
            .expect("session cursor");
        assert!(partial_cursor.byte_offset < fs::metadata(&session_path).unwrap().len() as i64);

        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&session_path)
            .expect("reopen fixture for completion");
        file.write_all(b"}\n").expect("complete partial record");
        file.flush().expect("flush completed record");
        drop(file);
        let cursors = first
            .cursors
            .into_iter()
            .map(|cursor| (cursor.resource_id.clone(), cursor))
            .collect();
        let second = adapter
            .import_history_sync(&cursors)
            .expect("retry completed record");
        assert_eq!(second.usage_events.len(), 1);
        assert_eq!(
            second.usage_events[0].request_id.as_deref(),
            Some("partial-request")
        );
    }

    #[test]
    fn imports_codex_thread_name_from_the_session_index() {
        let home = fixture_home("indexed_session.jsonl");
        let index_path = home.path().join("session_index.jsonl");
        let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/codex/session_index.jsonl");
        fs::copy(fixture_path, index_path).expect("copy session index fixture");

        let adapter = CodexSessionAdapter::new(home.path());
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("indexed title import");
        assert_eq!(batch.sessions.len(), 1);
        assert_eq!(
            batch.sessions[0].title.as_deref(),
            Some("完成 Phase 4b 产品化验收")
        );
    }

    #[test]
    fn reads_nested_codex_ids_and_preserves_the_existing_filename_identity() {
        let home = fixture_home("rollout-indexed-session.jsonl");
        let index_path = home.path().join("session_index.jsonl");
        let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/codex/session_index.jsonl");
        fs::copy(fixture_path, index_path).expect("copy session index fixture");

        let adapter = CodexSessionAdapter::new(home.path());
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("nested indexed title import");
        assert_eq!(batch.sessions.len(), 1);
        assert_eq!(batch.usage_events.len(), 1);
        assert_eq!(
            batch.sessions[0].external_session_id.as_deref(),
            Some("rollout-indexed-session")
        );
        assert_eq!(
            batch.sessions[0].title.as_deref(),
            Some("完成 Phase 4b 产品化验收")
        );
        assert_eq!(
            batch.usage_events[0].session_id.as_deref(),
            Some(batch.sessions[0].id.as_str())
        );
    }

    #[test]
    fn backfills_indexed_title_when_the_log_cursor_is_at_eof() {
        let home = fixture_home("indexed_session.jsonl");
        let index_path = home.path().join("session_index.jsonl");
        let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/codex/session_index.jsonl");
        fs::copy(fixture_path, index_path).expect("copy session index fixture");

        let adapter = CodexSessionAdapter::new(home.path());
        let first = adapter
            .import_history_sync(&HashMap::new())
            .expect("initial indexed title import");
        let cursors = first
            .cursors
            .into_iter()
            .filter(|cursor| cursor.resource_id != SESSION_INDEX_RESOURCE_ID)
            .map(|cursor| (cursor.resource_id.clone(), cursor))
            .collect();

        let second = adapter
            .import_history_sync(&cursors)
            .expect("indexed title backfill");
        assert!(second.usage_events.is_empty());
        assert_eq!(second.sessions.len(), 1);
        assert_eq!(second.cursors.len(), 2);
        assert_eq!(
            second.sessions[0].title.as_deref(),
            Some("完成 Phase 4b 产品化验收")
        );

        let stable_cursors = second
            .cursors
            .into_iter()
            .map(|cursor| (cursor.resource_id.clone(), cursor))
            .collect();
        let third = adapter
            .import_history_sync(&stable_cursors)
            .expect("stable indexed title import");
        assert!(third.sessions.is_empty());
        assert!(third.usage_events.is_empty());
    }

    #[test]
    fn cumulative_snapshots_are_differenced_and_rollbacks_start_a_new_generation() {
        let home = fixture_home("duplicate_snapshot.jsonl");
        let adapter = CodexSessionAdapter::new(home.path());
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("cumulative import");
        assert_eq!(batch.usage_events.len(), 3);
        let inputs: Vec<_> = batch
            .usage_events
            .iter()
            .map(|event| event.usage.input_tokens_total)
            .collect();
        assert_eq!(inputs, vec![Some(100), Some(50), Some(90)]);
        assert!(
            batch
                .usage_events
                .iter()
                .all(|event| event.usage.input_tokens_total.is_none_or(|value| value > 0))
        );
        assert_eq!(batch.cursors[0].snapshot_generation, 1);
    }

    #[test]
    fn inherited_history_is_not_counted_as_a_child_event() {
        let home = fixture_home("subagent_inherited_history.jsonl");
        let adapter = CodexSessionAdapter::new(home.path());
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("subagent import");
        assert_eq!(batch.usage_events.len(), 2);
        assert_eq!(batch.sessions.len(), 2);
        let child = batch
            .sessions
            .iter()
            .find(|session| session.external_session_id.as_deref() == Some("child-session"))
            .expect("child session");
        assert!(child.parent_session_id.is_some());
    }

    #[test]
    fn malformed_records_are_skipped_without_aborting_the_file() {
        let home = fixture_home("malformed_lines.jsonl");
        let adapter = CodexSessionAdapter::new(home.path());
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("malformed import");
        assert_eq!(batch.usage_events.len(), 1);
        assert!(batch.skipped_records >= 2);
    }

    #[test]
    fn custom_path_detection_does_not_touch_the_real_codex_home() {
        let home = tempfile::tempdir().expect("temporary home");
        let adapter = CodexSessionAdapter::new(home.path());
        let result = adapter.detect_sync().expect("detect");
        assert!(!result.detected);
        assert!(default_codex_home().is_some());
    }

    #[test]
    fn incremental_token_count_rows_keep_the_header_session_identity() {
        use std::io::Write;

        // A real rollout file: the session id lives only in the header
        // `session_meta` line; the `token_count` rows that follow carry none.
        let home = tempfile::tempdir().expect("home");
        let sessions = home.path().join("sessions");
        fs::create_dir_all(&sessions).expect("sessions directory");
        let file = sessions.join("rollout.jsonl");
        fs::write(
            &file,
            "{\"type\":\"session_meta\",\"timestamp\":\"2026-07-25T09:00:00Z\",\"payload\":{\"id\":\"real-sess\",\"session_id\":\"real-sess\",\"cwd\":\"/p\",\"model\":\"gpt-5-codex\"}}\n\
             {\"type\":\"token_count\",\"timestamp\":\"2026-07-25T09:00:01Z\",\"total_token_usage\":{\"input_tokens\":100,\"cached_input_tokens\":20,\"output_tokens\":40,\"total_tokens\":140}}\n",
        )
        .expect("write rollout");

        let adapter = CodexSessionAdapter::new(home.path());
        let first = adapter
            .import_history_sync(&HashMap::new())
            .expect("first import");
        assert_eq!(first.usage_events.len(), 1);
        let session_id = first.usage_events[0]
            .session_id
            .clone()
            .expect("header session id");
        let cursors: HashMap<_, _> = first
            .cursors
            .iter()
            .map(|cursor| (cursor.resource_id.clone(), cursor.clone()))
            .collect();

        // Append another header-less token_count row and import incrementally.
        let mut appended = fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .expect("open rollout");
        appended
            .write_all(b"{\"type\":\"token_count\",\"timestamp\":\"2026-07-25T09:00:02Z\",\"total_token_usage\":{\"input_tokens\":150,\"cached_input_tokens\":30,\"output_tokens\":60,\"total_tokens\":210}}\n")
            .expect("append token_count");
        drop(appended);

        let second = adapter
            .import_history_sync(&cursors)
            .expect("incremental import");
        assert_eq!(second.usage_events.len(), 1);
        // The appended row stays on the SAME session instead of splitting off to
        // the file-stem fallback identity.
        assert_eq!(
            second.usage_events[0].session_id.as_deref(),
            Some(session_id.as_str())
        );
    }
}
