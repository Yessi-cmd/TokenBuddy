//! Read-only DeepSeek Harness (DSH) session JSONL adapter.
//!
//! DeepSeek Harness keeps one append-only JSONL artifact per session under its
//! session root (default `<DSH_HOME>/sessions`, where `DSH_HOME` is
//! `$DSH_HOME` or `~/.dsh`): `<root>/<project-dir>/<session-id>/session.jsonl`,
//! optionally Zstandard-compressed frame-by-frame as `session.jsonl.zstd`.
//! Line 0 is the session header (`type: "session"`), every following line is a
//! `{ type, seq, time, data }` event. Model-call token accounting travels on
//! `assistant/message` events inside `data.usage`; the route that served a
//! request is stated by the preceding `request/context` events.
//!
//! The adapter only reads those numeric fields: prompt text, completion text,
//! reasoning text, tool arguments and results never enter the domain model.
#![warn(missing_docs)]

use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    time::SystemTime,
};

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokenbuddy_domain::{
    AdapterCapabilities, AdapterDescriptor, AdapterError, AppKind, DetectionResult, EventSink,
    ImportBatch, ImportCursor, IngestSource, LauncherKind, NormalizedUsage, PrecisionLevel,
    ProviderRecord, SessionRecord, SourceHealth, SourceRecord, UsageAdapter, UsageEvent,
    WatcherHandle,
};

/// Stable id of this source, used for cursors, event hashes, and session ids.
pub const SOURCE_ID: &str = "dsh-session";
/// Adapter kind recorded on the source row.
pub const ADAPTER_TYPE: &str = "dsh_session";
/// Name shown in the UI.
pub const DISPLAY_NAME: &str = "DeepSeek Harness";
/// Static capabilities advertised to the Core registry.
pub const DESCRIPTOR: AdapterDescriptor = AdapterDescriptor {
    id: SOURCE_ID,
    adapter_type: ADAPTER_TYPE,
    display_name: DISPLAY_NAME,
    capabilities: AdapterCapabilities {
        usage_events: true,
        provider_context: true,
        quota_snapshots: false,
        file_watch: true,
    },
    read_only: true,
};

/// Directory inside the DSH home that holds the per-session artifacts.
pub const SESSIONS_DIRNAME: &str = "sessions";
/// Both physical encodings the JSONL backend writes.
const SESSION_FILENAMES: [&str; 2] = ["session.jsonl", "session.jsonl.zstd"];
/// A decompressed session log larger than this is refused rather than held in
/// memory: a healthy transcript is orders of magnitude smaller.
const MAX_PLAINTEXT_BYTES: usize = 256 * 1024 * 1024;
/// A compressed artifact larger than this is refused before decompression.
const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;

/// Why reading the DeepSeek Harness session logs failed.
#[derive(Debug, Error)]
pub enum DshAdapterError {
    /// A session file could not be read or decoded.
    #[error("failed to read DeepSeek Harness session files: {0}")]
    Io(#[from] io::Error),
    /// The configured home exists but is not a directory.
    #[error("DeepSeek Harness home is not a directory: {0}")]
    InvalidHome(PathBuf),
}

/// Reads a DeepSeek Harness home: the per-session JSONL artifacts under
/// `<home>/sessions`, plaintext or Zstandard-compressed.
#[derive(Debug, Clone)]
pub struct DshSessionAdapter {
    dsh_home: PathBuf,
}

impl DshSessionAdapter {
    /// An adapter for `dsh_home` (`~/.dsh` or a user-chosen equivalent).
    pub fn new(dsh_home: impl Into<PathBuf>) -> Self {
        Self {
            dsh_home: dsh_home.into(),
        }
    }

    /// The home this adapter reads.
    pub fn dsh_home(&self) -> &Path {
        &self.dsh_home
    }

    /// Where session artifacts live inside the home.
    pub fn sessions_root(&self) -> PathBuf {
        self.dsh_home.join(SESSIONS_DIRNAME)
    }

    /// Whether this home holds a session root.
    pub fn detect_sync(&self) -> DetectionResult {
        let detected = self.sessions_root().is_dir();
        DetectionResult {
            source_id: SOURCE_ID.to_owned(),
            detected,
            path_or_endpoint: Some(self.sessions_root().to_string_lossy().into_owned()),
            detected_version: detected.then(|| "jsonl-v0".to_owned()),
            message: Some(if detected {
                "DeepSeek Harness session directory detected".to_owned()
            } else {
                "DeepSeek Harness session directory was not found".to_owned()
            }),
        }
    }

