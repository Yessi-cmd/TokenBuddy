//! A small, optional OTLP/HTTP traces receiver for local coding-agent usage.
//!
//! The receiver deliberately has a narrow boundary: it listens only on
//! `127.0.0.1`, accepts `/v1/traces`, extracts numeric usage and stable span
//! identities, and hands a sanitized [`ImportBatch`] to the Core. It does not
//! persist anything, inspect prompt bodies, or depend on a collector,
//! Prometheus, Loki, Docker, or a remote service.
#![warn(missing_docs)]

use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokenbuddy_domain::{
    AdapterCapabilities, AdapterDescriptor, AppKind, ImportBatch, IngestSource, LauncherKind,
    NormalizedUsage, PrecisionLevel, ProviderRecord, SessionRecord, SourceRecord, UsageEvent,
};

/// Stable source id for the optional local OTLP receiver.
pub const SOURCE_ID: &str = "otel-http";
/// Adapter type shown in the source diagnostics.
pub const ADAPTER_TYPE: &str = "otel_http";
/// Display name shown in the source diagnostics.
pub const DISPLAY_NAME: &str = "OpenTelemetry (loopback)";

/// Static descriptor for the local receiver.
pub const DESCRIPTOR: AdapterDescriptor = AdapterDescriptor {
    id: SOURCE_ID,
    adapter_type: ADAPTER_TYPE,
    display_name: DISPLAY_NAME,
    capabilities: AdapterCapabilities {
        usage_events: true,
        provider_context: true,
        quota_snapshots: false,
        file_watch: false,
    },
    // This source receives locally emitted observations rather than reading a
    // third-party database. It is therefore not an external read-only adapter.
    read_only: false,
};

const MAX_HEADERS: usize = 16 * 1024;
const MAX_BODY: usize = 8 * 1024 * 1024;

/// Callback used by the receiver to hand an already-normalized batch to Core.
pub type BatchSink = Arc<dyn Fn(ImportBatch) + Send + Sync + 'static>;

/// Errors that can prevent the optional receiver from binding.
#[derive(Debug, Error)]
pub enum OtelReceiverError {
    /// The loopback socket could not be opened.
    #[error("无法绑定本地 OTLP 接收器：{0}")]
    Io(#[from] io::Error),
}

/// A loopback-only OTLP/HTTP receiver owned by the application Core.
pub struct OtelReceiver {
    stop: Arc<AtomicBool>,
    wake_address: String,
    endpoint: String,
    port: u16,
    worker: Option<JoinHandle<()>>,
}

impl OtelReceiver {
    /// Start listening on `127.0.0.1:{port}`.
    ///
    /// Port `0` is accepted for tests and asks the operating system to choose a
    /// free port. A bind failure is returned to the caller so the Core can keep
    /// running with OTel disabled and surface the limitation as a warning.
    pub fn start(port: u16, sink: BatchSink) -> Result<Self, OtelReceiverError> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let actual_port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let wake_address = format!("127.0.0.1:{actual_port}");
        let endpoint = format!("http://{wake_address}/v1/traces");
        let thread_endpoint = endpoint.clone();
        let worker = thread::Builder::new()
            .name("tokenbuddy-otel".to_owned())
            .spawn(move || run_server(listener, thread_stop, sink, thread_endpoint))?;

        Ok(Self {
            stop,
            wake_address,
            endpoint,
            port: actual_port,
            worker: Some(worker),
        })
    }

    /// The loopback OTLP traces endpoint, including the selected port.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The bound loopback port.
    pub const fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for OtelReceiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the non-blocking accept loop immediately instead of waiting for
        // its short polling interval. This is loopback-only and carries no
        // application data.
        let _ = TcpStream::connect(&self.wake_address);
        if let Some(worker) = self.worker.take()
            && worker.thread().id() != thread::current().id()
        {
            let _ = worker.join();
        }
    }
}

fn run_server(listener: TcpListener, stop: Arc<AtomicBool>, sink: BatchSink, endpoint: String) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _peer)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                handle_connection(&mut stream, &sink, &endpoint);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) if stop.load(Ordering::SeqCst) => break,
            Err(_) => thread::sleep(Duration::from_millis(25)),
        }
    }
}

fn handle_connection(stream: &mut TcpStream, sink: &BatchSink, endpoint: &str) {
    let request = match read_request(stream) {
        Ok(request) => request,
        Err(_) => {
            let _ = write_response(stream, 400, "text/plain; charset=utf-8", b"bad request");
            return;
        }
    };

    if request.method != "POST" {
        let _ = write_response(
            stream,
            405,
            "text/plain; charset=utf-8",
            b"method not allowed",
        );
        return;
    }
    if request.path != "/v1/traces" {
        let _ = write_response(stream, 404, "text/plain; charset=utf-8", b"not found");
        return;
    }

    let parsed = if request
        .content_type
        .as_deref()
        .is_some_and(|value| value.contains("json"))
    {
        serde_json::from_slice::<Value>(&request.body)
            .map_err(ParseError::from)
            .and_then(|value| parse_json_export(&value))
    } else {
        parse_protobuf_export(&request.body)
    };

    match parsed {
        Ok(spans) => {
            sink(batch_from_spans(spans, endpoint));
            let _ = write_response(stream, 200, "application/x-protobuf", &[]);
        }
        Err(_) => {
            // Do not echo the payload or a parser-derived string: an exporter
            // may have put sensitive data in an unknown attribute.
            let _ = write_response(
                stream,
                400,
                "text/plain; charset=utf-8",
                b"invalid OTLP payload",
            );
        }
    }
}

