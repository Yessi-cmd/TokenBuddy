//! Read-only official ChatGPT/Codex quota snapshots.
//!
//! The official Codex client reads the authenticated `GET /wham/usage` endpoint
//! (or `/api/codex/usage` for the Codex API route). This adapter follows that
//! same request shape, but keeps the access token in memory for one request and
//! never puts it into a domain record, a cursor, a source error, or SQLite.
#![warn(missing_docs)]

use std::{path::PathBuf, time::Duration};

use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokenbuddy_codex_session::account::{
    AUTH_FILENAME, OfficialAuthMaterial, PROVIDER_DISPLAY_NAME, PROVIDER_FAMILY, PROVIDER_ID,
    read_official_account,
};
use tokenbuddy_domain::{
    AdapterCapabilities, AdapterDescriptor, AdapterError, DetectionResult, ImportBatch,
    ImportCursor, LauncherKind, PrecisionLevel, ProviderRecord, QuotaSnapshot, SourceHealth,
    SourceRecord, UsageAdapter, WatcherHandle,
};

/// Stable source id for the direct official quota endpoint.
pub const SOURCE_ID: &str = "openai-official-quota";
/// Adapter type persisted in the source table.
pub const ADAPTER_TYPE: &str = "official_quota_api";
/// Human-readable source name.
pub const DISPLAY_NAME: &str = "OpenAI Official Quota API";
/// Default ChatGPT backend host used by the official Codex client.
pub const DEFAULT_BASE_URL: &str = "https://chatgpt.com/backend-api";
/// Cursor resource used to deduplicate unchanged remote responses.
pub const CURSOR_RESOURCE_ID: &str = "official-rate-limits";

/// Static capabilities advertised to the Core catalog.
pub const DESCRIPTOR: AdapterDescriptor = AdapterDescriptor {
    id: SOURCE_ID,
    adapter_type: ADAPTER_TYPE,
    display_name: DISPLAY_NAME,
    capabilities: AdapterCapabilities {
        usage_events: false,
        provider_context: true,
        quota_snapshots: true,
        file_watch: false,
    },
    read_only: true,
};