    /// Current health of this source.
    pub fn health_sync(&self) -> SourceHealth {
        let detected = self.sessions_root().is_dir();
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

    /// Read every new event since `cursors` and return it as one batch.
    ///
    /// Files are skipped when size and modification time are unchanged; when
    /// they changed, the whole artifact is decoded and parsed, and only the
    /// plaintext bytes past the stored offset produce events. A truncation or
    /// a replaced header id resets the file to the beginning.
    pub fn import_history_sync(
        &self,
        cursors: &HashMap<String, ImportCursor>,
    ) -> Result<ImportBatch, DshAdapterError> {
        let root = self.sessions_root();
        if !root.exists() {
            return Ok(ImportBatch {
                source: Some(self.source_record("not_found")),
                ..ImportBatch::default()
            });
        }
        if !root.is_dir() {
            return Err(DshAdapterError::InvalidHome(root));
        }

        let mut files = Vec::new();
        collect_session_artifacts(&root, &mut files)?;
        files.sort();

        let mut batch = ImportBatch {
            source: Some(self.source_record("healthy")),
            ..ImportBatch::default()
        };
        let mut providers = HashSet::<String>::new();

        for path in files {
            let resource_id = resource_id(&root, &path);
            let cursor = cursors.get(&resource_id);
            let Some(parsed) = self.import_file(&path, &resource_id, cursor)? else {
                continue;
            };
            providers.extend(parsed.providers);
            batch.sessions.extend(parsed.sessions);
            batch.usage_events.extend(parsed.usage_events);
            batch.cursors.push(parsed.cursor);
            batch.skipped_records += parsed.skipped_records;
        }

        for provider in providers {
            batch.providers.push(ProviderRecord {
                id: provider.clone(),
                provider_family: "dsh".to_owned(),
                display_name: provider,
                upstream_url: None,
                launcher: Some(LauncherKind::Direct),
                source_id: Some(SOURCE_ID.to_owned()),
            });
        }

        Ok(batch)
    }

    fn source_record(&self, status: &str) -> SourceRecord {
        let timestamp = now();
        SourceRecord {
            id: DESCRIPTOR.id.to_owned(),
            adapter_type: DESCRIPTOR.adapter_type.to_owned(),
            display_name: DESCRIPTOR.display_name.to_owned(),
            path_or_endpoint: Some(self.sessions_root().to_string_lossy().into_owned()),
            enabled: true,
            detected_version: Some("jsonl-v0".to_owned()),
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
    ) -> Result<Option<ParsedFile>, DshAdapterError> {
        let metadata = fs::metadata(path)?;
        let file_size = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
        let modified_at = metadata.modified().ok().map(DateTime::<Utc>::from);
        let unchanged = cursor.is_some_and(|value| {
            value.file_size == Some(file_size)
                && value.modified_at.is_some()
                && value.modified_at == modified_at
                && value.byte_offset >= 0
        });
        if unchanged {
            return Ok(Some(ParsedFile {
                sessions: Vec::new(),
                usage_events: Vec::new(),
                providers: Vec::new(),
                cursor: cursor.expect("unchanged requires a cursor").clone(),
                skipped_records: 0,
            }));
        }

        if metadata.len() > MAX_ARTIFACT_BYTES {
            return Err(DshAdapterError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("session artifact {resource_id} exceeds the size limit"),
            )));
        }

        let compressed = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zstd"));
        let plaintext = read_plaintext(path, compressed)?;
        if plaintext.len() > MAX_PLAINTEXT_BYTES {
            return Err(DshAdapterError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("decompressed session {resource_id} exceeds the size limit"),
            )));
        }

        let Some(header) = first_line(&plaintext).and_then(parse_header) else {
            // Not a session artifact (e.g. an unrelated session.jsonl in the
            // tree): contribute nothing and leave no cursor behind.
            return Ok(None);
        };
        let header_id = header.id.clone();
        let stale = cursor.is_none_or(|value| {
            value.byte_offset < 0
                || u64::try_from(value.byte_offset)
                    .map_or(true, |offset| offset > plaintext.len() as u64)
                || value.last_session_id.as_deref() != Some(header_id.as_str())
        });
        let start_offset = if stale {
            0
        } else {
            cursor.map_or(0, |value| value.byte_offset.max(0)) as usize
        };