struct HttpRequest {
    method: String,
    path: String,
    content_type: Option<String>,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, ParseError> {
    let mut buffer = Vec::with_capacity(4096);
    let header_end = loop {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(ParseError::Invalid(
                "connection closed before headers".to_owned(),
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > MAX_HEADERS + MAX_BODY {
            return Err(ParseError::Invalid("request too large".to_owned()));
        }
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break index;
        }
        if buffer.len() > MAX_HEADERS {
            return Err(ParseError::Invalid("headers too large".to_owned()));
        }
    };

    let header_text = std::str::from_utf8(&buffer[..header_end])
        .map_err(|_| ParseError::Invalid("headers are not UTF-8".to_owned()))?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| ParseError::Invalid("missing request line".to_owned()))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| ParseError::Invalid("missing method".to_owned()))?
        .to_owned();
    let path = request_parts
        .next()
        .ok_or_else(|| ParseError::Invalid("missing path".to_owned()))?
        .to_owned();

    let mut content_length = None;
    let mut content_type = None;
    let mut transfer_encoding = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                content_length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| ParseError::Invalid("invalid content length".to_owned()))?,
                );
            }
            "content-type" => content_type = Some(value.trim().to_ascii_lowercase()),
            "transfer-encoding" => transfer_encoding = Some(value.trim().to_ascii_lowercase()),
            _ => {}
        }
    }
    if transfer_encoding.is_some_and(|value| value != "identity") {
        return Err(ParseError::Invalid(
            "chunked transfer is unsupported".to_owned(),
        ));
    }

    let content_length = content_length.unwrap_or(0);
    if content_length > MAX_BODY {
        return Err(ParseError::Invalid("body too large".to_owned()));
    }
    let body_start = header_end + 4;
    let mut body = buffer.get(body_start..).unwrap_or_default().to_vec();
    if body.len() > content_length {
        body.truncate(content_length);
    }
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let mut chunk = vec![0_u8; remaining.min(8192)];
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(ParseError::Invalid("body ended early".to_owned()));
        }
        body.extend_from_slice(&chunk[..read]);
    }

    Ok(HttpRequest {
        method,
        path: path.split('?').next().unwrap_or(&path).to_owned(),
        content_type,
        body,
    })
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

#[derive(Debug, Error)]
enum ParseError {
    #[error("invalid payload: {0}")]
    Invalid(String),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone)]
enum AttributeValue {
    String(String),
    Integer(i64),
    Float(f64),
    Bool(bool),
}

