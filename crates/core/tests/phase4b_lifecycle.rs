use std::{
    fs,
    io::Write,
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use chrono::Utc;
use tempfile::{TempDir, tempdir};
use tokenbuddy_core::{Core, CoreConfig};

fn fixture_home() -> (TempDir, std::path::PathBuf) {
    let home = tempdir().expect("temporary Codex home");
    let sessions = home.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions directory");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/codex/simple_session.jsonl");
    let session_path = sessions.join("simple_session.jsonl");
    fs::copy(fixture, &session_path).expect("copy sanitized fixture");
    (home, session_path)
}

#[test]
fn shared_entries_follow_one_core_through_native_refresh_and_shutdown() {
    let (home, session_path) = fixture_home();
    let database = tempdir().expect("database directory");
    let mut config = CoreConfig::new(
        database.path().join("tokenbuddy.sqlite3"),
        Some(home.path().to_owned()),
    );
    config.poll_interval = Duration::from_secs(5);
    let core = Core::start(config).expect("core starts");

    let tray_entry = Arc::clone(&core);
    let desktop_entry = Arc::clone(&core);
    let web_entry = Arc::clone(&core);
    assert!(Arc::ptr_eq(&tray_entry, &desktop_entry));
    assert!(Arc::ptr_eq(&desktop_entry, &web_entry));
    assert!(tray_entry.is_running());
    assert_eq!(
        tray_entry
            .list_usage_events(None, 100, 0)
            .expect("events")
            .total,
        2
    );

    writeln!(
        fs::OpenOptions::new()
            .append(true)
            .open(session_path)
            .expect("open sanitized fixture"),
        "{{\"type\":\"response.completed\",\"session_id\":\"simple-session\",\"timestamp\":\"{}\",\"request_id\":\"phase4b-integration\",\"model\":\"gpt-5-codex\",\"usage\":{{\"input_tokens\":20,\"output_tokens\":8}}}}",
        Utc::now().to_rfc3339(),
    )
    .expect("append sanitized fixture record");

    let started = Instant::now();
    loop {
        if tray_entry
            .list_usage_events(None, 100, 0)
            .expect("read events")
            .total
            == 3
        {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(2));
        thread::sleep(Duration::from_millis(15));
    }

    assert_eq!(
        tray_entry.quick_summary().expect("tray summary"),
        desktop_entry.quick_summary().expect("desktop summary")
    );
    assert_eq!(
        desktop_entry.quick_summary().expect("desktop summary"),
        web_entry.quick_summary().expect("web summary")
    );

    core.shutdown().expect("core shuts down");
    assert!(!tray_entry.is_running());
    assert!(!desktop_entry.is_running());
    assert!(!web_entry.is_running());
}