        let mut state = ParseState {
            current_provider: None,
            current_model: (!stale)
                .then(|| cursor.and_then(|value| value.last_model.clone()))
                .flatten(),
        };
        let session_id = session_domain_id(&header_id);
        let parent_session_id = header.parent_session.as_deref().map(session_domain_id);
        let created_at =
            DateTime::from_timestamp_millis(header.created_at_ms).unwrap_or_else(Utc::now);
        let query_source = if header.is_subagent {
            "subagent"
        } else {
            "main"
        };
        let mut ended_at = created_at;
        let mut usage_events = Vec::new();
        let mut providers = Vec::new();
        let mut skipped_records = 0;
        let offset = start_offset;
        let mut line_offset = offset;

        while line_offset < plaintext.len() {
            let line_start = line_offset;
            let Some(relative_end) = plaintext[line_start..]
                .iter()
                .position(|byte| *byte == b'\n')
            else {
                break;
            };
            let line_end = line_start + relative_end;
            line_offset = line_end + 1;
            let trimmed = plaintext[line_start..line_end].trim_ascii();
            if trimmed.is_empty() {
                continue;
            }

            let value: Value = match serde_json::from_slice(trimmed) {
                Ok(value) => value,
                Err(_) => {
                    skipped_records += 1;
                    continue;
                }
            };
            let Some(event_type) = value.get("type").and_then(Value::as_str) else {
                skipped_records += 1;
                continue;
            };
            let data = value.get("data");
            let time = value.get("time").and_then(Value::as_i64);

            match event_type {
                "request/context" => {
                    if let Some(data) = data.and_then(Value::as_object) {
                        if let Some(provider) = data
                            .get("provider")
                            .and_then(Value::as_str)
                            .filter(|value| !value.is_empty())
                        {
                            state.current_provider = Some(provider.to_owned());
                            providers.push(provider.to_owned());
                        }
                        if let Some(model) = data
                            .get("model")
                            .and_then(Value::as_str)
                            .filter(|value| !value.is_empty())
                        {
                            state.current_model = Some(model.to_owned());
                        }
                    }
                }
                "request/header" => {
                    let model = data
                        .and_then(|value| value.get("config"))
                        .and_then(|value| value.get("model"))
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty());
                    if state.current_model.is_none()
                        && let Some(model) = model
                    {
                        state.current_model = Some(model.to_owned());
                    }
                }
                "assistant/message" => {
                    let usage = data.and_then(|value| value.get("usage"));
                    let Some(usage) = usage.and_then(normalize_usage) else {
                        continue;
                    };
                    let Some(time) = time.and_then(DateTime::from_timestamp_millis) else {
                        skipped_records += 1;
                        continue;
                    };
                    let seq = value
                        .get("seq")
                        .and_then(Value::as_i64)
                        .unwrap_or(line_start as i64);
                    let provider_id = state.current_provider.clone();
                    let model = state.current_model.clone();
                    let raw_usage_json = sanitized_usage_json(&usage);
                    let raw_event_hash = hash_parts([
                        SOURCE_ID,
                        resource_id,
                        &header_id,
                        &line_start.to_string(),
                        &seq.to_string(),
                        &time.to_rfc3339(),
                        model.as_deref().unwrap_or_default(),
                        provider_id.as_deref().unwrap_or_default(),
                        &raw_usage_json.to_string(),
                    ]);
                    ended_at = ended_at.max(time);
                    usage_events.push(UsageEvent {
                        id: raw_event_hash.clone(),
                        occurred_at: time,
                        app: AppKind::DeepseekHarness,
                        launcher: LauncherKind::Direct,
                        ingest_source: IngestSource::SessionLog,
                        source_id: SOURCE_ID.to_owned(),
                        provider_id,
                        account_id: None,
                        session_id: Some(session_id.clone()),
                        parent_session_id: parent_session_id.clone(),
                        request_id: None,
                        response_id: None,
                        model,
                        query_source: Some(query_source.to_owned()),
                        usage,
                        provider_reported_cost: None,
                        estimated_cost: None,
                        currency: None,
                        http_status: None,
                        latency_ms: None,
                        success: None,
                        precision_token: PrecisionLevel::ExactSession,
                        precision_session: PrecisionLevel::ExactSession,
                        precision_provider: if state.current_provider.is_some() {
                            PrecisionLevel::ExactSession
                        } else {
                            PrecisionLevel::Unavailable
                        },
                        precision_account: PrecisionLevel::Unavailable,
                        raw_event_hash,
                        raw_usage_json: Some(raw_usage_json),
                    });
                }
                _ => {}
            }
        }

        let session = SessionRecord {
            id: session_id,
            external_session_id: Some(header_id.clone()),
            parent_session_id,
            app: AppKind::DeepseekHarness,
            launcher: Some(LauncherKind::Direct),
            project_path: header.cwd,
            title: None,
            started_at: Some(created_at),
            ended_at: Some(ended_at),
            source_id: Some(SOURCE_ID.to_owned()),
            created_at: now(),
            updated_at: now(),
        };

        let cursor = ImportCursor {
            source_id: SOURCE_ID.to_owned(),
            resource_id: resource_id.to_owned(),
            file_size: Some(file_size),
            modified_at,
            byte_offset: i64::try_from(line_offset).unwrap_or(i64::MAX),
            content_hash: None,
            last_cumulative_usage: None,
            snapshot_generation: 0,
            last_session_id: Some(header_id),
            last_model: state.current_model,
            updated_at: now(),
        };

        Ok(Some(ParsedFile {
            sessions: vec![session],
            usage_events,
            providers,
            cursor,
            skipped_records,
        }))
    }
}