impl AttributeValue {
    fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            Self::Float(value) if value.is_finite() && value.fract() == 0.0 => {
                (*value >= i64::MIN as f64 && *value <= i64::MAX as f64).then_some(*value as i64)
            }
            _ => None,
        }
    }

    fn as_u64(&self) -> Option<u64> {
        self.as_i64().and_then(|value| u64::try_from(value).ok())
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
struct SpanData {
    trace_id: String,
    span_id: String,
    name: Option<String>,
    start_nanos: Option<u64>,
    end_nanos: Option<u64>,
    status_code: Option<u64>,
    attributes: BTreeMap<String, AttributeValue>,
}

fn batch_from_spans(spans: Vec<SpanData>, endpoint: &str) -> ImportBatch {
    let now = Utc::now();
    let mut batch = ImportBatch {
        source: Some(SourceRecord {
            id: SOURCE_ID.to_owned(),
            adapter_type: ADAPTER_TYPE.to_owned(),
            display_name: DISPLAY_NAME.to_owned(),
            path_or_endpoint: Some(endpoint.to_owned()),
            enabled: true,
            detected_version: Some("otlp-http-v1".to_owned()),
            health_status: Some("healthy".to_owned()),
            last_success_at: Some(now),
            last_error: None,
            created_at: now,
            updated_at: now,
        }),
        ..ImportBatch::default()
    };
    let mut providers = BTreeMap::<String, ProviderRecord>::new();
    let mut sessions = BTreeMap::<String, SessionRecord>::new();
    for span in spans {
        let Some(event) = usage_event_from_span(&span, now, &mut providers) else {
            batch.skipped_records += 1;
            continue;
        };
        if let Some(session_id) = event.session_id.clone() {
            let session = SessionRecord {
                id: session_id.clone(),
                external_session_id: first_text(
                    &span.attributes,
                    &[
                        "tokenbuddy.session.id",
                        "gen_ai.conversation.id",
                        "gen_ai.session.id",
                        "session.id",
                        "conversation.id",
                    ],
                ),
                parent_session_id: event.parent_session_id.clone(),
                app: event.app,
                launcher: Some(event.launcher),
                project_path: None,
                title: None,
                started_at: Some(event.occurred_at),
                ended_at: Some(event.occurred_at),
                source_id: Some(SOURCE_ID.to_owned()),
                created_at: now,
                updated_at: now,
            };
            sessions
                .entry(session_id)
                .and_modify(|current| {
                    current.started_at = match (current.started_at, session.started_at) {
                        (Some(current), Some(incoming)) => Some(current.min(incoming)),
                        (current, incoming) => current.or(incoming),
                    };
                    current.ended_at = match (current.ended_at, session.ended_at) {
                        (Some(current), Some(incoming)) => Some(current.max(incoming)),
                        (current, incoming) => current.or(incoming),
                    };
                    current.parent_session_id = current
                        .parent_session_id
                        .clone()
                        .or_else(|| session.parent_session_id.clone());
                    current.updated_at = now;
                })
                .or_insert(session);
        }
        batch.usage_events.push(event);
    }
    batch.providers = providers.into_values().collect();
    batch.sessions = sessions.into_values().collect();
    batch
}

fn usage_event_from_span(
    span: &SpanData,
    receive_time: DateTime<Utc>,
    providers: &mut BTreeMap<String, ProviderRecord>,
) -> Option<UsageEvent> {
    let app = app_from_attributes(&span.attributes);
    let request_id = first_text(
        &span.attributes,
        &[
            "tokenbuddy.request.id",
            "gen_ai.request.id",
            "gen_ai.request_id",
            "llm.request.id",
            "request.id",
        ],
    );
    let response_id = first_text(
        &span.attributes,
        &[
            "tokenbuddy.response.id",
            "gen_ai.response.id",
            "gen_ai.response_id",
            "llm.response.id",
            "response.id",
        ],
    );
    let model = first_text(
        &span.attributes,
        &[
            "gen_ai.request.model",
            "gen_ai.request_model",
            "llm.request.model",
            "model",
        ],
    );
    let session_external_id = first_text(
        &span.attributes,
        &[
            "tokenbuddy.session.id",
            "gen_ai.conversation.id",
            "gen_ai.session.id",
            "session.id",
            "conversation.id",
        ],
    );
    let parent_external_id = first_text(
        &span.attributes,
        &[
            "tokenbuddy.parent_session.id",
            "gen_ai.parent_session.id",
            "parent.session.id",
        ],
    );
    let provider = first_text(
        &span.attributes,
        &[
            "tokenbuddy.provider",
            "gen_ai.provider.name",
            "gen_ai.system",
            "llm.system",
        ],
    );
    let provider_id = provider.as_deref().map(|value| {
        let (id, family, display_name) = provider_identity(value);
        providers
            .entry(id.clone())
            .or_insert_with(|| ProviderRecord {
                id: id.clone(),
                provider_family: family,
                display_name,
                upstream_url: None,
                launcher: None,
                source_id: None,
            });
        id
    });

    let usage = NormalizedUsage {
        input_tokens_total: first_u64(
            &span.attributes,
            &[
                "gen_ai.usage.input_tokens",
                "gen_ai.usage.prompt_tokens",
                "gen_ai.response.usage.input_tokens",
                "llm.usage.prompt_tokens",
            ],
        ),
        input_tokens_uncached: first_u64(
            &span.attributes,
            &[
                "gen_ai.usage.input_tokens_uncached",
                "llm.usage.input_tokens_uncached",
            ],
        ),
        cache_read_tokens: first_u64(
            &span.attributes,
            &[
                "gen_ai.usage.cache_read_input_tokens",
                "gen_ai.usage.cache_read_tokens",
                "gen_ai.usage.cache_read.input_tokens",
                "llm.usage.cache_read_input_tokens",
            ],
        ),
        cache_write_tokens: first_u64(
            &span.attributes,
            &[
                "gen_ai.usage.cache_creation_input_tokens",
                "gen_ai.usage.cache_write_input_tokens",
                "gen_ai.usage.cache_write_tokens",
                "gen_ai.usage.cache_write.input_tokens",
                "llm.usage.cache_creation_input_tokens",
            ],
        ),
        output_tokens_total: first_u64(
            &span.attributes,
            &[
                "gen_ai.usage.output_tokens",
                "gen_ai.usage.completion_tokens",
                "gen_ai.response.usage.output_tokens",
                "llm.usage.completion_tokens",
            ],
        ),
        reasoning_tokens: first_u64(
            &span.attributes,
            &[
                "gen_ai.usage.reasoning_tokens",
                "llm.usage.reasoning_tokens",
            ],
        ),
        visible_output_tokens: first_u64(
            &span.attributes,
            &[
                "gen_ai.usage.visible_output_tokens",
                "llm.usage.visible_output_tokens",
            ],
        ),
    };

    let occurred_at = span
        .start_nanos
        .and_then(timestamp_from_nanos)
        .unwrap_or(receive_time);
    let latency_ms = match (span.start_nanos, span.end_nanos) {
        (Some(start), Some(end)) if end >= start => i64::try_from((end - start) / 1_000_000).ok(),
        _ => None,
    };
    let http_status = first_i64(
        &span.attributes,
        &[
            "http.response.status_code",
            "http.status_code",
            "http.status",
        ],
    );
    let success = span
        .status_code
        .and_then(|code| match code {
            1 => Some(true),
            2 => Some(false),
            _ => None,
        })
        .or_else(|| http_status.map(|status| (200..400).contains(&status)))
        .or_else(|| {
            first_bool(
                &span.attributes,
                &["tokenbuddy.success", "gen_ai.success", "request.success"],
            )
        });
    let launcher = launcher_from_attributes(&span.attributes);
    let precision_token = if usage.is_empty() {
        PrecisionLevel::Unavailable
    } else {
        PrecisionLevel::Verified
    };
    let precision_session = session_external_id
        .as_ref()
        .filter(|_| !matches!(app, AppKind::Unknown))
        .map_or(PrecisionLevel::Unavailable, |_| PrecisionLevel::Correlated);
    let precision_provider = provider
        .as_ref()
        .map_or(PrecisionLevel::Unavailable, |_| PrecisionLevel::Verified);
    let session_id = session_external_id
        .as_deref()
        .map(|value| stable_session_id(app, value));
    let parent_session_id = parent_external_id
        .as_deref()
        .map(|value| stable_session_id(app, value));
    let raw_usage_json = sanitized_usage_json(&usage);
    let identity_parts = [
        SOURCE_ID.to_owned(),
        span.trace_id.clone(),
        span.span_id.clone(),
        request_id.clone().unwrap_or_default(),
        response_id.clone().unwrap_or_default(),
        occurred_at.to_rfc3339(),
        model.clone().unwrap_or_default(),
        usage_fingerprint(&usage),
    ];
    let raw_event_hash = hash_parts(identity_parts.iter().map(String::as_str));

    Some(UsageEvent {
        id: raw_event_hash.clone(),
        occurred_at,
        app,
        launcher,
        ingest_source: IngestSource::Otel,
        source_id: SOURCE_ID.to_owned(),
        provider_id,
        account_id: None,
        session_id,
        parent_session_id,
        request_id,
        response_id,
        model,
        query_source: safe_query_source(&span.attributes),
        usage,
        provider_reported_cost: None,
        estimated_cost: None,
        currency: None,
        http_status,
        latency_ms,
        success,
        precision_token,
        precision_session,
        precision_provider,
        precision_account: PrecisionLevel::Unavailable,
        raw_event_hash,
        raw_usage_json,
    })
}

fn sanitized_usage_json(usage: &NormalizedUsage) -> Option<Value> {
    if usage.is_empty() {
        return None;
    }
    Some(json!({
        "input_tokens_total": usage.input_tokens_total,
        "input_tokens_uncached": usage.input_tokens_uncached,
        "cache_read_tokens": usage.cache_read_tokens,
        "cache_write_tokens": usage.cache_write_tokens,
        "output_tokens_total": usage.output_tokens_total,
        "reasoning_tokens": usage.reasoning_tokens,
        "visible_output_tokens": usage.visible_output_tokens,
    }))
}

fn usage_fingerprint(usage: &NormalizedUsage) -> String {
    [
        usage.input_tokens_total,
        usage.input_tokens_uncached,
        usage.cache_read_tokens,
        usage.cache_write_tokens,
        usage.output_tokens_total,
        usage.reasoning_tokens,
        usage.visible_output_tokens,
    ]
    .iter()
    .map(|value| value.map_or_else(|| "-".to_owned(), |value| value.to_string()))
    .collect::<Vec<_>>()
    .join(",")
}

fn app_from_attributes(attributes: &BTreeMap<String, AttributeValue>) -> AppKind {
    let value = first_text(
        attributes,
        &[
            "tokenbuddy.app",
            "gen_ai.app.name",
            "service.name",
            "agent.name",
            "application.name",
        ],
    )
    .unwrap_or_default()
    .to_ascii_lowercase();
    if value.contains("codex") {
        AppKind::Codex
    } else if value.contains("claude") {
        AppKind::ClaudeCode
    } else {
        AppKind::Unknown
    }
}

fn launcher_from_attributes(attributes: &BTreeMap<String, AttributeValue>) -> LauncherKind {
    let value = first_text(
        attributes,
        &["tokenbuddy.launcher", "gen_ai.launcher", "launcher"],
    )
    .unwrap_or_default()
    .to_ascii_lowercase();
    if value.contains("cc-switch") || value.contains("cc_switch") {
        LauncherKind::CCSwitch
    } else if value.contains("cockpit") {
        LauncherKind::Cockpit
    } else if value.contains("proxy") {
        LauncherKind::ObserverProxy
    } else if value.is_empty() || value == "direct" {
        LauncherKind::Direct
    } else {
        LauncherKind::Unknown
    }
}

fn safe_query_source(attributes: &BTreeMap<String, AttributeValue>) -> Option<String> {
    let value = first_text(
        attributes,
        &["tokenbuddy.query_source", "gen_ai.operation.name"],
    )?;
    let lower = value.to_ascii_lowercase();
    [
        "main",
        "main_agent",
        "subagent",
        "sub_agent",
        "tool",
        "agent",
    ]
    .iter()
    .any(|allowed| lower == *allowed)
    .then_some(value)
}

fn provider_identity(value: &str) -> (String, String, String) {
    let normalized = value.trim().to_ascii_lowercase();
    let known = [
        ("openai", "openai", "OpenAI"),
        ("anthropic", "anthropic", "Anthropic"),
        ("google", "google", "Google"),
        ("gemini", "google", "Google Gemini"),
        ("deepseek", "deepseek", "DeepSeek"),
        ("mistral", "mistral", "Mistral"),
    ];
    if let Some((id, family, display)) = known
        .into_iter()
        .find(|(needle, _, _)| normalized.contains(needle))
    {
        return (id.to_owned(), family.to_owned(), display.to_owned());
    }
    let short = short_hash(&normalized);
    (
        format!("otel:{short}"),
        bounded_text(&normalized, 80),
        bounded_text(value.trim(), 120),
    )
}

fn stable_session_id(app: AppKind, external: &str) -> String {
    if external.starts_with("codex-session:") || external.starts_with("claude-code-session:") {
        return external.to_owned();
    }
    let source = match app {
        AppKind::Codex => "codex-session",
        AppKind::ClaudeCode => "claude-code-session",
        AppKind::OpenCode => "opencode",
        AppKind::DeepseekHarness => "dsh-session",
        AppKind::Unknown => SOURCE_ID,
    };
    format!("{source}:{}", short_hash(external))
}

fn first_text(attributes: &BTreeMap<String, AttributeValue>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| attributes.get(*key).and_then(AttributeValue::as_str))
        .map(|value| bounded_text(value, 256))
        .filter(|value| !value.is_empty())
}

