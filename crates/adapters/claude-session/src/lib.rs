//! Read-only Claude Code session JSONL adapter.
//!
//! Claude Code transcript files have changed shape over time. This adapter
//! keeps the schema-specific extraction in small versioned parsers and uses a
//! conservative fallback that only accepts explicit, stable fields. Message
//! bodies are deliberately never copied into the domain model or raw payload.

use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsStr,
    fs::{self, File},
    io::{self, BufRead, BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::SystemTime,
};

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokenbuddy_domain::{
    AdapterError, AppKind, DetectionResult, EventSink, ImportBatch, ImportCursor, IngestSource,
    LauncherKind, NormalizedUsage, PrecisionLevel, SessionRecord, SourceHealth, SourceRecord,
    UsageAdapter, UsageEvent, WatcherHandle,
};

pub const SOURCE_ID: &str = "claude-code-session";
pub const ADAPTER_TYPE: &str = "claude_session";
pub const DISPLAY_NAME: &str = "Claude Code Sessions";

#[derive(Debug, Error)]
pub enum ClaudeAdapterError {
    #[error("failed to read Claude Code session files: {0}")]
    Io(#[from] io::Error),
    #[error("Claude Code home is not a directory: {0}")]
    InvalidHome(PathBuf),
}

#[derive(Debug, Clone)]
pub struct ClaudeSessionAdapter {
    claude_home: PathBuf,
}

impl ClaudeSessionAdapter {
    pub fn new(claude_home: impl Into<PathBuf>) -> Self {
        Self {
            claude_home: claude_home.into(),
        }
    }

    pub fn claude_home(&self) -> &Path {
        &self.claude_home
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.claude_home.join("projects")
    }

    pub fn detect_sync(&self) -> Result<DetectionResult, ClaudeAdapterError> {
        let projects_dir = self.projects_dir();
        let detected = projects_dir.is_dir();
        Ok(DetectionResult {
            source_id: SOURCE_ID.to_owned(),
            detected,
            path_or_endpoint: Some(self.claude_home.to_string_lossy().into_owned()),
            detected_version: detected.then(|| "jsonl-v1/v2".to_owned()),
            message: if detected {
                Some("Claude Code project session directory detected".to_owned())
            } else {
                Some("Claude Code project session directory was not found".to_owned())
            },
        })
    }

    pub fn import_history_sync(
        &self,
        cursors: &HashMap<String, ImportCursor>,
    ) -> Result<ImportBatch, ClaudeAdapterError> {
        if self.claude_home.exists() && !self.claude_home.is_dir() {
            return Err(ClaudeAdapterError::InvalidHome(self.claude_home.clone()));
        }
        let projects_dir = self.projects_dir();
        if !projects_dir.exists() {
            return Ok(ImportBatch {
                source: Some(self.source_record("not_found")),
                ..ImportBatch::default()
            });
        }
        if !projects_dir.is_dir() {
            return Err(ClaudeAdapterError::InvalidHome(projects_dir));
        }

        let mut files = Vec::new();
        collect_jsonl_files(&projects_dir, &mut files)?;
        files.sort();

        let mut batch = ImportBatch {
            source: Some(self.source_record("healthy")),
            ..ImportBatch::default()
        };
        for path in files {
            let resource_id = resource_id(&self.claude_home, &path);
            let parsed = self.import_file(&path, &resource_id, cursors.get(&resource_id))?;
            batch.sessions.extend(parsed.sessions);
            batch.usage_events.extend(parsed.usage_events);
            batch.cursors.push(parsed.cursor);
            batch.skipped_records += parsed.skipped_records;
        }

        Ok(batch)
    }

    pub fn health_sync(&self) -> SourceHealth {
        let detected = self.projects_dir().is_dir();
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
            path_or_endpoint: Some(self.claude_home.to_string_lossy().into_owned()),
            enabled: true,
            detected_version: Some("jsonl-v1/v2/fallback".to_owned()),
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
    ) -> Result<ParsedFile, ClaudeAdapterError> {
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
            current_session_id: None,
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
                    // Keep an incomplete final JSONL record out of the cursor
                    // so a later append can be retried from its first byte.
                    if !has_newline {
                        offset = line_offset;
                        break;
                    }
                    skipped_records += 1;
                    continue;
                }
            };

            let parsed = parse_record(&value, &default_external_session_id, &state);
            if let Some(external_session_id) = &parsed.context.session_id {
                state.current_session_id = Some(external_session_id.clone());
                update_session(
                    &mut sessions,
                    &self.session_record(external_session_id, &parsed.context),
                    parsed.context.timestamp,
                );
            }

