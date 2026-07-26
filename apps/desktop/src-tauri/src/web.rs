use std::{
    fs,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokenbuddy_codex_session::CodexSessionAdapter;
use tokenbuddy_core::Core;

const MAX_REQUEST_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct LocalWebApiStatus {
    pub running: bool,
    pub url: Option<String>,
}

pub struct LocalWebServer {
    url: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl LocalWebServer {
    pub fn start(core: Arc<Core>, static_root: PathBuf) -> io::Result<Self> {
        // Bind explicitly to IPv4 loopback. This keeps the optional dashboard
        // inaccessible from the LAN; the server never binds 0.0.0.0.
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let url = format!("http://127.0.0.1:{port}");
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("tokenbuddy-web-api".to_owned())
            .spawn(move || serve(listener, core, static_root, thread_stop))?;
        Ok(Self {
            url,
            stop,
            worker: Some(worker),
        })
    }

    pub fn status(&self) -> LocalWebApiStatus {
        LocalWebApiStatus {
            running: true,
            url: Some(self.url.clone()),
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
}

fn serve(listener: TcpListener, core: Arc<Core>, static_root: PathBuf, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = handle_connection(&mut stream, &core, &static_root);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(15));
            }
            Err(_) => break,
        }
    }
}

fn handle_connection(stream: &mut TcpStream, core: &Core, static_root: &Path) -> io::Result<()> {
    let Some(request) = read_request(stream)? else {
        return Ok(());
    };
    let response = route_request(&request, core, static_root);
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

fn route_request(request: &Request, core: &Core, static_root: &Path) -> Vec<u8> {
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
            .today_dashboard_summary()
            .map_or_else(api_error, |summary| json_response(200, &summary)),
        ("GET", "/api/sources") => core
            .list_sources()
            .map_or_else(api_error, |sources| json_response(200, &sources)),
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
        ("GET", "/api/sessions") => {
            let search = query_value(query, "search");
            let limit = query_value(query, "limit")
                .and_then(|value| value.parse().ok())
                .unwrap_or(50);
            let offset = query_value(query, "offset")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            core.list_sessions(search.as_deref(), limit, offset)
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
            core.list_usage_events(session_id.as_deref(), limit, offset)
                .map_or_else(api_error, |events| json_response(200, &events))
        }
        ("POST", "/api/rescan-codex") => {
            let body = serde_json::from_slice::<RescanRequest>(&request.body);
            match body {
                Ok(body) => core
                    .rescan_codex(normalize_path(body.codex_home))
                    .map_or_else(api_error, |report| json_response(200, &report)),
                Err(error) => api_error(format!("invalid rescan request: {error}")),
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
    if decoded.split('/').any(|part| part == "..") {
        return text_response(404, "not found", "text/plain; charset=utf-8");
    }
    let requested = if decoded.is_empty() {
        root.join("index.html")
    } else {
        root.join(&decoded)
    };
    let file = if requested.is_file() {
        requested
    } else {
        root.join("index.html")
    };
    match fs::read(&file) {
        Ok(body) => raw_response(200, content_type(&file), &body),
        Err(_) => text_response(404, "web build not found", "text/plain; charset=utf-8"),
    }
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
        io::{Read, Write},
        net::TcpStream,
        path::PathBuf,
        sync::Arc,
        time::Duration,
    };

    use tempfile::tempdir;
    use tokenbuddy_core::{Core, CoreConfig};

    use super::{LocalWebServer, percent_decode, query_value};

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
    fn local_api_serves_core_data_over_ipv4_loopback() {
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
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("loopback connection");
        stream
            .write_all(
                b"GET /api/quick-summary HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .expect("request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("response");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("\"collection_status\":\"idle\""));
        drop(server);
        core.shutdown().expect("core stops");
    }
}
