//! The direct first-party quota source works without Cockpit or CC-Switch.

use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    thread,
    time::Duration,
};

use tokenbuddy_core::{Core, CoreConfig};
use tokenbuddy_domain::PrecisionLevel;

fn fixture_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("temporary home");
    let sessions = home.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions directory");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/codex");
    fs::copy(
        fixtures.join("rate_limits.jsonl"),
        sessions.join("rate_limits.jsonl"),
    )
    .expect("copy rollout fixture");
    fs::copy(
        fixtures.join("auth/chatgpt_auth.json"),
        home.path().join("auth.json"),
    )
    .expect("copy auth fixture");
    home
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
fn official_quota_is_collected_without_a_launcher_database() {
    let home = fixture_home();
    let body = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/codex/official_quota_response.json"),
    )
    .expect("official quota fixture");
    let (base_url, request_handle) = serve_once(body);
    let database = tempfile::tempdir().expect("database directory");
    let mut config = CoreConfig::new(
        database.path().join("tokenbuddy.sqlite3"),
        Some(home.path().to_owned()),
    )
    .with_official_quota_enabled(true)
    .with_official_quota_base_url(Some(format!("{base_url}/backend-api")));
    config.enable_file_watcher = false;
    config.poll_interval = Duration::from_secs(3_600);

    let core = Core::start(config).expect("core starts");
    let quotas = core.list_quota_snapshots(None, 100).expect("quota rows");
    assert!(quotas.iter().any(|quota| {
        quota.window_type == "primary_5h" && quota.precision == PrecisionLevel::Verified
    }));
    assert!(quotas.iter().any(|quota| quota.window_type == "credits"));
    assert_eq!(
        core.list_usage_events(None, 100, 0)
            .expect("usage events")
            .total,
        3
    );

    let source = core
        .list_sources()
        .expect("sources")
        .into_iter()
        .find(|source| source.id == "openai-official-quota")
        .expect("official quota source");
    assert_eq!(source.health_status.as_deref(), Some("healthy"));

    core.shutdown().expect("core stops");
    let request = request_handle.join().expect("server");
    assert!(request.contains("GET /backend-api/wham/usage HTTP/1.1"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("chatgpt-account-id: acct-fixture-0001")
    );
}