fn first_u64(attributes: &BTreeMap<String, AttributeValue>, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| attributes.get(*key).and_then(AttributeValue::as_u64))
}

fn first_i64(attributes: &BTreeMap<String, AttributeValue>, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| attributes.get(*key).and_then(AttributeValue::as_i64))
}

fn first_bool(attributes: &BTreeMap<String, AttributeValue>, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| attributes.get(*key).and_then(AttributeValue::as_bool))
}

fn bounded_text(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn timestamp_from_nanos(nanos: u64) -> Option<DateTime<Utc>> {
    let seconds = i64::try_from(nanos / 1_000_000_000).ok()?;
    let subsec = u32::try_from(nanos % 1_000_000_000).ok()?;
    DateTime::from_timestamp(seconds, subsec)
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

// --- OTLP JSON -----------------------------------------------------------

fn parse_json_export(root: &Value) -> Result<Vec<SpanData>, ParseError> {
    let resources = array_field(root, &["resourceSpans", "resource_spans"])
        .ok_or_else(|| ParseError::Invalid("resourceSpans missing".to_owned()))?;
    let mut spans = Vec::new();
    for resource_span in resources {
        let resource_attrs = object_field(resource_span, "resource")
            .and_then(|resource| resource.get("attributes"))
            .map(parse_json_attributes)
            .unwrap_or_default();
        let scopes = array_field(resource_span, &["scopeSpans", "scope_spans"]).unwrap_or(&[]);
        for scope in scopes {
            for value in array_field(scope, &["spans"]).unwrap_or(&[]) {
                let mut span = parse_json_span(value)?;
                for (key, value) in &resource_attrs {
                    span.attributes
                        .entry(key.clone())
                        .or_insert_with(|| value.clone());
                }
                spans.push(span);
            }
        }
    }
    Ok(spans)
}

fn parse_json_span(value: &Value) -> Result<SpanData, ParseError> {
    let object = value
        .as_object()
        .ok_or_else(|| ParseError::Invalid("span is not an object".to_owned()))?;
    let attributes = object
        .get("attributes")
        .map(parse_json_attributes)
        .unwrap_or_default();
    let status_code = object
        .get("status")
        .and_then(|status| status.get("code"))
        .and_then(parse_u64_json);
    Ok(SpanData {
        trace_id: object
            .get("traceId")
            .or_else(|| object.get("trace_id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        span_id: object
            .get("spanId")
            .or_else(|| object.get("span_id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        name: object
            .get("name")
            .and_then(Value::as_str)
            .map(|value| bounded_text(value, 120)),
        start_nanos: object
            .get("startTimeUnixNano")
            .or_else(|| object.get("start_time_unix_nano"))
            .and_then(parse_u64_json),
        end_nanos: object
            .get("endTimeUnixNano")
            .or_else(|| object.get("end_time_unix_nano"))
            .and_then(parse_u64_json),
        status_code,
        attributes,
    })
}

fn parse_json_attributes(value: &Value) -> BTreeMap<String, AttributeValue> {
    let mut attributes = BTreeMap::new();
    let Some(items) = value.as_array() else {
        return attributes;
    };
    for item in items {
        let Some(object) = item.as_object() else {
            continue;
        };
        let Some(key) = object.get("key").and_then(Value::as_str) else {
            continue;
        };
        let Some(value) = object.get("value").and_then(parse_json_any_value) else {
            continue;
        };
        attributes.insert(key.to_owned(), value);
    }
    attributes
}

fn parse_json_any_value(value: &Value) -> Option<AttributeValue> {
    let object = value.as_object()?;
    object
        .get("stringValue")
        .or_else(|| object.get("string_value"))
        .and_then(Value::as_str)
        .map(|value| AttributeValue::String(bounded_text(value, 256)))
        .or_else(|| {
            object
                .get("intValue")
                .or_else(|| object.get("int_value"))
                .and_then(parse_i64_json)
                .map(AttributeValue::Integer)
        })
        .or_else(|| {
            object
                .get("doubleValue")
                .or_else(|| object.get("double_value"))
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite())
                .map(AttributeValue::Float)
        })
        .or_else(|| {
            object
                .get("boolValue")
                .or_else(|| object.get("bool_value"))
                .and_then(Value::as_bool)
                .map(AttributeValue::Bool)
        })
}

fn parse_u64_json(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.parse::<u64>().ok())
}

fn parse_i64_json(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str()?.parse::<i64>().ok())
}

fn array_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a [Value]> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_array))
        .map(Vec::as_slice)
}