/// Errors from the official quota endpoint.
#[derive(Debug, Error)]
pub enum OfficialQuotaError {
    /// The configured home has no ChatGPT OAuth material.
    #[error("ChatGPT OAuth 登录态不可用")]
    MissingAuth,
    /// The endpoint could not be reached.
    #[error("官方额度请求失败：{0}")]
    Request(#[from] reqwest::Error),
    /// The endpoint rejected the current access token.
    #[error("官方额度请求被拒绝（HTTP {0}），请在 Codex/ChatGPT 中重新登录")]
    HttpStatus(u16),
    /// The endpoint returned a successful response that is not a known quota
    /// shape. The body is intentionally not included in the error message.
    #[error("官方额度响应缺少可识别的额度窗口")]
    UnsupportedResponse,
    /// The auth file or fixture could not be decoded.
    #[error("官方额度响应 JSON 无法解析：{0}")]
    Json(#[from] serde_json::Error),
    /// The local HTTP client could not be constructed.
    #[error("官方额度 HTTP 客户端无法初始化：{0}")]
    Client(#[source] reqwest::Error),
}

/// One direct official quota reader.
pub struct OfficialQuotaAdapter {
    codex_home: PathBuf,
    fingerprint_salt: String,
    base_url: String,
    client: Client,
}

impl std::fmt::Debug for OfficialQuotaAdapter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OfficialQuotaAdapter")
            .field("codex_home", &self.codex_home)
            .field("fingerprint_salt", &"<redacted>")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl OfficialQuotaAdapter {
    /// Build a reader using the first-party ChatGPT backend route.
    pub fn new(
        codex_home: impl Into<PathBuf>,
        fingerprint_salt: impl Into<String>,
    ) -> Result<Self, OfficialQuotaError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(OfficialQuotaError::Client)?;
        Ok(Self {
            codex_home: codex_home.into(),
            fingerprint_salt: fingerprint_salt.into(),
            base_url: DEFAULT_BASE_URL.to_owned(),
            client,
        })
    }

    /// Override the backend base URL. This is primarily for a controlled test
    /// server; production uses [`DEFAULT_BASE_URL`].
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_owned();
        self
    }

    /// The exact endpoint that the next refresh will contact.
    pub fn endpoint_url(&self) -> String {
        self.endpoint_url_inner()
    }

    /// Read the current account and quota snapshot synchronously.
    pub fn import_history_sync(
        &self,
        cursor: Option<&ImportCursor>,
    ) -> Result<ImportBatch, OfficialQuotaError> {
        let now = Utc::now();
        let Some(auth) =
            tokenbuddy_codex_session::account::read_official_auth_material(&self.codex_home)
        else {
            return Ok(self.not_found_batch(now));
        };
        let Some(mut account) = read_official_account(&self.codex_home, &self.fingerprint_salt)
        else {
            return Ok(self.not_found_batch(now));
        };

        let payload = self.fetch_payload(&auth)?;
        let response_hash = response_hash(&account.id, &payload);
        let snapshots = parse_quota_snapshots(&payload, &account.id, now);
        if snapshots.is_empty() {
            return Err(OfficialQuotaError::UnsupportedResponse);
        }

        if let Some(plan) = plan_type(&payload) {
            account.plan = Some(plan);
        }

        let source = source_record(now, &self.base_url, "healthy", None);
        let unchanged =
            cursor.and_then(|value| value.content_hash.as_deref()) == Some(response_hash.as_str());
        let cursor = ImportCursor {
            source_id: SOURCE_ID.to_owned(),
            resource_id: CURSOR_RESOURCE_ID.to_owned(),
            file_size: None,
            modified_at: Some(now),
            byte_offset: 0,
            content_hash: Some(response_hash.clone()),
            last_cumulative_usage: None,
            snapshot_generation: 0,
            last_session_id: None,
            last_model: None,
            updated_at: now,
        };

        // The response hash includes account identity and all upstream JSON,
        // so a repeated poll of the same official state is a no-op while a
        // login switch or quota change creates a new time-series point.
        let quota_snapshots = if unchanged { Vec::new() } else { snapshots };

        Ok(ImportBatch {
            source: Some(source),
            providers: vec![official_provider()],
            accounts: vec![account],
            quota_snapshots,
            cursors: vec![cursor],
            ..ImportBatch::default()
        })
    }

    /// Probe only local authentication presence. The endpoint is not contacted
    /// by detection, so opening the Sources page never causes a network call.
    pub fn detect_sync(&self) -> DetectionResult {
        let auth_path = self.codex_home.join(AUTH_FILENAME);
        let detected =
            tokenbuddy_codex_session::account::read_official_auth_material(&self.codex_home)
                .is_some();
        DetectionResult {
            source_id: SOURCE_ID.to_owned(),
            detected,
            path_or_endpoint: Some(self.endpoint_url()),
            detected_version: detected.then(|| "chatgpt-oauth".to_owned()),
            message: Some(if detected {
                "已发现 ChatGPT OAuth 登录态；官方额度将在 Core 刷新时读取".to_owned()
            } else if auth_path.is_file() {
                "发现 auth.json，但其中没有可用 ChatGPT OAuth access token".to_owned()
            } else {
                "未发现可用 ChatGPT OAuth 登录态".to_owned()
            }),
        }
    }

    fn fetch_payload(&self, auth: &OfficialAuthMaterial) -> Result<Value, OfficialQuotaError> {
        let response = self
            .client
            .get(self.endpoint_url())
            .bearer_auth(&auth.access_token)
            .header("ChatGPT-Account-ID", &auth.account_id)
            .header(
                reqwest::header::USER_AGENT,
                concat!("TokenBuddy/", env!("CARGO_PKG_VERSION")),
            )
            .header(reqwest::header::CACHE_CONTROL, "no-store")
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(OfficialQuotaError::HttpStatus(status.as_u16()));
        }
        Ok(response.json()?)
    }

    fn endpoint_url_inner(&self) -> String {
        if self.base_url.contains("/backend-api") {
            format!("{}/wham/usage", self.base_url)
        } else {
            format!("{}/api/codex/usage", self.base_url)
        }
    }

    fn not_found_batch(&self, now: DateTime<Utc>) -> ImportBatch {
        ImportBatch {
            source: Some(source_record(now, &self.base_url, "not_found", None)),
            ..ImportBatch::default()
        }
    }
}

impl UsageAdapter for OfficialQuotaAdapter {
    fn id(&self) -> &'static str {
        SOURCE_ID
    }

    fn display_name(&self) -> &'static str {
        DISPLAY_NAME
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
        self.import_history_sync(cursor.as_ref())
            .map_err(|error| AdapterError {
                message: error.to_string(),
            })
    }

    async fn health(&self) -> Result<SourceHealth, AdapterError> {
        let detection = self.detect_sync();
        Ok(SourceHealth {
            source_id: SOURCE_ID.to_owned(),
            status: if detection.detected {
                "healthy".to_owned()
            } else {
                "not_found".to_owned()
            },
            last_success_at: None,
            last_error: None,
        })
    }

    async fn start_watch(
        &self,
        _sink: tokenbuddy_domain::EventSink,
    ) -> Result<WatcherHandle, AdapterError> {
        Err(AdapterError {
            message: "官方额度由 Core 定时刷新，不提供文件 watcher".to_owned(),
        })
    }
}

