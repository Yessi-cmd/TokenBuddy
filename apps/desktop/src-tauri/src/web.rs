use std::{
    fs,
    io::{self, Read, Write},
    net::{Ipv4Addr, Ipv6Addr, TcpListener, TcpStream},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokenbuddy_cc_switch::CcSwitchAdapter;
use tokenbuddy_claude_session::ClaudeSessionAdapter;
use tokenbuddy_cockpit::CockpitAdapter;
use tokenbuddy_codex_session::CodexSessionAdapter;
use tokenbuddy_core::Core;
use tokenbuddy_domain::{AppKind, PrecisionLevel, UsageFilters};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct LocalWebApiStatus {
    pub running: bool,
    pub url: Option<String>,
    pub loopback_urls: Vec<String>,
}

pub struct LocalWebServer {
    url: String,
    loopback_urls: Vec<String>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

pub type AutostartCallback = Arc<dyn Fn(bool) -> Result<(), String> + Send + Sync>;

impl LocalWebServer {
    #[cfg(test)]
    pub fn start(core: Arc<Core>, static_root: PathBuf) -> io::Result<Self> {
        Self::start_with_autostart(core, static_root, None)
    }

    pub fn start_with_autostart(
        core: Arc<Core>,
        static_root: PathBuf,
        autostart: Option<AutostartCallback>,
    ) -> io::Result<Self> {
        // Bind both loopback families explicitly. This keeps the optional
        // dashboard inaccessible from the LAN while supporting clients that
        // resolve localhost to either address family.
        let ipv4_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = ipv4_listener.local_addr()?.port();
        let ipv6_listener = TcpListener::bind((Ipv6Addr::LOCALHOST, port))?;
        ipv4_listener.set_nonblocking(true)?;
        ipv6_listener.set_nonblocking(true)?;
        let url = format!("http://127.0.0.1:{port}");
        let loopback_urls = vec![url.clone(), format!("http://[::1]:{port}")];
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("tokenbuddy-web-api".to_owned())
            .spawn(move || {
                serve(
                    vec![ipv4_listener, ipv6_listener],
                    core,
                    static_root,
                    thread_stop,
                    autostart,
                )
            })?;
        Ok(Self {
            url,
            loopback_urls,
            stop,
            worker: Some(worker),
        })
    }

    pub fn status(&self) -> LocalWebApiStatus {
        LocalWebApiStatus {
            running: true,
            url: Some(self.url.clone()),
            loopback_urls: self.loopback_urls.clone(),
        }
    }
}

impl Drop for LocalWebServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Debug)]
struct Request {
    method: String,
    target: String,
    body: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct RescanRequest {
    codex_home: Option<String>,
    claude_home: Option<String>,
    cc_switch_db: Option<String>,
    cockpit_db: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExportRequest {
    format: String,
    filters: Option<UsageFilters>,
}

fn serve(
    listeners: Vec<TcpListener>,
    core: Arc<Core>,
    static_root: PathBuf,
    stop: Arc<AtomicBool>,
    autostart: Option<AutostartCallback>,
) {
    while !stop.load(Ordering::SeqCst) {
        let mut accepted = false;
        for listener in &listeners {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    accepted = true;
                    let _ = handle_connection(&mut stream, &core, &static_root, autostart.as_ref());
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(_) => return,
            }
        }
        if !accepted {
            thread::sleep(Duration::from_millis(5));
        }
    }
}

fn handle_connection(
    stream: &mut TcpStream,
    core: &Core,
    static_root: &Path,
    autostart: Option<&AutostartCallback>,
) -> io::Result<()> {
    let Some(request) = read_request(stream)? else {
        return Ok(());
    };
    let response = route_request_with_autostart(&request, core, static_root, autostart);
    stream.write_all(&response)?;
    stream.flush()
}

fn read_request(stream: &mut TcpStream) -> io::Result<Option<Request>> {
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end;
    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            return Ok(None);
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > MAX_REQUEST_BYTES {
            return Ok(None);
        }
        if let Some(index) = find_bytes(&bytes, b"\r\n\r\n") {
            header_end = index + 4;
            break;
        }
    }

