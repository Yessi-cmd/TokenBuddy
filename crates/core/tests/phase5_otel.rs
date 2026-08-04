//! Phase 5 integration coverage for the optional loopback OTLP receiver.
#![allow(clippy::missing_docs_in_private_items)]

use std::{
    io::Write,
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};

use tempfile::tempdir;
use tokenbuddy_core::{Core, CoreConfig};
use tokenbuddy_domain::AppSettings;

#[test]
fn enabled_loopback_otel_receiver_imports_usage_through_the_core_boundary() {
    let database = tempdir().expect("database directory");
    let mut config = CoreConfig::new(database.path().join("tokenbuddy.sqlite3"), None);
    config.enable_file_watcher = false;
    let core = Core::start(config).expect("core starts");
    core.update_app_settings(AppSettings {
        otel_port: Some(0),
        ..AppSettings::default()
    })
    .expect("enable OTel");

    let endpoint = core
        .otel_endpoint()
        .expect("endpoint query")
        .expect("receiver bound");
    let address = endpoint
        .strip_prefix("http://")
        .and_then(|value| value.split('/').next())
        .expect("loopback endpoint");
    let payload = br#"{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"codex"}}]},"scopeSpans":[{"spans":[{"traceId":"01","spanId":"02","startTimeUnixNano":"1721900000000000000","attributes":[{"key":"gen_ai.request.id","value":{"stringValue":"otel-request"}},{"key":"gen_ai.conversation.id","value":{"stringValue":"otel-session"}},{"key":"gen_ai.usage.input_tokens","value":{"intValue":"40"}},{"key":"gen_ai.usage.output_tokens","value":{"intValue":"12"}},{"key":"gen_ai.prompt","value":{"stringValue":"must not persist"}}]}]}]}]}"#;
    let mut stream = TcpStream::connect(address).expect("connect loopback receiver");
    write!(
        stream,
        "POST /v1/traces HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    )
    .expect("write headers");
    stream.write_all(payload).expect("write payload");

    let deadline = Instant::now() + Duration::from_secs(2);
    let events = loop {
        let events = core.list_usage_events(None, 100, 0).expect("query events");
        if events.total == 1 {
            break events.events;
        }
        assert!(Instant::now() < deadline, "OTel event was not imported");
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(events[0].request_id.as_deref(), Some("otel-request"));
    assert_eq!(events[0].usage.input_tokens_total, Some(40));
    assert_eq!(events[0].usage.output_tokens_total, Some(12));
    assert!(events[0].raw_usage_json.is_none());
    let sessions = core
        .list_sessions(&Default::default(), 100, 0)
        .expect("query sessions");
    assert_eq!(sessions.total, 1);

    core.shutdown().expect("core stops");
}

#[test]
fn a_busy_otel_port_does_not_block_the_core_or_file_sources() {
    let occupied = TcpListener::bind(("127.0.0.1", 0)).expect("occupy loopback port");
    let port = occupied.local_addr().expect("occupied address").port();
    let database = tempdir().expect("database directory");
    let mut config = CoreConfig::new(database.path().join("tokenbuddy.sqlite3"), None);
    config.enable_file_watcher = false;
    let core = Core::start(config).expect("core starts");

    core.update_app_settings(AppSettings {
        otel_port: Some(port),
        ..AppSettings::default()
    })
    .expect("settings remain writable");
    assert_eq!(core.otel_endpoint().expect("endpoint query"), None);
    assert!(core.is_running());

    core.shutdown().expect("core stops");
    drop(occupied);
}