fn object_field<'a>(value: &'a Value, key: &str) -> Option<&'a Map<String, Value>> {
    value.get(key).and_then(Value::as_object)
}

// --- OTLP protobuf -------------------------------------------------------

#[derive(Debug)]
enum FieldValue {
    Varint(u64),
    Fixed64(u64),
    Bytes(Vec<u8>),
}

struct ProtoReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> ProtoReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn next(&mut self) -> Result<Option<(u32, FieldValue)>, ParseError> {
        if self.offset == self.bytes.len() {
            return Ok(None);
        }
        let key = self.varint()?;
        let field = u32::try_from(key >> 3)
            .map_err(|_| ParseError::Invalid("protobuf field overflow".to_owned()))?;
        if field == 0 {
            return Err(ParseError::Invalid("protobuf field zero".to_owned()));
        }
        let value = match key & 7 {
            0 => FieldValue::Varint(self.varint()?),
            1 => FieldValue::Fixed64(self.fixed64()?),
            2 => FieldValue::Bytes(self.bytes_value()?),
            5 => FieldValue::Fixed64(u64::from(self.fixed32()?)),
            _ => {
                return Err(ParseError::Invalid(
                    "unsupported protobuf wire type".to_owned(),
                ));
            }
        };
        Ok(Some((field, value)))
    }

    fn varint(&mut self) -> Result<u64, ParseError> {
        let mut value = 0_u64;
        for shift in (0..64).step_by(7) {
            let byte = *self
                .bytes
                .get(self.offset)
                .ok_or_else(|| ParseError::Invalid("truncated protobuf varint".to_owned()))?;
            self.offset += 1;
            if shift == 63 && byte > 1 {
                return Err(ParseError::Invalid("protobuf varint overflow".to_owned()));
            }
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(ParseError::Invalid("protobuf varint overflow".to_owned()))
    }

    fn fixed32(&mut self) -> Result<u32, ParseError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes(
            bytes.try_into().expect("fixed32 length"),
        ))
    }

    fn fixed64(&mut self) -> Result<u64, ParseError> {
        let bytes = self.take(8)?;
        Ok(u64::from_le_bytes(
            bytes.try_into().expect("fixed64 length"),
        ))
    }

    fn bytes_value(&mut self) -> Result<Vec<u8>, ParseError> {
        let length = usize::try_from(self.varint()?)
            .map_err(|_| ParseError::Invalid("protobuf length overflow".to_owned()))?;
        Ok(self.take(length)?.to_vec())
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], ParseError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| ParseError::Invalid("protobuf length overflow".to_owned()))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| ParseError::Invalid("truncated protobuf message".to_owned()))?;
        self.offset = end;
        Ok(bytes)
    }
}