fn official_provider() -> ProviderRecord {
    ProviderRecord {
        id: PROVIDER_ID.to_owned(),
        provider_family: PROVIDER_FAMILY.to_owned(),
        display_name: PROVIDER_DISPLAY_NAME.to_owned(),
        upstream_url: Some(DEFAULT_BASE_URL.to_owned()),
        launcher: Some(LauncherKind::Direct),
        source_id: Some(SOURCE_ID.to_owned()),
    }
}

fn source_record(
    now: DateTime<Utc>,
    base_url: &str,
    status: &str,
    last_error: Option<String>,
) -> SourceRecord {
    SourceRecord {
        id: SOURCE_ID.to_owned(),
        adapter_type: ADAPTER_TYPE.to_owned(),
        display_name: DISPLAY_NAME.to_owned(),
        path_or_endpoint: Some(base_url.to_owned()),
        enabled: true,
        detected_version: Some("official-quota".to_owned()),
        health_status: Some(status.to_owned()),
        last_success_at: (status == "healthy").then_some(now),
        last_error,
        created_at: now,
        updated_at: now,
    }
}

fn response_hash(account_id: &str, payload: &Value) -> String {
    hash_strings([account_id, &payload.to_string()])
}

fn hash_strings<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    let mut hasher = Sha256::new();
    for value in values {
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

/// Parse the official endpoint response into independent window snapshots.
/// Missing fields remain `None`; no token count is inferred from percentages.
pub fn parse_quota_snapshots(
    payload: &Value,
    account_id: &str,
    captured_at: DateTime<Utc>,
) -> Vec<QuotaSnapshot> {
    let mut snapshots = Vec::new();
    let quota = get_any(payload, &["rate_limit", "rateLimit"])
        .or_else(|| get_any(payload, &["rate_limits", "rateLimits"]))
        .unwrap_or(payload);

    for (name, prefix) in [
        ("primary_window", "primary"),
        ("secondary_window", "secondary"),
    ] {
        if let Some(window) = get_any(quota, &[name, camel_window_name(name)]) {
            push_window(
                &mut snapshots,
                account_id,
                prefix,
                window,
                captured_at,
                payload,
            );
        }
    }

    if let Some(additional) = get_any(payload, &["additional_rate_limits", "additionalRateLimits"])
        .and_then(Value::as_array)
    {
        for item in additional {
            let feature = get_any(item, &["metered_feature", "meteredFeature"])
                .and_then(Value::as_str)
                .unwrap_or("additional");
            if let Some(rate_limit) = get_any(item, &["rate_limit", "rateLimit"]) {
                for (name, prefix) in [
                    ("primary_window", "primary"),
                    ("secondary_window", "secondary"),
                ] {
                    if let Some(window) = get_any(rate_limit, &[name, camel_window_name(name)]) {
                        push_window(
                            &mut snapshots,
                            account_id,
                            &format!("{feature}_{prefix}"),
                            window,
                            captured_at,
                            payload,
                        );
                    }
                }
            }
        }
    }

    if let Some(limit) = get_any(payload, &["spend_control", "spendControl"])
        .and_then(|value| get_any(value, &["individual_limit", "individualLimit"]))
    {
        let remaining_percent = number(get_any(limit, &["remaining_percent", "remainingPercent"]));
        let used_percent = remaining_percent
            .filter(|value| (0.0..=100.0).contains(value))
            .map(|value| 100.0 - value);
        let reset_at = timestamp(get_any(limit, &["reset_at", "resetsAt"])).or_else(|| {
            reset_after(
                get_any(limit, &["reset_after_seconds", "resetAfterSeconds"]),
                captured_at,
            )
        });
        if used_percent.is_some() || remaining_percent.is_some() || reset_at.is_some() {
            snapshots.push(make_snapshot(
                account_id,
                "individual_limit",
                SnapshotValues {
                    used_percent,
                    remaining_percent,
                    reset_at,
                    credits_remaining: None,
                },
                captured_at,
                payload,
            ));
        }
    }

    if let Some(credits) = get_any(payload, &["credits"]).and_then(Value::as_object) {
        let balance = number(credits.get("balance"));
        let has_credits = bool_value(
            credits
                .get("has_credits")
                .or_else(|| credits.get("hasCredits")),
        );
        let unlimited = bool_value(credits.get("unlimited"));
        if balance.is_some() || has_credits == Some(true) || unlimited == Some(true) {
            snapshots.push(make_snapshot(
                account_id,
                "credits",
                SnapshotValues {
                    used_percent: None,
                    remaining_percent: None,
                    reset_at: None,
                    credits_remaining: balance,
                },
                captured_at,
                payload,
            ));
        }
    }

    snapshots
}

fn push_window(
    output: &mut Vec<QuotaSnapshot>,
    account_id: &str,
    prefix: &str,
    window: &Value,
    captured_at: DateTime<Utc>,
    payload: &Value,
) {
    let used_percent = number(get_any(window, &["used_percent", "usedPercent"]));
    let remaining_percent = number(get_any(window, &["remaining_percent", "remainingPercent"]))
        .or_else(|| {
            used_percent
                .filter(|value| (0.0..=100.0).contains(value))
                .map(|value| 100.0 - value)
        });
    let reset_at = timestamp(get_any(window, &["reset_at", "resetsAt"])).or_else(|| {
        reset_after(
            get_any(
                window,
                &[
                    "reset_after_seconds",
                    "resetAfterSeconds",
                    "resets_in_seconds",
                ],
            ),
            captured_at,
        )
    });
    let duration = integer(get_any(
        window,
        &["limit_window_seconds", "limitWindowSeconds"],
    ))
    .or_else(|| {
        integer(get_any(window, &["window_minutes", "windowDurationMins"]))
            .map(|minutes| minutes * 60)
    });
    if used_percent.is_none() && remaining_percent.is_none() && reset_at.is_none() {
        return;
    }
    let window_type = duration.filter(|seconds| *seconds > 0).map_or_else(
        || prefix.to_owned(),
        |seconds| format_window_type(prefix, seconds),
    );
    output.push(make_snapshot(
        account_id,
        &window_type,
        SnapshotValues {
            used_percent,
            remaining_percent,
            reset_at,
            credits_remaining: None,
        },
        captured_at,
        payload,
    ));
}

struct SnapshotValues {
    used_percent: Option<f64>,
    remaining_percent: Option<f64>,
    reset_at: Option<DateTime<Utc>>,
    credits_remaining: Option<f64>,
}

fn make_snapshot(
    account_id: &str,
    window_type: &str,
    values: SnapshotValues,
    captured_at: DateTime<Utc>,
    payload: &Value,
) -> QuotaSnapshot {
    let signature = format!(
        "{window_type}|{used_percent:?}|{remaining_percent:?}|{reset_at:?}|{credits_remaining:?}",
        used_percent = values.used_percent,
        remaining_percent = values.remaining_percent,
        reset_at = values.reset_at,
        credits_remaining = values.credits_remaining,
    );
    let payload_signature = payload.to_string();
    QuotaSnapshot {
        // Include the complete upstream state so a change in another window
        // can still create a coherent point for this window. The durable
        // response cursor suppresses repeated polls of the same state.
        id: hash_strings([SOURCE_ID, account_id, &signature, &payload_signature]),
        account_id: account_id.to_owned(),
        account_name: None,
        provider_name: None,
        captured_at,
        window_type: window_type.to_owned(),
        used_percent: values.used_percent,
        remaining_percent: values.remaining_percent,
        reset_at: values.reset_at,
        credits_remaining: values.credits_remaining,
        precision: PrecisionLevel::Verified,
        raw_json: Some(payload.clone()),
    }
}

fn plan_type(payload: &Value) -> Option<String> {
    get_any(payload, &["plan_type", "planType"])
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn get_any<'a>(value: &'a Value, names: &[&str]) -> Option<&'a Value> {
    let object = value.as_object()?;
    names.iter().find_map(|name| object.get(*name))
}