impl UsageAdapter for DshSessionAdapter {
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
        Ok(self.detect_sync())
    }

    async fn import_history(
        &self,
        cursor: Option<ImportCursor>,
    ) -> Result<ImportBatch, AdapterError> {
        let cursors = cursor
            .map(|value| HashMap::from([(value.resource_id.clone(), value)]))
            .unwrap_or_default();
        self.import_history_sync(&cursors)
            .map_err(|error| AdapterError {
                message: error.to_string(),
            })
    }

    async fn start_watch(&self, _sink: EventSink) -> Result<WatcherHandle, AdapterError> {
        Err(AdapterError {
            message: "DeepSeek Harness sessions are imported by the Core file watcher".to_owned(),
        })
    }

    async fn health(&self) -> Result<SourceHealth, AdapterError> {
        Ok(self.health_sync())
    }
}

/// The platform's default DeepSeek Harness home: `$DSH_HOME` when the harness
/// exported it, otherwise `<USERPROFILE|HOME>/.dsh`.
pub fn default_dsh_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("DSH_HOME")
        && !home.is_empty()
    {
        return Some(PathBuf::from(home));
    }
    home_dir().map(|home| home.join(".dsh"))
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

#[derive(Debug)]
struct ParsedFile {
    sessions: Vec<SessionRecord>,
    usage_events: Vec<UsageEvent>,
    providers: Vec<String>,
    cursor: ImportCursor,
    skipped_records: usize,
}

#[derive(Debug, Default)]
struct ParseState {
    current_provider: Option<String>,
    current_model: Option<String>,
}

/// The immutable session header record: DSH line 0.
#[derive(Debug)]
struct SessionHeaderInfo {
    id: String,
    created_at_ms: i64,
    cwd: Option<String>,
    parent_session: Option<String>,
    is_subagent: bool,
}

fn first_line(plaintext: &[u8]) -> Option<&[u8]> {
    let end = plaintext.iter().position(|byte| *byte == b'\n')?;
    Some(&plaintext[..end])
}

fn parse_header(line: &[u8]) -> Option<SessionHeaderInfo> {
    let value: Value = serde_json::from_slice(line).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("session") {
        return None;
    }
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?
        .to_owned();
    let created_at_ms = value.get("createdAt").and_then(Value::as_i64)?;
    let cwd = value
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let parent_session = value
        .get("parentSession")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let is_subagent = value.get("origin").and_then(Value::as_str) == Some("subagent")
        || value
            .get("delegationDepth")
            .and_then(Value::as_i64)
            .is_some_and(|depth| depth > 0);
    Some(SessionHeaderInfo {
        id,
        created_at_ms,
        cwd,
        parent_session,
        is_subagent,
    })
}