fn parse_protobuf_export(bytes: &[u8]) -> Result<Vec<SpanData>, ParseError> {
    let mut reader = ProtoReader::new(bytes);
    let mut spans = Vec::new();
    while let Some((field, value)) = reader.next()? {
        if field == 1
            && let FieldValue::Bytes(bytes) = value
        {
            parse_resource_spans(&bytes, &mut spans)?;
        }
    }
    Ok(spans)
}

fn parse_resource_spans(bytes: &[u8], output: &mut Vec<SpanData>) -> Result<(), ParseError> {
    let mut reader = ProtoReader::new(bytes);
    let mut resource_attributes = BTreeMap::new();
    let mut scope_messages = Vec::new();
    while let Some((field, value)) = reader.next()? {
        match (field, value) {
            (1, FieldValue::Bytes(bytes)) => resource_attributes = parse_resource(&bytes)?,
            (2, FieldValue::Bytes(bytes)) => scope_messages.push(bytes),
            _ => {}
        }
    }
    for scope in scope_messages {
        parse_scope_spans(&scope, &resource_attributes, output)?;
    }
    Ok(())
}

fn parse_resource(bytes: &[u8]) -> Result<BTreeMap<String, AttributeValue>, ParseError> {
    let mut reader = ProtoReader::new(bytes);
    let mut attributes = BTreeMap::new();
    while let Some((field, value)) = reader.next()? {
        if field == 1
            && let FieldValue::Bytes(bytes) = value
            && let Some((key, value)) = parse_proto_key_value(&bytes)?
        {
            attributes.insert(key, value);
        }
    }
    Ok(attributes)
}

fn parse_scope_spans(
    bytes: &[u8],
    resource_attributes: &BTreeMap<String, AttributeValue>,
    output: &mut Vec<SpanData>,
) -> Result<(), ParseError> {
    let mut reader = ProtoReader::new(bytes);
    while let Some((field, value)) = reader.next()? {
        if field == 2
            && let FieldValue::Bytes(bytes) = value
        {
            let mut span = parse_proto_span(&bytes)?;
            for (key, value) in resource_attributes {
                span.attributes
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
            }
            output.push(span);
        }
    }
    Ok(())
}