fn camel_window_name(name: &str) -> &str {
    match name {
        "primary_window" => "primaryWindow",
        "secondary_window" => "secondaryWindow",
        _ => name,
    }
}

fn number(value: Option<&Value>) -> Option<f64> {
    value.and_then(|value| match value {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    })
}

fn integer(value: Option<&Value>) -> Option<i64> {
    value.and_then(|value| match value {
        Value::Number(value) => value
            .as_i64()
            .or_else(|| value.as_f64().map(|value| value as i64)),
        Value::String(value) => value.parse().ok(),
        _ => None,
    })
}

fn bool_value(value: Option<&Value>) -> Option<bool> {
    value.and_then(Value::as_bool)
}

fn timestamp(value: Option<&Value>) -> Option<DateTime<Utc>> {
    let value = value?;
    match value {
        Value::Number(_) => {
            let seconds = integer(Some(value))?;
            (seconds > 0)
                .then(|| DateTime::from_timestamp(seconds, 0))
                .flatten()
        }
        Value::String(value) => value
            .parse::<i64>()
            .ok()
            .and_then(|seconds| (seconds > 0).then(|| DateTime::from_timestamp(seconds, 0)))
            .flatten()
            .or_else(|| {
                DateTime::parse_from_rfc3339(value)
                    .ok()
                    .map(|value| value.with_timezone(&Utc))
            }),
        _ => None,
    }
}