    let (method, target, content_length) = {
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let mut lines = headers.split("\r\n");
        let Some(request_line) = lines.next() else {
            return Ok(None);
        };
        let mut request_parts = request_line.split_whitespace();
        let (Some(method), Some(target)) = (request_parts.next(), request_parts.next()) else {
            return Ok(None);
        };
        let content_length = lines
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        (method.to_owned(), target.to_owned(), content_length)
    };
    if content_length > MAX_REQUEST_BYTES {
        return Ok(None);
    }

    while bytes.len() < header_end + content_length {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    let body_end = (header_end + content_length).min(bytes.len());
    Ok(Some(Request {
        method,
        target,
        body: bytes[header_end..body_end].to_vec(),
    }))
}

#[cfg(test)]
fn route_request(request: &Request, core: &Core, static_root: &Path) -> Vec<u8> {
    route_request_with_autostart(request, core, static_root, None)
}

fn route_request_with_autostart(
    request: &Request,
    core: &Core,
    static_root: &Path,
    autostart: Option<&AutostartCallback>,
) -> Vec<u8> {
    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((&request.target, ""));
    match (request.method.as_str(), path) {
        ("GET", "/api/health") => json_response(200, &serde_json::json!({"ok": true})),
        ("GET", "/api/quick-summary") => core
            .quick_summary()
            .map_or_else(api_error, |summary| json_response(200, &summary)),
        ("GET", "/api/dashboard-summary") => core
            .dashboard_summary_filtered(usage_filters_from_query(query))
            .map_or_else(api_error, |summary| json_response(200, &summary)),
        ("GET", "/api/model-breakdown") => core
            .model_breakdown(usage_filters_from_query(query))
            .map_or_else(api_error, |breakdown| json_response(200, &breakdown)),
        ("GET", "/api/sources") => core
            .list_sources()
            .map_or_else(api_error, |sources| json_response(200, &sources)),
        ("GET", "/api/providers") => core
            .list_providers()
            .map_or_else(api_error, |providers| json_response(200, &providers)),
        ("GET", "/api/accounts") => core
            .list_accounts()
            .map_or_else(api_error, |accounts| json_response(200, &accounts)),
        ("GET", "/api/quotas") => {
            let account_id = query_value(query, "account_id");
            let limit = query_value(query, "limit")
                .and_then(|value| value.parse().ok())
                .unwrap_or(100);
            core.list_quota_snapshots(account_id.as_deref(), limit)
                .map_or_else(api_error, |quotas| json_response(200, &quotas))
        }
        ("GET", "/api/settings") => core
            .get_app_settings()
            .map_or_else(api_error, |settings| json_response(200, &settings)),
        ("GET", "/api/detect-codex") => {
            let detection = query_value(query, "codex_home")
                .filter(|value| !value.trim().is_empty())
                .map(|path| CodexSessionAdapter::new(path).detect_sync());
            match detection {
                Some(Ok(result)) => json_response(200, &result),
                Some(Err(error)) => api_error(error.to_string()),
                None => core
                    .detect_codex_path()
                    .map_or_else(api_error, |result| json_response(200, &result)),
            }
        }
        ("GET", "/api/detect-claude") => {
            let detection = query_value(query, "claude_home")
                .filter(|value| !value.trim().is_empty())
                .map(|path| ClaudeSessionAdapter::new(path).detect_sync());
            match detection {
                Some(Ok(result)) => json_response(200, &result),
                Some(Err(error)) => api_error(error.to_string()),
                None => core
                    .detect_claude_path()
                    .map_or_else(api_error, |result| json_response(200, &result)),
            }
        }
        ("GET", "/api/sessions") => {
            let limit = query_value(query, "limit")
                .and_then(|value| value.parse().ok())
                .unwrap_or(50);
            let offset = query_value(query, "offset")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            core.list_sessions(&usage_filters_from_query(query), limit, offset)
                .map_or_else(api_error, |sessions| json_response(200, &sessions))
        }
        ("GET", "/api/usage-events") => {
            let session_id = query_value(query, "session_id");
            let limit = query_value(query, "limit")
                .and_then(|value| value.parse().ok())
                .unwrap_or(100);
            let offset = query_value(query, "offset")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            core.list_usage_events_filtered(
                session_id.as_deref(),
                limit,
                offset,
                &usage_filters_from_query(query),
            )
            .map_or_else(api_error, |events| json_response(200, &events))
        }
        ("POST", "/api/export") => match serde_json::from_slice::<ExportRequest>(&request.body) {
            Ok(body) => core
                .export_usage(&body.format, &body.filters.unwrap_or_default())
                .map_or_else(api_error, |export| json_response(200, &export)),
            Err(error) => api_error(format!("invalid export request: {error}")),
        },
        ("POST", "/api/rescan-codex") => {
            let body = serde_json::from_slice::<RescanRequest>(&request.body);
            match body {
                Ok(body) => core
                    .rescan_codex(normalize_path(body.codex_home))
                    .map_or_else(api_error, |report| json_response(200, &report)),
                Err(error) => api_error(format!("invalid rescan request: {error}")),
            }
        }
        ("POST", "/api/rescan-claude") => {
            let body = serde_json::from_slice::<RescanRequest>(&request.body);
            match body {
                Ok(body) => core
                    .rescan_claude(normalize_path(body.claude_home))
                    .map_or_else(api_error, |report| json_response(200, &report)),
                Err(error) => api_error(format!("invalid rescan request: {error}")),
            }
        }
        ("GET", "/api/detect-cc-switch") => {
            let detection = query_value(query, "cc_switch_db")
                .filter(|value| !value.trim().is_empty())
                .map(|path| CcSwitchAdapter::new(path).detect_sync());
            match detection {
                Some(Ok(result)) => json_response(200, &result),
                Some(Err(error)) => api_error(error.to_string()),
                None => core
                    .detect_cc_switch_path()
                    .map_or_else(api_error, |result| json_response(200, &result)),
            }
        }
        ("POST", "/api/rescan-cc-switch") => {
            let body = serde_json::from_slice::<RescanRequest>(&request.body);
            match body {
                Ok(body) => core
                    .rescan_cc_switch(normalize_path(body.cc_switch_db))
                    .map_or_else(api_error, |report| json_response(200, &report)),
                Err(error) => api_error(format!("invalid rescan request: {error}")),
            }
        }
        ("GET", "/api/detect-cockpit") => {
            let detection = query_value(query, "cockpit_db")
                .filter(|value| !value.trim().is_empty())
                .map(|path| CockpitAdapter::new(path).detect_sync());
            match detection {
                Some(Ok(result)) => json_response(200, &result),
                Some(Err(error)) => api_error(error.to_string()),
                None => core
                    .detect_cockpit_path()
                    .map_or_else(api_error, |result| json_response(200, &result)),
            }
        }
        ("POST", "/api/rescan-cockpit") => {
            let body = serde_json::from_slice::<RescanRequest>(&request.body);
            match body {
                Ok(body) => core
                    .rescan_cockpit(normalize_path(body.cockpit_db))
                    .map_or_else(api_error, |report| json_response(200, &report)),
                Err(error) => api_error(format!("invalid rescan request: {error}")),
            }
        }
        ("PUT", "/api/settings") | ("POST", "/api/settings") => {
            match serde_json::from_slice::<tokenbuddy_domain::AppSettings>(&request.body) {
                Ok(settings) => match core.get_app_settings() {
                    Ok(previous) => match core.update_app_settings(settings.clone()) {
                        Ok(()) => {
                            if previous.auto_start != settings.auto_start
                                && let Some(callback) = autostart
                                && let Err(error) = callback(settings.auto_start)
                            {
                                return api_error(error);
                            }
                            core.get_app_settings()
                                .map_or_else(api_error, |settings| json_response(200, &settings))
                        }
                        Err(error) => api_error(error),
                    },
                    Err(error) => api_error(error),
                },
                Err(error) => api_error(format!("invalid settings request: {error}")),
            }
        }
        ("GET", path) if path.starts_with("/api/sessions/") => {
            let session_id = percent_decode(path.trim_start_matches("/api/sessions/"));
            core.get_session_detail(&session_id)
                .map_or_else(api_error, |detail| json_response(200, &detail))
        }
        ("GET", _) => serve_static(static_root, path),
        _ => text_response(405, "method not allowed", "text/plain; charset=utf-8"),
    }
}

fn serve_static(root: &Path, path: &str) -> Vec<u8> {
    let decoded = percent_decode(path.trim_start_matches('/'));
    // Anything that is not a real file inside `root` falls back to the SPA
    // entry point, exactly as before — but resolution now refuses to escape the
    // served directory (see resolve_static_file).
    let file = resolve_static_file(root, &decoded).unwrap_or_else(|| root.join("index.html"));
    match fs::read(&file) {
        Ok(body) => raw_response(200, content_type(&file), &body),
        Err(_) => text_response(404, "web build not found", "text/plain; charset=utf-8"),
    }
}

/// Resolve a decoded request path to a real file *inside* `root`, or `None` if
/// it escapes the served directory or does not name a file.
///
/// The previous implementation only rejected literal `..` segments, which let a
/// percent-encoded absolute path (`/%2Fetc%2Fpasswd`) slip through: `Path::join`
/// silently discards `root` when the joined path is absolute, so the server
/// happily read arbitrary files off disk. We now require every component to be a
/// plain name (rejecting root/prefix/`..` components) and then canonicalize and
/// verify containment, so neither absolute paths nor symlinks can point outside
/// the build directory.
fn resolve_static_file(root: &Path, decoded: &str) -> Option<PathBuf> {
    if decoded.is_empty() {
        return None;
    }
    let relative = Path::new(decoded);
    let all_plain_names = relative
        .components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir));
    if !all_plain_names {
        return None;
    }
    let canonical_root = fs::canonicalize(root).ok()?;
    let canonical_file = fs::canonicalize(canonical_root.join(relative)).ok()?;
    if !canonical_file.starts_with(&canonical_root) || !canonical_file.is_file() {
        return None;
    }
    Some(canonical_file)
}