/// Map DSH token accounting onto the shared vocabulary (spec §13).
///
/// DSH counts are DISJOINT: `inputTokens` is uncached input only, and cached
/// input is reported separately. When the provider reported no cache fields,
/// `inputTokens` is the whole prompt count, so it becomes the total and the
/// uncached split stays unavailable rather than being invented.
fn normalize_usage(usage: &Value) -> Option<NormalizedUsage> {
    let input = usage.get("inputTokens")?.as_u64()?;
    let output = usage.get("outputTokens")?.as_u64()?;
    let cache_read = usage.get("cacheReadTokens").and_then(Value::as_u64);
    let cache_write = usage.get("cacheWriteTokens").and_then(Value::as_u64);
    let reasoning = usage.get("reasoningTokens").and_then(Value::as_u64);
    let cache_known = cache_read.is_some() || cache_write.is_some();
    Some(NormalizedUsage {
        input_tokens_total: Some(
            input
                .checked_add(cache_read.unwrap_or(0))?
                .checked_add(cache_write.unwrap_or(0))?,
        ),
        input_tokens_uncached: cache_known.then_some(input),
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        output_tokens_total: Some(output),
        reasoning_tokens: reasoning,
        visible_output_tokens: Some(reasoning.map_or(output, |value| output.saturating_sub(value))),
    })
}

/// Numbers only — never prompt, completion, or reasoning text.
fn sanitized_usage_json(usage: &NormalizedUsage) -> Value {
    let mut object = Map::new();
    if let Some(value) = usage.input_tokens_total {
        object.insert("input_tokens_total".to_owned(), Value::from(value));
    }
    if let Some(value) = usage.input_tokens_uncached {
        object.insert("input_tokens_uncached".to_owned(), Value::from(value));
    }
    if let Some(value) = usage.cache_read_tokens {
        object.insert("cache_read_tokens".to_owned(), Value::from(value));
    }
    if let Some(value) = usage.cache_write_tokens {
        object.insert("cache_write_tokens".to_owned(), Value::from(value));
    }
    if let Some(value) = usage.output_tokens_total {
        object.insert("output_tokens_total".to_owned(), Value::from(value));
    }
    if let Some(value) = usage.reasoning_tokens {
        object.insert("reasoning_tokens".to_owned(), Value::from(value));
    }
    if let Some(value) = usage.visible_output_tokens {
        object.insert("visible_output_tokens".to_owned(), Value::from(value));
    }
    Value::Object(object)
}

fn read_plaintext(path: &Path, compressed: bool) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    if compressed {
        zstd::stream::decode_all(bytes.as_slice())
    } else {
        Ok(bytes)
    }
}

fn collect_session_artifacts(directory: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_session_artifacts(&path, files)?;
        } else if file_type.is_file() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if SESSION_FILENAMES
                .iter()
                .any(|candidate| name.eq_ignore_ascii_case(candidate))
            {
                files.push(path);
            }
        }
    }
    Ok(())
}

fn resource_id(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let with_slashes = relative.to_string_lossy().replace('\\', "/");
    // Both physical encodings describe the same logical session artifact, so a
    // config switch between plaintext and Zstandard keeps one stable identity
    // (and an artifact left behind in the old encoding deduplicates instead of
    // double-counting).
    with_slashes
        .strip_suffix(".zstd")
        .unwrap_or(&with_slashes)
        .to_owned()
}

/// Mint the session id exactly as other session-log adapters do, so a DSH
/// session id never collides with another source's.
fn session_domain_id(external_session_id: &str) -> String {
    format!("{SOURCE_ID}:{}", short_hash(external_session_id))
}