fn reset_after(value: Option<&Value>, captured_at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let seconds = integer(value)?.max(0);
    captured_at.checked_add_signed(chrono::Duration::seconds(seconds))
}

fn format_window_type(prefix: &str, seconds: i64) -> String {
    let minutes = (seconds + 59) / 60;
    if minutes >= 1_440 && minutes % 1_440 == 0 {
        format!("{prefix}_{}d", minutes / 1_440)
    } else if minutes >= 60 && minutes % 60 == 0 {
        format!("{prefix}_{}h", minutes / 60)
    } else {
        format!("{prefix}_{minutes}m")
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        path::Path,
        thread,
    };

    use chrono::Utc;
    use serde_json::Value;
    use tempfile::TempDir;
    use tokenbuddy_domain::ImportCursor;

    use super::{CURSOR_RESOURCE_ID, OfficialQuotaAdapter, SOURCE_ID, parse_quota_snapshots};

    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/codex")
            .join(name);
        fs::read_to_string(path).expect("fixture")
    }

    fn serve_once(body: String) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("server binds");
        let address = listener.local_addr().expect("server address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let size = stream.read(&mut buffer).expect("read request");
                if size == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..size]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request).expect("request UTF-8");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("response");
            request
        });
        (format!("http://{address}"), handle)
    }

    #[test]
    fn parses_official_windows_credits_and_preserves_missing_values() {
        let payload: Value =
            serde_json::from_str(&fixture("official_quota_response.json")).expect("payload");
        let snapshots = parse_quota_snapshots(&payload, "account-1", Utc::now());
        assert!(
            snapshots
                .iter()
                .any(|snapshot| snapshot.window_type == "primary_5h")
        );
        assert!(
            snapshots
                .iter()
                .any(|snapshot| snapshot.window_type == "secondary_7d")
        );
        assert!(
            snapshots
                .iter()
                .any(|snapshot| snapshot.window_type == "credits")
        );
        assert!(
            snapshots
                .iter()
                .all(|snapshot| snapshot.precision == tokenbuddy_domain::PrecisionLevel::Verified)
        );
        let secondary = snapshots
            .iter()
            .find(|snapshot| snapshot.window_type == "secondary_7d")
            .expect("secondary window");
        assert_eq!(secondary.used_percent, None);
        assert_eq!(secondary.remaining_percent, None);
    }

    #[test]
    fn imports_with_official_headers_and_deduplicates_by_response_cursor() {
        let home = TempDir::new().expect("home");
        let auth = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/codex/auth/chatgpt_auth.json");
        fs::copy(auth, home.path().join("auth.json")).expect("auth fixture");
        let body = fixture("official_quota_response.json");
        let (base_url, handle) = serve_once(body.clone());
        let adapter = OfficialQuotaAdapter::new(home.path(), "fixture-salt")
            .expect("adapter")
            .with_base_url(format!("{base_url}/backend-api"));
        let first = adapter.import_history_sync(None).expect("first import");
        assert_eq!(
            first
                .source
                .as_ref()
                .and_then(|source| source.health_status.as_deref()),
            Some("healthy")
        );
        assert_eq!(first.cursors[0].resource_id, CURSOR_RESOURCE_ID);
        assert!(!first.quota_snapshots.is_empty());
        let request = handle.join().expect("server");
        assert!(request.contains("GET /backend-api/wham/usage HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer sanitized-access-token")
        );
        assert!(
            request
                .to_ascii_lowercase()
                .contains("chatgpt-account-id: acct-fixture-0001")
        );

        // The adapter's cursor is the durable deduplication boundary. The
        // second call is tested with the same response through a fresh server.
        let (base_url, handle) = serve_once(body);
        let adapter = adapter.with_base_url(format!("{base_url}/backend-api"));
        let cursor: ImportCursor = first.cursors[0].clone();
        let second = adapter
            .import_history_sync(Some(&cursor))
            .expect("second import");
        assert!(second.quota_snapshots.is_empty());
        handle.join().expect("server");
        assert_eq!(second.cursors[0].source_id, SOURCE_ID);
    }
}