fn json_response<T: Serialize>(status: u16, value: &T) -> Vec<u8> {
    match serde_json::to_vec(value) {
        Ok(body) => raw_response(status, "application/json; charset=utf-8", &body),
        Err(error) => api_error(error.to_string()),
    }
}

fn api_error(error: impl ToString) -> Vec<u8> {
    let body = serde_json::json!({"error": error.to_string()});
    json_response(500, &body)
}

fn raw_response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut response = head.into_bytes();
    response.extend_from_slice(body);
    response
}

fn text_response(status: u16, body: &str, content_type: &str) -> Vec<u8> {
    raw_response(status, content_type, body.as_bytes())
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn query_value(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (name, value) = part.split_once('=')?;
        (name == key).then(|| percent_decode(value))
    })
}

fn usage_filters_from_query(query: &str) -> UsageFilters {
    UsageFilters {
        period_start: query_value(query, "period_start").and_then(|value| parse_datetime(&value)),
        period_end: query_value(query, "period_end").and_then(|value| parse_datetime(&value)),
        app: query_value(query, "app").and_then(|value| match value.as_str() {
            "codex" => Some(AppKind::Codex),
            "claude_code" => Some(AppKind::ClaudeCode),
            "unknown" => Some(AppKind::Unknown),
            _ => None,
        }),
        provider_id: query_value(query, "provider_id"),
        account_id: query_value(query, "account_id"),
        model: query_value(query, "model"),
        project_path: query_value(query, "project_path"),
        precision: query_value(query, "precision").and_then(|value| match value.as_str() {
            "verified" => Some(PrecisionLevel::Verified),
            "exact_session" => Some(PrecisionLevel::ExactSession),
            "correlated" => Some(PrecisionLevel::Correlated),
            "estimated" => Some(PrecisionLevel::Estimated),
            "unavailable" => Some(PrecisionLevel::Unavailable),
            _ => None,
        }),
        search: query_value(query, "search"),
    }
}