fn parse_proto_span(bytes: &[u8]) -> Result<SpanData, ParseError> {
    let mut reader = ProtoReader::new(bytes);
    let mut span = SpanData::default();
    while let Some((field, value)) = reader.next()? {
        match (field, value) {
            (1, FieldValue::Bytes(bytes)) => span.trace_id = hex_bytes(&bytes),
            (2, FieldValue::Bytes(bytes)) => span.span_id = hex_bytes(&bytes),
            (5, FieldValue::Bytes(bytes)) => {
                span.name = Some(bounded_text(
                    std::str::from_utf8(&bytes)
                        .map_err(|_| ParseError::Invalid("span name is not UTF-8".to_owned()))?,
                    120,
                ));
            }
            (6, FieldValue::Fixed64(value)) => span.start_nanos = Some(value),
            (7, FieldValue::Fixed64(value)) => span.end_nanos = Some(value),
            (8, FieldValue::Bytes(bytes)) => {
                if let Some((key, value)) = parse_proto_key_value(&bytes)? {
                    span.attributes.insert(key, value);
                }
            }
            (15, FieldValue::Bytes(bytes)) => span.status_code = parse_proto_status(&bytes)?,
            _ => {}
        }
    }
    Ok(span)
}

fn parse_proto_status(bytes: &[u8]) -> Result<Option<u64>, ParseError> {
    let mut reader = ProtoReader::new(bytes);
    while let Some((field, value)) = reader.next()? {
        if field == 1
            && let FieldValue::Varint(value) = value
        {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn parse_proto_key_value(bytes: &[u8]) -> Result<Option<(String, AttributeValue)>, ParseError> {
    let mut reader = ProtoReader::new(bytes);
    let mut key = None;
    let mut value = None;
    while let Some((field, field_value)) = reader.next()? {
        match (field, field_value) {
            (1, FieldValue::Bytes(bytes)) => {
                key = Some(
                    std::str::from_utf8(&bytes)
                        .map_err(|_| ParseError::Invalid("attribute key is not UTF-8".to_owned()))?
                        .to_owned(),
                );
            }
            (2, FieldValue::Bytes(bytes)) => value = parse_proto_any_value(&bytes)?,
            _ => {}
        }
    }
    Ok(key.zip(value))
}

fn parse_proto_any_value(bytes: &[u8]) -> Result<Option<AttributeValue>, ParseError> {
    let mut reader = ProtoReader::new(bytes);
    while let Some((field, value)) = reader.next()? {
        let parsed = match (field, value) {
            (1, FieldValue::Bytes(bytes)) => Some(AttributeValue::String(bounded_text(
                std::str::from_utf8(&bytes)
                    .map_err(|_| ParseError::Invalid("attribute value is not UTF-8".to_owned()))?,
                256,
            ))),
            (2, FieldValue::Varint(value)) => Some(AttributeValue::Bool(value != 0)),
            (3, FieldValue::Varint(value)) => Some(AttributeValue::Integer(value as i64)),
            (4, FieldValue::Fixed64(value)) => {
                let value = f64::from_bits(value);
                value.is_finite().then_some(AttributeValue::Float(value))
            }
            _ => None,
        };
        if parsed.is_some() {
            return Ok(parsed);
        }
    }
    Ok(None)
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{Shutdown, TcpStream},
        sync::mpsc,
        time::Duration,
    };

    use super::{
        BatchSink, DESCRIPTOR, SOURCE_ID, batch_from_spans, parse_json_export,
        parse_protobuf_export,
    };
    use serde_json::json;
    use tokenbuddy_domain::{AppKind, IngestSource, PrecisionLevel};

    const _: () = {
        assert!(DESCRIPTOR.capabilities.usage_events);
        assert!(!DESCRIPTOR.read_only);
    };

    #[test]
    fn descriptor_is_loopback_only_and_usage_source_is_otel() {
        assert_eq!(DESCRIPTOR.id, SOURCE_ID);
    }

    #[test]
    fn json_export_extracts_numeric_usage_and_drops_unknown_body_attributes() {
        let value = json!({
            "resourceSpans": [{
                "resource": {"attributes": [
                    {"key": "service.name", "value": {"stringValue": "claude-code"}},
                    {"key": "gen_ai.system", "value": {"stringValue": "anthropic"}}
                ]},
                "scopeSpans": [{"spans": [{
                    "traceId": "01",
                    "spanId": "02",
                    "startTimeUnixNano": "1721900000000000000",
                    "endTimeUnixNano": "1721900000123000000",
                    "attributes": [
                        {"key": "gen_ai.request.id", "value": {"stringValue": "req-1"}},
                        {"key": "gen_ai.response.id", "value": {"stringValue": "resp-1"}},
                        {"key": "gen_ai.request.model", "value": {"stringValue": "claude-test"}},
                        {"key": "gen_ai.usage.input_tokens", "value": {"intValue": "100"}},
                        {"key": "gen_ai.usage.output_tokens", "value": {"intValue": "25"}},
                        {"key": "gen_ai.prompt", "value": {"stringValue": "do not persist me"}}
                    ]
                }]}]
            }]
        });
        let spans = parse_json_export(&value).expect("parse JSON");
        let batch = batch_from_spans(spans, "http://127.0.0.1:4318/v1/traces");
        assert_eq!(batch.usage_events.len(), 1);
        let event = &batch.usage_events[0];
        assert_eq!(event.app, AppKind::ClaudeCode);
        assert_eq!(event.ingest_source, IngestSource::Otel);
        assert_eq!(event.precision_token, PrecisionLevel::Verified);
        assert_eq!(event.usage.input_tokens_total, Some(100));
        assert_eq!(event.usage.output_tokens_total, Some(25));
        let raw = event.raw_usage_json.as_ref().expect("sanitized usage");
        assert!(raw.get("gen_ai.prompt").is_none());
        assert!(raw.get("input_tokens_total").is_some());
    }

    #[test]
    fn protobuf_export_reads_resource_attributes_and_span_usage() {
        let payload = export_message(&resource_spans_message());
        let spans = parse_protobuf_export(&payload).expect("parse protobuf");
        assert_eq!(spans.len(), 1);
        let batch = batch_from_spans(spans, "http://127.0.0.1:4318/v1/traces");
        let event = &batch.usage_events[0];
        assert_eq!(event.app, AppKind::Codex);
        assert_eq!(event.request_id.as_deref(), Some("request-proto"));
        assert_eq!(event.usage.input_tokens_total, Some(321));
        assert_eq!(event.usage.output_tokens_total, Some(12));
    }

    #[test]
    fn receiver_accepts_loopback_json_and_delivers_a_batch() {
        let (sender, receiver) = mpsc::channel();
        let sink: BatchSink = std::sync::Arc::new(move |batch| {
            sender.send(batch).expect("batch receiver");
        });
        let server = super::OtelReceiver::start(0, sink).expect("receiver starts");
        let address = format!("127.0.0.1:{}", server.port());
        let payload = br#"{"resourceSpans":[{"scopeSpans":[{"spans":[{"spanId":"01","attributes":[{"key":"service.name","value":{"stringValue":"codex"}},{"key":"gen_ai.usage.input_tokens","value":{"intValue":"7"}}]}]}]}]}"#;
        let mut stream = TcpStream::connect(address).expect("connect loopback");
        write!(
            stream,
            "POST /v1/traces HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        )
        .expect("write headers");
        stream.write_all(payload).expect("write body");
        stream
            .shutdown(Shutdown::Write)
            .expect("close request body");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set response timeout");
        let mut response = Vec::new();
        stream.read_to_end(&mut response).expect("read response");
        assert!(response.starts_with(b"HTTP/1.1 200"), "unexpected response");
        let batch = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("receiver did not deliver batch");
        assert_eq!(batch.usage_events.len(), 1);
        assert_eq!(batch.usage_events[0].usage.input_tokens_total, Some(7));
        drop(server);
    }

    fn resource_spans_message() -> Vec<u8> {
        let resource = message(&[(1, bytes(&repeated_key_value("service.name", "codex")))]);
        let span = message(&[
            (2, bytes(&[1, 2])),
            (5, bytes(b"generation")),
            (6, fixed64(1_721_900_000_000_000_000)),
            (7, fixed64(1_721_900_000_012_000_000)),
            (
                8,
                bytes(&repeated_key_value("gen_ai.request.id", "request-proto")),
            ),
            (
                8,
                bytes(&repeated_key_value("gen_ai.usage.input_tokens", "321")),
            ),
            (
                8,
                bytes(&repeated_key_value("gen_ai.usage.output_tokens", "12")),
            ),
        ]);
        let scope = message(&[(2, bytes(&span))]);
        message(&[(1, bytes(&resource)), (2, bytes(&scope))])
    }

    fn repeated_key_value(key: &str, value: &str) -> Vec<u8> {
        let any = if key.contains("tokens") {
            message(&[(3, varint(value.parse::<u64>().expect("integer")))])
        } else {
            message(&[(1, bytes(value.as_bytes()))])
        };
        message(&[(1, bytes(key.as_bytes())), (2, bytes(&any))])
    }

    fn export_message(resource_spans: &[u8]) -> Vec<u8> {
        message(&[(1, bytes(resource_spans))])
    }

    enum Encoded {
        Varint(u64),
        Fixed64(u64),
        Bytes(Vec<u8>),
    }

    fn message(fields: &[(u32, Encoded)]) -> Vec<u8> {
        let mut output = Vec::new();
        for (field, value) in fields {
            let wire_type = match value {
                Encoded::Varint(_) => 0,
                Encoded::Fixed64(_) => 1,
                Encoded::Bytes(_) => 2,
            };
            varint_into(&mut output, u64::from(*field) << 3 | wire_type);
            match value {
                Encoded::Varint(value) => varint_into(&mut output, *value),
                Encoded::Fixed64(value) => output.extend_from_slice(&value.to_le_bytes()),
                Encoded::Bytes(value) => {
                    varint_into(&mut output, value.len() as u64);
                    output.extend_from_slice(value);
                }
            }
        }
        output
    }

    fn varint(value: u64) -> Encoded {
        Encoded::Varint(value)
    }

    fn fixed64(value: u64) -> Encoded {
        Encoded::Fixed64(value)
    }

    fn bytes(value: &[u8]) -> Encoded {
        Encoded::Bytes(value.to_vec())
    }

    fn varint_into(output: &mut Vec<u8>, mut value: u64) {
        while value >= 0x80 {
            output.push((value as u8 & 0x7f) | 0x80);
            value >>= 7;
        }
        output.push(value as u8);
    }
}