            if parsed.context.inherited_history {
                continue;
            }
            let Some(candidate) = parsed.usage else {
                continue;
            };
            let Some(timestamp) = parsed.context.timestamp else {
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

            let external_session_id = parsed
                .context
                .session_id
                .clone()
                .or_else(|| state.current_session_id.clone())
                .unwrap_or_else(|| default_external_session_id.clone());
            let session = self.session_record(&external_session_id, &parsed.context);
            update_session(&mut sessions, &session, Some(timestamp));
            let parent_session_id = parsed
                .context
                .parent_session_id
                .as_deref()
                .map(|value| self.session_id(value));
            let raw_usage_json = Some(candidate.value.clone());
            let raw_event_hash = event_hash(
                resource_id,
                line_offset,
                &parsed.context,
                &raw_usage_json,
                candidate.cumulative.then_some(state.snapshot_generation),
            );
            usage_events.push(UsageEvent {
                id: raw_event_hash.clone(),
                occurred_at: timestamp,
                app: AppKind::ClaudeCode,
                launcher: LauncherKind::Direct,
                ingest_source: IngestSource::SessionLog,
                source_id: SOURCE_ID.to_owned(),
                provider_id: None,
                account_id: None,
                session_id: Some(session.id),
                parent_session_id,
                request_id: parsed.context.request_id.clone(),
                response_id: parsed.context.response_id.clone(),
                model: parsed.context.model.clone(),
                query_source: parsed.context.query_source.clone(),
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
            updated_at: now(),
        };

        Ok(ParsedFile {
            sessions: sessions.into_values().collect(),
            usage_events,
            cursor,
            skipped_records,
        })
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
            app: AppKind::ClaudeCode,
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

impl UsageAdapter for ClaudeSessionAdapter {
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
            message: "Claude Code file watching is owned by the shared Core".to_owned(),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaudeParser {
    V1,
    V2,
    Fallback,
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

#[derive(Debug)]
struct ParsedRecord {
    context: ParseContext,
    usage: Option<UsageCandidate>,
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

fn resource_id(claude_home: &Path, path: &Path) -> String {
    path.strip_prefix(claude_home)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn parse_record(value: &Value, default_session_id: &str, state: &ParseState) -> ParsedRecord {
    let parser = parser_for(value);
    let record_type = first_string(value, &["type", "event_type", "eventType", "event"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    let session_id = explicit_string(
        value,
        &[
            "session_id",
            "sessionId",
            "conversation_id",
            "conversationId",
            "thread_id",
            "threadId",
        ],
    )
    .or_else(|| state.current_session_id.clone())
    .or_else(|| Some(default_session_id.to_owned()));
    let context = ParseContext {
        session_id,
        parent_session_id: explicit_string(
            value,
            &[
                "parent_session_id",
                "parentSessionId",
                "parent_session",
                "parentSession",
            ],
        ),
        project_path: explicit_string(
            value,
            &["project_path", "projectPath", "cwd", "working_directory"],
        ),
        title: explicit_string(
            value,
            &["title", "session_title", "sessionTitle", "summary", "slug"],
        ),
        model: explicit_string(value, &["model", "model_name", "modelName"])
            .or_else(|| nested_string(value, "message", &["model", "model_name", "modelName"])),
        request_id: explicit_string(value, &["request_id", "requestId"]),
        response_id: explicit_string(value, &["response_id", "responseId"])
            .or_else(|| nested_string(value, "message", &["id", "message_id", "messageId"])),
        query_source: explicit_string(value, &["query_source", "querySource"]),
        timestamp: timestamp_value(value),
        inherited_history: explicit_bool(
            value,
            &[
                "inherited",
                "is_inherited",
                "isInherited",
                "history_inherited",
            ],
        )
        .unwrap_or(false)
            || record_type.contains("inherited")
            || record_type.contains("history_copy"),
    };

    let usage = find_usage(value, parser);
    ParsedRecord { context, usage }
}

fn parser_for(value: &Value) -> ClaudeParser {
    let object = value.as_object();
    let record_type = first_string(value, &["type", "event_type", "eventType"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    if object.is_some_and(|object| {
        object.contains_key("sessionId")
            || object.contains_key("message")
            || record_type == "assistant"
            || record_type == "user"
            || record_type == "summary"
    }) {
        ClaudeParser::V1
    } else if object.is_some_and(|object| {
        object.contains_key("session_id")
            || object.contains_key("event")
            || object.contains_key("event_type")
            || object.contains_key("project_path")
    }) {
        ClaudeParser::V2
    } else {
        ClaudeParser::Fallback
    }
}

fn find_usage(value: &Value, parser: ClaudeParser) -> Option<UsageCandidate> {
    let cumulative_keys = [
        "total_token_usage",
        "totalTokenUsage",
        "cumulative_usage",
        "cumulativeUsage",
    ];
    let usage = match parser {
        ClaudeParser::V1 => nested_object(
            value,
            "message",
            &[
                "usage",
                "input_tokens",
                "inputTokens",
                "output_tokens",
                "outputTokens",
            ],
        )
        .or_else(|| direct_object(value, "usage")),
        ClaudeParser::V2 | ClaudeParser::Fallback => {
            direct_object(value, "usage").or_else(|| nested_object(value, "payload", &["usage"]))
        }
    };
    for key in cumulative_keys {
        if let Some(candidate) = direct_object(value, key) {
            return Some(UsageCandidate {
                value: candidate,
                cumulative: true,
            });
        }
    }
    usage
        .filter(has_usage_field)
        .map(|candidate| UsageCandidate {
            value: candidate,
            cumulative: false,
        })
}

fn normalize_usage(value: &Value) -> Option<NormalizedUsage> {
    let input_tokens_uncached = number(value, &["input_tokens", "inputTokens"]);
    let cache_read_tokens = number(
        value,
        &[
            "cache_read_input_tokens",
            "cacheReadInputTokens",
            "cache_read_tokens",
            "cacheReadTokens",
        ],
    );
    let cache_write_tokens = number(
        value,
        &[
            "cache_creation_input_tokens",
            "cacheCreationInputTokens",
            "cache_write_tokens",
            "cacheWriteTokens",
        ],
    );
    // Anthropic's input_tokens excludes cache reads and writes. The total is
    // only known when all three source fields are present; missing fields stay
    // unavailable instead of being silently treated as zero.
    let input_tokens_total = input_tokens_uncached
        .zip(cache_write_tokens)
        .and_then(|(input, cache_write)| input.checked_add(cache_write))
        .zip(cache_read_tokens)
        .and_then(|(total, cache_read)| total.checked_add(cache_read));
    let output_tokens_total = number(value, &["output_tokens", "outputTokens"]);
    let reasoning_tokens = number(value, &["reasoning_tokens", "reasoningTokens"]);
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

fn has_usage_field(value: &Value) -> bool {
    value.as_object().is_some_and(|object| {
        [
            "input_tokens",
            "inputTokens",
            "cache_creation_input_tokens",
            "cacheCreationInputTokens",
            "cache_read_input_tokens",
            "cacheReadInputTokens",
            "output_tokens",
            "outputTokens",
        ]
        .iter()
        .any(|key| object.contains_key(*key))
    })
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

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    value.as_object().and_then(|object| {
        keys.iter()
            .find_map(|key| object.get(*key)?.as_str().map(str::to_owned))
    })
}

fn explicit_string(value: &Value, keys: &[&str]) -> Option<String> {
    first_string(value, keys).or_else(|| {
        value
            .get("payload")
            .and_then(|payload| first_string(payload, keys))
    })
}

fn nested_string(value: &Value, object_key: &str, keys: &[&str]) -> Option<String> {
    value
        .get(object_key)
        .and_then(|nested| first_string(nested, keys))
}

fn explicit_bool(value: &Value, keys: &[&str]) -> Option<bool> {
    value
        .as_object()
        .and_then(|object| keys.iter().find_map(|key| object.get(*key)?.as_bool()))
        .or_else(|| {
            value
                .get("payload")
                .and_then(|payload| explicit_bool(payload, keys))
        })
}

fn direct_object(value: &Value, key: &str) -> Option<Value> {
    value
        .get(key)
        .filter(|candidate| candidate.is_object())
        .cloned()
}

fn nested_object(value: &Value, object_key: &str, keys: &[&str]) -> Option<Value> {
    let nested = value.get(object_key)?.as_object()?;
    if let Some(candidate) = keys.iter().find_map(|key| {
        nested
            .get(*key)
            .filter(|candidate| candidate.is_object())
            .cloned()
    }) {
        return Some(candidate);
    }
    if object_key == "payload"
        && let Some(message) = nested.get("message").and_then(Value::as_object)
    {
        return keys.iter().find_map(|key| {
            message
                .get(*key)
                .filter(|candidate| candidate.is_object())
                .cloned()
        });
    }
    None
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

fn event_hash(
    resource_id: &str,
    offset: u64,
    context: &ParseContext,
    raw_usage_json: &Option<Value>,
    snapshot_generation: Option<i64>,
) -> String {
    let raw_usage = raw_usage_json
        .as_ref()
        .map_or_else(String::new, |value| value.to_string());
    let identity = context
        .request_id
        .as_deref()
        .or(context.response_id.as_deref());
    let fallback = format!("{resource_id}:{offset}");
    let parts = [
        SOURCE_ID.to_owned(),
        identity.unwrap_or(&fallback).to_owned(),
        context.session_id.clone().unwrap_or_default(),
        context.parent_session_id.clone().unwrap_or_default(),
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
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn now() -> DateTime<Utc> {
    DateTime::<Utc>::from(SystemTime::now())
}

fn adapter_error(error: ClaudeAdapterError) -> AdapterError {
    AdapterError {
        message: error.to_string(),
    }
}

pub fn default_claude_home() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let user_profile = std::env::var_os("USERPROFILE");
        let home_drive = std::env::var_os("HOMEDRIVE");
        let home_path = std::env::var_os("HOMEPATH");
        claude_home_from_env(
            user_profile.as_deref(),
            home_drive.as_deref(),
            home_path.as_deref(),
        )
    }

    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude"))
    }
}

#[cfg(windows)]
fn claude_home_from_env(
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
        .map(|home| home.join(".claude"))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs::{self, OpenOptions},
        io::Write,
        path::Path,
    };

    use tempfile::TempDir;
    use tokenbuddy_domain::PrecisionLevel;

    use super::{ClaudeSessionAdapter, default_claude_home};

    fn fixture_home(fixture: &str) -> (TempDir, std::path::PathBuf) {
        let home = tempfile::tempdir().expect("temporary Claude home");
        let project = home.path().join("projects").join("sanitized-project");
        fs::create_dir_all(&project).expect("project directory");
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/claude")
            .join(fixture);
        let destination = project.join(fixture);
        fs::copy(source, &destination).expect("copy fixture");
        (home, destination)
    }

    #[test]
    fn imports_v1_message_usage_without_persisting_message_body() {
        let (home, _) = fixture_home("simple_session.jsonl");
        let adapter = ClaudeSessionAdapter::new(home.path());
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import succeeds");

        assert_eq!(batch.usage_events.len(), 2);
        assert_eq!(batch.sessions.len(), 1);
        assert_eq!(batch.usage_events[0].usage.input_tokens_total, Some(150));
        assert_eq!(batch.usage_events[0].usage.input_tokens_uncached, Some(100));
        assert_eq!(batch.usage_events[0].usage.cache_read_tokens, Some(30));
        assert_eq!(batch.usage_events[0].usage.cache_write_tokens, Some(20));
        assert_eq!(batch.usage_events[0].usage.output_tokens_total, Some(40));
        assert_eq!(
            batch.usage_events[0].model.as_deref(),
            Some("claude-3-7-sonnet")
        );
        assert_eq!(
            batch.sessions[0].title.as_deref(),
            Some("sanitized-claude-session")
        );
        assert_eq!(
            batch.usage_events[0].precision_token,
            PrecisionLevel::ExactSession
        );
        let raw_usage = batch.usage_events[0]
            .raw_usage_json
            .as_ref()
            .expect("raw usage");
        assert!(raw_usage.get("input_tokens").is_some());
        assert!(raw_usage.to_string().contains("cache_read_input_tokens"));
        assert!(!raw_usage.to_string().contains("REDACTED_PROMPT"));
        assert!(!raw_usage.to_string().contains("costUSD"));
    }

    #[test]
    fn imports_v2_variant_and_keeps_missing_total_unavailable() {
        let (home, _) = fixture_home("schema_variant.jsonl");
        let adapter = ClaudeSessionAdapter::new(home.path());
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import succeeds");

        assert_eq!(batch.usage_events.len(), 2);
        assert_eq!(batch.usage_events[0].usage.input_tokens_total, Some(110));
        assert_eq!(batch.usage_events[0].usage.input_tokens_uncached, Some(90));
        assert_eq!(batch.usage_events[0].usage.cache_read_tokens, Some(15));
        assert_eq!(batch.usage_events[0].usage.cache_write_tokens, Some(5));
        assert_eq!(batch.usage_events[1].usage.input_tokens_total, None);
        assert_eq!(batch.usage_events[1].usage.input_tokens_uncached, Some(50));
        assert_eq!(batch.usage_events[1].usage.output_tokens_total, Some(10));
        assert_eq!(batch.usage_events[0].query_source.as_deref(), Some("cli"));
        assert_eq!(batch.sessions[0].title.as_deref(), Some("Variant session"));
    }

    #[test]
    fn imports_subagent_usage_but_skips_inherited_history() {
        let (home, _) = fixture_home("subagent.jsonl");
        let adapter = ClaudeSessionAdapter::new(home.path());
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import succeeds");

        assert_eq!(batch.usage_events.len(), 2);
        assert_eq!(batch.sessions.len(), 2);
        let child = batch
            .usage_events
            .iter()
            .find(|event| event.session_id != batch.usage_events[0].session_id)
            .expect("child event");
        assert!(child.parent_session_id.is_some());
        assert!(
            !batch
                .usage_events
                .iter()
                .any(|event| event.occurred_at.to_rfc3339() == "2026-07-25T10:00:03+00:00")
        );
    }

    #[test]
    fn malformed_records_are_skipped_without_aborting_the_file() {
        let (home, _) = fixture_home("malformed_lines.jsonl");
        let adapter = ClaudeSessionAdapter::new(home.path());
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import succeeds");

        assert_eq!(batch.usage_events.len(), 1);
        assert_eq!(batch.skipped_records, 3);
    }

    #[test]
    fn repeated_import_uses_cursor_and_does_not_emit_duplicate_events() {
        let (home, _) = fixture_home("simple_session.jsonl");
        let adapter = ClaudeSessionAdapter::new(home.path());
        let first = adapter
            .import_history_sync(&HashMap::new())
            .expect("first import");
        let cursors = first
            .cursors
            .iter()
            .map(|cursor| (cursor.resource_id.clone(), cursor.clone()))
            .collect();
        let second = adapter
            .import_history_sync(&cursors)
            .expect("second import");

        assert_eq!(first.usage_events.len(), 2);
        assert!(second.usage_events.is_empty());
        assert_eq!(second.sessions.len(), 0);
    }

    #[test]
    fn incomplete_final_record_is_retried_after_append() {
        let (home, path) = fixture_home("schema_variant.jsonl");
        let original = fs::read_to_string(&path).expect("read fixture");
        let split_at = original.rfind('\n').expect("fixture newline");
        let complete = &original[..split_at + 1];
        let partial = r#"{"event":"assistant_response","session_id":"partial-session","timestamp":"2026-07-25T09:00:03Z","usage":{"input_tokens":20,"cache_creation_input_tokens":1,"cache_read_input_tokens":2,"output_tokens":3}"#;
        fs::write(&path, format!("{complete}{partial}")).expect("write partial record");
        let adapter = ClaudeSessionAdapter::new(home.path());
        let first = adapter
            .import_history_sync(&HashMap::new())
            .expect("partial import");
        let cursor = first.cursors[0].clone();
        assert_eq!(first.usage_events.len(), 2);
        assert_eq!(cursor.byte_offset as usize, complete.len());

        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open partial fixture");
        writeln!(file, "}}").expect("finish partial record");
        let cursors = HashMap::from([(cursor.resource_id.clone(), cursor)]);
        let second = adapter.import_history_sync(&cursors).expect("retry import");
        assert_eq!(second.usage_events.len(), 1);
        assert!(
            second.usage_events[0]
                .session_id
                .as_deref()
                .is_some_and(|id| id.starts_with("claude-code-session:"))
        );
    }

    #[test]
    fn rotation_resets_cursor_and_imports_new_file() {
        let (home, path) = fixture_home("simple_session.jsonl");
        let adapter = ClaudeSessionAdapter::new(home.path());
        let first = adapter
            .import_history_sync(&HashMap::new())
            .expect("first import");
        let cursor = first.cursors[0].clone();
        fs::write(
            &path,
            "{\"type\":\"assistant\",\"sessionId\":\"rotated-session\",\"timestamp\":\"2026-07-26T08:00:00Z\",\"message\":{\"id\":\"rotated-message\",\"model\":\"claude-3-7-sonnet\",\"usage\":{\"input_tokens\":1,\"cache_creation_input_tokens\":2,\"cache_read_input_tokens\":3,\"output_tokens\":4}}}\n",
        )
        .expect("rotate fixture");
        let cursors = HashMap::from([(cursor.resource_id.clone(), cursor)]);
        let second = adapter
            .import_history_sync(&cursors)
            .expect("rotation import");
        assert_eq!(second.usage_events.len(), 1);
        assert_eq!(second.usage_events[0].usage.input_tokens_total, Some(6));
    }

    #[test]
    fn default_home_is_derived_without_reading_the_real_projects_directory() {
        assert!(default_claude_home().is_some());
    }
}