fn parse_datetime(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc))
}

fn normalize_path(path: Option<String>) -> Option<PathBuf> {
    path.filter(|value| !value.trim().is_empty())
        .map(|value| PathBuf::from(value.trim()))
}

fn percent_decode(value: &str) -> String {
    let mut output = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => output.push(b' '),
            b'%' if index + 2 < bytes.len() => {
                let high = hex_value(bytes[index + 1]);
                let low = hex_value(bytes[index + 2]);
                if let (Some(high), Some(low)) = (high, low) {
                    output.push((high << 4) | low);
                    index += 2;
                } else {
                    output.push(bytes[index]);
                }
            }
            byte => output.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{Read, Write},
        net::{Ipv6Addr, TcpStream},
        path::PathBuf,
        sync::Arc,
        time::{Duration, Instant},
    };

    use tempfile::tempdir;
    use tokenbuddy_core::{Core, CoreConfig};

    use super::{LocalWebServer, percent_decode, query_value, resolve_static_file, serve_static};

    #[test]
    fn static_resolution_stays_inside_the_web_root() {
        let root = tempdir().expect("web root");
        fs::write(root.path().join("index.html"), b"ok").expect("index");
        fs::create_dir(root.path().join("assets")).expect("assets dir");
        fs::write(root.path().join("assets/app.js"), b"//js").expect("asset");

        assert!(resolve_static_file(root.path(), "index.html").is_some());
        assert!(resolve_static_file(root.path(), "assets/app.js").is_some());
        // Traversal, absolute paths, and non-files are all refused.
        assert!(resolve_static_file(root.path(), "../secret").is_none());
        assert!(resolve_static_file(root.path(), "/etc/passwd").is_none());
        assert!(resolve_static_file(root.path(), "assets/../../secret").is_none());
        assert!(resolve_static_file(root.path(), "assets").is_none());
        assert!(resolve_static_file(root.path(), "").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn percent_encoded_absolute_path_cannot_read_outside_the_web_root() {
        let root = tempdir().expect("web root");
        fs::write(root.path().join("index.html"), b"INDEX-FALLBACK").expect("index");
        let outside = tempdir().expect("outside dir");
        let secret = outside.path().join("secret.txt");
        fs::write(&secret, b"TOP-SECRET").expect("secret");

        // Percent-encode the separators so the naive `..`-only check is bypassed,
        // exactly like the original `GET /%2Fetc%2Fpasswd` exploit.
        let encoded: String = secret
            .to_string_lossy()
            .chars()
            .map(|character| {
                if character == '/' {
                    "%2F".to_owned()
                } else {
                    character.to_string()
                }
            })
            .collect();
        let response = String::from_utf8_lossy(&serve_static(root.path(), &encoded)).into_owned();
        assert!(
            !response.contains("TOP-SECRET"),
            "static server leaked a file outside the web root"
        );
        assert!(response.contains("INDEX-FALLBACK"));
    }

    #[test]
    fn decodes_loopback_api_query_values_without_external_dependencies() {
        assert_eq!(percent_decode("/tmp/hello%20world"), "/tmp/hello world");
        assert_eq!(
            query_value("search=hello%20world&limit=10", "search"),
            Some("hello world".to_owned())
        );
        assert_eq!(
            query_value("search=hello%20world&limit=10", "limit"),
            Some("10".to_owned())
        );
    }

    #[test]
    fn local_api_serves_core_data_over_both_loopback_families() {
        let database = tempdir().expect("database directory");
        let mut config = CoreConfig::new(database.path().join("tokenbuddy.sqlite3"), None);
        config.poll_interval = Duration::from_millis(10);
        let core = Core::start(config).expect("core starts");
        let server = LocalWebServer::start(Arc::clone(&core), PathBuf::from("/missing"))
            .expect("web server starts");
        let url = server.status().url.expect("server URL");
        let port = url
            .rsplit(':')
            .next()
            .expect("port")
            .parse::<u16>()
            .expect("valid port");
        assert_eq!(server.status().loopback_urls.len(), 2);
        let request =
            b"GET /api/quick-summary HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
        let mut ipv4_stream = TcpStream::connect(("127.0.0.1", port)).expect("IPv4 loopback");
        ipv4_stream.write_all(request).expect("IPv4 request");
        let mut ipv4_response = String::new();
        ipv4_stream
            .read_to_string(&mut ipv4_response)
            .expect("IPv4 response");

        let mut ipv6_stream =
            TcpStream::connect((Ipv6Addr::LOCALHOST, port)).expect("IPv6 loopback");
        ipv6_stream.write_all(request).expect("IPv6 request");
        let mut ipv6_response = String::new();
        ipv6_stream
            .read_to_string(&mut ipv6_response)
            .expect("IPv6 response");

        assert!(ipv4_response.starts_with("HTTP/1.1 200 OK"));
        assert!(ipv6_response.starts_with("HTTP/1.1 200 OK"));
        assert_eq!(response_body(&ipv4_response), response_body(&ipv6_response));
        assert!(ipv4_response.contains("\"collection_status\":\"idle\""));
        drop(server);
        core.shutdown().expect("core stops");
    }

    #[test]
    fn web_entrypoint_matches_the_core_query_contract() {
        let database = tempdir().expect("database directory");
        let core = Core::start(CoreConfig::new(
            database.path().join("tokenbuddy.sqlite3"),
            None,
        ))
        .expect("core starts");
        let response = super::route_request(
            &super::Request {
                method: "GET".to_owned(),
                target: "/api/quick-summary".to_owned(),
                body: Vec::new(),
            },
            &core,
            PathBuf::from("/missing").as_path(),
        );
        let response = String::from_utf8(response).expect("UTF-8 response");
        let web_summary: tokenbuddy_domain::QuickSummary =
            serde_json::from_str(response_body(&response)).expect("summary JSON");
        assert_eq!(web_summary, core.quick_summary().expect("core summary"));
        core.shutdown().expect("core stops");
    }

    #[test]
    fn shared_read_routes_return_core_owned_data_for_all_phase_four_b_views() {
        let database = tempdir().expect("database directory");
        let core = Core::start(CoreConfig::new(
            database.path().join("tokenbuddy.sqlite3"),
            None,
        ))
        .expect("core starts");
        for target in ["/api/providers", "/api/quotas", "/api/settings"] {
            let response = super::route_request(
                &super::Request {
                    method: "GET".to_owned(),
                    target: target.to_owned(),
                    body: Vec::new(),
                },
                &core,
                PathBuf::from("/missing").as_path(),
            );
            let response = String::from_utf8(response).expect("UTF-8 response");
            assert!(response.starts_with("HTTP/1.1 200 OK"), "{target}");
        }
        core.shutdown().expect("core stops");
    }

    #[test]
    fn local_api_exposes_claude_detection_and_rescan() {
        let database = tempdir().expect("database directory");
        let claude_home = tempdir().expect("Claude home");
        fs::create_dir_all(claude_home.path().join("projects")).expect("projects directory");
        let core = Core::start(CoreConfig::new(
            database.path().join("tokenbuddy.sqlite3"),
            None,
        ))
        .expect("core starts");
        let path = claude_home.path().to_string_lossy();
        let response = super::route_request(
            &super::Request {
                method: "GET".to_owned(),
                target: format!("/api/detect-claude?claude_home={path}"),
                body: Vec::new(),
            },
            &core,
            PathBuf::from("/missing").as_path(),
        );
        let response = String::from_utf8(response).expect("detection response");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("claude-code-session"));
        assert!(response.contains("\"detected\":true"));

        let response = super::route_request(
            &super::Request {
                method: "POST".to_owned(),
                target: "/api/rescan-claude".to_owned(),
                body: serde_json::to_vec(&serde_json::json!({
                    "claude_home": path.as_ref(),
                }))
                .expect("rescan JSON"),
            },
            &core,
            PathBuf::from("/missing").as_path(),
        );
        let response = String::from_utf8(response).expect("rescan response");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert_eq!(
            core.get_app_settings()
                .expect("settings")
                .claude_home
                .as_deref(),
            Some(path.as_ref())
        );
        core.shutdown().expect("core stops");
    }

    #[test]
    fn dashboard_filters_and_export_use_the_same_local_api_contract() {
        let database = tempdir().expect("database directory");
        let core = Core::start(CoreConfig::new(
            database.path().join("tokenbuddy.sqlite3"),
            None,
        ))
        .expect("core starts");
        let dashboard_response = super::route_request(
            &super::Request {
                method: "GET".to_owned(),
                target: "/api/dashboard-summary?app=codex&precision=verified".to_owned(),
                body: Vec::new(),
            },
            &core,
            PathBuf::from("/missing").as_path(),
        );
        assert!(
            String::from_utf8(dashboard_response)
                .expect("dashboard response")
                .starts_with("HTTP/1.1 200 OK")
        );

        let export_response = super::route_request(
            &super::Request {
                method: "POST".to_owned(),
                target: "/api/export".to_owned(),
                body: br#"{"format":"csv","filters":{"app":"codex"}}"#.to_vec(),
            },
            &core,
            PathBuf::from("/missing").as_path(),
        );
        let export_response = String::from_utf8(export_response).expect("export response");
        assert!(export_response.starts_with("HTTP/1.1 200 OK"));
        assert!(export_response.contains("id,occurred_at,app"));
        core.shutdown().expect("core stops");
    }

    #[test]
    fn quick_summary_http_p95_stays_within_lightweight_entry_budget() {
        let database = tempdir().expect("database directory");
        let core = Core::start(CoreConfig::new(
            database.path().join("tokenbuddy.sqlite3"),
            None,
        ))
        .expect("core starts");
        let server = LocalWebServer::start(Arc::clone(&core), PathBuf::from("/missing"))
            .expect("web server starts");
        let url = server.status().url.expect("server URL");
        let port = url
            .rsplit(':')
            .next()
            .expect("port")
            .parse::<u16>()
            .expect("valid port");
        let request =
            b"GET /api/quick-summary HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
        let mut samples = (0..50)
            .map(|_| {
                let started = Instant::now();
                let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("loopback");
                stream.write_all(request).expect("request");
                let mut response = String::new();
                stream.read_to_string(&mut response).expect("response");
                assert!(response.starts_with("HTTP/1.1 200 OK"));
                started.elapsed()
            })
            .collect::<Vec<_>>();
        samples.sort_unstable();
        let p95 = samples[((samples.len() * 95).div_ceil(100)).saturating_sub(1)];
        println!("QuickSummary HTTP P95: {} ms", p95.as_secs_f64() * 1_000.0);
        assert!(p95 < Duration::from_millis(200));
        drop(server);
        core.shutdown().expect("core stops");
    }

    fn response_body(response: &str) -> &str {
        response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .expect("HTTP response body")
    }
}