fn short_hash(value: &str) -> String {
    hash_parts([value]).chars().take(16).collect()
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
    use std::{collections::HashMap, fs, io::Write, path::Path};

    use tempfile::TempDir;
    use tokenbuddy_domain::{AppKind, ImportCursor};

    use super::{DshSessionAdapter, normalize_usage, session_domain_id};

    fn fixture_path(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/dsh")
            .join(name);
        fs::read_to_string(path).expect("fixture")
    }

    /// A home whose `sessions/--sanitized--/<session-id>/` holds the fixture.
    fn fixture_home(name: &str, session_id: &str) -> TempDir {
        let home = tempfile::tempdir().expect("home");
        let session_dir = home
            .path()
            .join("sessions")
            .join("--sanitized--")
            .join(session_id);
        fs::create_dir_all(&session_dir).expect("session dir");
        fs::write(session_dir.join("session.jsonl"), fixture_path(name)).expect("write fixture");
        home
    }

    fn import(home: &TempDir, cursors: &HashMap<String, ImportCursor>) -> super::ImportBatch {
        let adapter = DshSessionAdapter::new(home.path().to_owned());
        adapter.import_history_sync(cursors).expect("import")
    }

    #[test]
    fn maps_usage_and_route_context_into_events() {
        let home = fixture_home("simple_session.jsonl", "ses-dsh-main");
        let batch = import(&home, &HashMap::new());

        assert_eq!(batch.usage_events.len(), 2);
        assert_eq!(batch.sessions.len(), 1);
        assert!(
            batch
                .providers
                .iter()
                .any(|provider| provider.id == "deepseek-official")
        );
        assert_eq!(batch.skipped_records, 0);

        let session = &batch.sessions[0];
        assert_eq!(session.app, AppKind::DeepseekHarness);
        assert_eq!(session.external_session_id.as_deref(), Some("ses-dsh-main"));
        assert_eq!(session.project_path.as_deref(), Some("/sanitized/demo"));
        assert_eq!(session.id, session_domain_id("ses-dsh-main"));

        let first = &batch.usage_events[0];
        assert_eq!(first.model.as_deref(), Some("deepseek-v4-pro"));
        assert_eq!(first.provider_id.as_deref(), Some("deepseek-official"));
        assert_eq!(first.usage.input_tokens_uncached, Some(1200));
        assert_eq!(first.usage.cache_read_tokens, Some(800));
        assert_eq!(first.usage.cache_write_tokens, Some(0));
        assert_eq!(first.usage.input_tokens_total, Some(2000));
        assert_eq!(first.usage.output_tokens_total, Some(340));
        assert_eq!(first.usage.reasoning_tokens, Some(120));
        assert_eq!(first.usage.visible_output_tokens, Some(220));
        assert_eq!(first.query_source.as_deref(), Some("main"));

        // The second request reports no cache fields: the total stays the
        // reported input, the uncached split stays unavailable.
        let second = &batch.usage_events[1];
        assert_eq!(second.usage.input_tokens_total, Some(1500));
        assert_eq!(second.usage.input_tokens_uncached, None);
        assert_eq!(second.usage.cache_read_tokens, None);
        assert_eq!(second.usage.visible_output_tokens, Some(60));
        assert_eq!(second.model.as_deref(), Some("deepseek-v4-pro"));
    }

    #[test]
    fn zstd_artifacts_import_identically_to_plaintext() {
        let plain_home = fixture_home("simple_session.jsonl", "ses-dsh-main");
        let plain = import(&plain_home, &HashMap::new());

        let home = tempfile::tempdir().expect("home");
        let session_dir = home
            .path()
            .join("sessions")
            .join("--sanitized--")
            .join("ses-dsh-main");
        fs::create_dir_all(&session_dir).expect("session dir");
        let compressed = zstd::bulk::compress(fixture_path("simple_session.jsonl").as_bytes(), 3)
            .expect("compress");
        fs::write(session_dir.join("session.jsonl.zstd"), compressed).expect("write zstd");
        let zstd_batch = import(&home, &HashMap::new());

        assert_eq!(zstd_batch.usage_events, plain.usage_events);
        assert_eq!(zstd_batch.sessions.len(), plain.sessions.len());
        let zstd_session = &zstd_batch.sessions[0];
        let plain_session = &plain.sessions[0];
        assert_eq!(zstd_session.id, plain_session.id);
        assert_eq!(
            zstd_session.external_session_id,
            plain_session.external_session_id
        );
        assert_eq!(
            zstd_session.parent_session_id,
            plain_session.parent_session_id
        );
        assert_eq!(zstd_session.project_path, plain_session.project_path);
        assert_eq!(zstd_session.started_at, plain_session.started_at);
        assert_eq!(zstd_session.ended_at, plain_session.ended_at);
        assert_eq!(zstd_batch.providers, plain.providers);
    }

    #[test]
    fn repeated_import_is_idempotent() {
        let home = fixture_home("simple_session.jsonl", "ses-dsh-main");
        let first = import(&home, &HashMap::new());
        assert_eq!(first.usage_events.len(), 2);

        let cursors: HashMap<String, ImportCursor> = first
            .cursors
            .iter()
            .map(|cursor| (cursor.resource_id.clone(), cursor.clone()))
            .collect();
        let second = import(&home, &cursors);
        assert_eq!(second.usage_events.len(), 0);
        assert_eq!(second.sessions.len(), 0);
        assert_eq!(second.skipped_records, 0);
    }

    #[test]
    fn appended_records_are_imported_incrementally() {
        let home = fixture_home("simple_session.jsonl", "ses-dsh-main");
        let first = import(&home, &HashMap::new());
        let cursors: HashMap<String, ImportCursor> = first
            .cursors
            .iter()
            .map(|cursor| (cursor.resource_id.clone(), cursor.clone()))
            .collect();

        let artifact = home
            .path()
            .join("sessions")
            .join("--sanitized--")
            .join("ses-dsh-main")
            .join("session.jsonl");
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&artifact)
            .expect("open");
        writeln!(
            file,
            "{{\"type\":\"assistant/message\",\"seq\":13,\"time\":1786114983091,\"data\":{{\"turn\":3,\"step\":1,\"message\":{{\"role\":\"assistant\",\"content\":\"sanitized-late\"}},\"usage\":{{\"inputTokens\":60,\"outputTokens\":12}}}},\"surfaceOp\":\"append\"}}"
        )
        .expect("append");

        let second = import(&home, &cursors);
        assert_eq!(second.usage_events.len(), 1);
        assert_eq!(second.usage_events[0].usage.input_tokens_total, Some(60));
        assert_eq!(second.usage_events[0].usage.output_tokens_total, Some(12));

        // The old rows keep their identity: no duplicates, no re-import.
        let ids: std::collections::HashSet<_> = first
            .usage_events
            .iter()
            .map(|event| event.raw_event_hash.clone())
            .collect();
        assert!(ids.iter().all(|id| {
            !second
                .usage_events
                .iter()
                .any(|event| event.raw_event_hash == *id)
        }));
    }

    #[test]
    fn truncation_and_rotation_restart_the_file() {
        let home = fixture_home("simple_session.jsonl", "ses-dsh-main");
        let first = import(&home, &HashMap::new());
        let cursors: HashMap<String, ImportCursor> = first
            .cursors
            .iter()
            .map(|cursor| (cursor.resource_id.clone(), cursor.clone()))
            .collect();

        // A rotated artifact: same path, different session id, fewer lines.
        let artifact = home
            .path()
            .join("sessions")
            .join("--sanitized--")
            .join("ses-dsh-main")
            .join("session.jsonl");
        let rotated = fixture_path("subagent.jsonl").replace("ses-dsh-child", "ses-dsh-rotated");
        fs::write(&artifact, rotated).expect("rotate");

        let second = import(&home, &cursors);
        assert_eq!(second.usage_events.len(), 1);
        assert_eq!(second.sessions.len(), 1);
        assert_eq!(
            second.sessions[0].external_session_id.as_deref(),
            Some("ses-dsh-rotated")
        );
    }

    #[test]
    fn subagent_history_maps_parent_and_query_source() {
        let home = fixture_home("subagent.jsonl", "ses-dsh-child");
        let batch = import(&home, &HashMap::new());

        assert_eq!(batch.usage_events.len(), 1);
        let event = &batch.usage_events[0];
        assert_eq!(event.query_source.as_deref(), Some("subagent"));
        assert_eq!(
            event.parent_session_id.as_deref(),
            Some(session_domain_id("ses-dsh-main").as_str())
        );
        assert_eq!(
            event.session_id.as_deref(),
            Some(session_domain_id("ses-dsh-child").as_str())
        );
        let session = &batch.sessions[0];
        assert_eq!(
            session.parent_session_id.as_deref(),
            Some(session_domain_id("ses-dsh-main").as_str())
        );
    }

    #[test]
    fn malformed_lines_are_skipped_and_partial_tails_are_retried() {
        let home = fixture_home("simple_session.jsonl", "ses-dsh-main");
        let first = import(&home, &HashMap::new());
        let cursors: HashMap<String, ImportCursor> = first
            .cursors
            .iter()
            .map(|cursor| (cursor.resource_id.clone(), cursor.clone()))
            .collect();

        let artifact = home
            .path()
            .join("sessions")
            .join("--sanitized--")
            .join("ses-dsh-main")
            .join("session.jsonl");
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&artifact)
            .expect("open");
        writeln!(file, "not valid json {{").expect("append garbage");
        // A valid record whose trailing newline has not arrived yet.
        write!(
            file,
            "{{\"type\":\"assistant/message\",\"seq\":14,\"time\":1786114984091,\"data\":{{\"turn\":4,\"step\":1,\"message\":{{\"role\":\"assistant\"}},\"usage\":{{\"inputTokens\":7,\"outputTokens\":3}}}},\"surfaceOp\":\"append\"}}"
        )
        .expect("append partial");

        let second = import(&home, &cursors);
        assert_eq!(second.usage_events.len(), 0);
        assert_eq!(second.skipped_records, 1);
        let cursor = &second.cursors[0];
        // The cursor stops before the incomplete final line…
        let bytes = fs::read(&artifact).expect("read artifact");
        assert!(cursor.byte_offset < bytes.len() as i64);

        // …and the completed line imports on the next pass.
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&artifact)
            .expect("open");
        writeln!(file).expect("complete line");
        let cursors: HashMap<String, ImportCursor> = second
            .cursors
            .iter()
            .map(|cursor| (cursor.resource_id.clone(), cursor.clone()))
            .collect();
        let third = import(&home, &cursors);
        assert_eq!(third.usage_events.len(), 1);
        assert_eq!(third.usage_events[0].usage.input_tokens_total, Some(7));
    }

    #[test]
    fn prompt_and_completion_text_never_enter_the_domain_model() {
        let home = fixture_home("simple_session.jsonl", "ses-dsh-main");
        let batch = import(&home, &HashMap::new());

        for event in &batch.usage_events {
            let serialized = serde_json::to_string(event).expect("serialize event");
            assert!(!serialized.contains("sanitized-prompt-text"));
            assert!(!serialized.contains("sanitized-completion-text"));
            let raw = event.raw_usage_json.as_ref().expect("raw usage");
            assert!(!raw.to_string().contains("text"));
            assert!(!raw.to_string().contains("sanitized"));
        }
        assert_eq!(batch.sessions[0].title, None);
    }

    #[test]
    fn unrelated_files_in_the_tree_are_ignored() {
        let home = fixture_home("simple_session.jsonl", "ses-dsh-main");
        let noise_dir = home
            .path()
            .join("sessions")
            .join("--sanitized--")
            .join("noise");
        fs::create_dir_all(&noise_dir).expect("noise dir");
        fs::write(noise_dir.join("notes.jsonl"), "{\"type\":\"session\"}\n").expect("noise file");

        let batch = import(&home, &HashMap::new());
        assert_eq!(batch.usage_events.len(), 2);
        // No cursor is left behind for the non-artifact file.
        assert!(
            batch
                .cursors
                .iter()
                .all(|cursor| !cursor.resource_id.is_empty())
        );
    }

    #[test]
    fn missing_session_root_resolves_to_not_found_health() {
        let home = tempfile::tempdir().expect("home");
        let adapter = DshSessionAdapter::new(home.path().to_owned());
        let batch = adapter
            .import_history_sync(&HashMap::new())
            .expect("import");
        assert_eq!(
            batch
                .source
                .as_ref()
                .and_then(|source| source.health_status.as_deref()),
            Some("not_found")
        );
        assert_eq!(batch.usage_events.len(), 0);
    }

    #[test]
    fn usage_mapping_keeps_missing_cache_fields_unavailable() {
        let usage: serde_json::Value =
            serde_json::from_str(r#"{"inputTokens":10,"outputTokens":5,"reasoningTokens":2}"#)
                .expect("usage json");
        let normalized = normalize_usage(&usage).expect("normalize");
        assert_eq!(normalized.input_tokens_total, Some(10));
        assert_eq!(normalized.input_tokens_uncached, None);
        assert_eq!(normalized.cache_read_tokens, None);
        assert_eq!(normalized.visible_output_tokens, Some(3));

        let usage: serde_json::Value = serde_json::from_str(
            r#"{"inputTokens":10,"outputTokens":5,"cacheReadTokens":4,"cacheWriteTokens":1}"#,
        )
        .expect("usage json");
        let normalized = normalize_usage(&usage).expect("normalize");
        assert_eq!(normalized.input_tokens_total, Some(15));
        assert_eq!(normalized.input_tokens_uncached, Some(10));
        assert_eq!(normalized.cache_read_tokens, Some(4));
        assert_eq!(normalized.cache_write_tokens, Some(1));
        assert_eq!(normalized.visible_output_tokens, Some(5));
    }
}
