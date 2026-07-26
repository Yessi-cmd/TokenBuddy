use std::{
    fs,
    io::Write,
    path::Path,
    thread,
    time::{Duration, Instant},
};

use tempfile::{NamedTempFile, TempDir};
use tokenbuddy_core::{Core, CoreConfig};
use tokenbuddy_domain::AppKind;

fn claude_fixture_home(fixture: &str) -> (TempDir, std::path::PathBuf) {
    let home = tempfile::tempdir().expect("Claude home");
    let project = home.path().join("projects").join("sanitized-project");
    fs::create_dir_all(&project).expect("Claude project directory");
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/claude")
        .join(fixture);
    let destination = project.join(fixture);
    fs::copy(fixture_path, &destination).expect("copy Claude fixture");
    (home, destination)
}

#[test]
fn core_imports_claude_events_into_shared_queries_and_summary() {
    let (home, _) = claude_fixture_home("simple_session.jsonl");
    let database = tempfile::tempdir().expect("database directory");
    let mut config = CoreConfig::new(database.path().join("tokenbuddy.sqlite3"), None)
        .with_claude_home(Some(home.path().to_owned()));
    config.poll_interval = Duration::from_secs(60);
    config.enable_file_watcher = false;
    let core = Core::start(config).expect("core starts");

    let events = core.list_usage_events(None, 100, 0).expect("Claude events");
    assert_eq!(events.total, 2);
    assert!(
        events
            .events
            .iter()
            .all(|event| event.app == AppKind::ClaudeCode)
    );
    assert_eq!(events.events[0].usage.input_tokens_total, Some(150));
    // Session logs carry no provider, but the model prefix identifies one so the
    // Providers view reflects real usage instead of staying permanently empty.
    assert_eq!(events.events[0].provider_id.as_deref(), Some("anthropic"));
    let raw_usage = events.events[0]
        .raw_usage_json
        .as_ref()
        .expect("raw Claude usage");
    assert_eq!(
        raw_usage
            .get("input_tokens")
            .and_then(|value| value.as_u64()),
        Some(100)
    );
    assert!(!raw_usage.to_string().contains("REDACTED_PROMPT"));

    let summary = core.quick_summary().expect("QuickSummary");
    assert_eq!(summary.active_app, Some(AppKind::ClaudeCode));
    assert_eq!(summary.session_input_tokens, Some(320));
    assert_eq!(summary.session_output_tokens, Some(90));
    assert!(summary.today_total_tokens.is_some());
    assert_eq!(
        summary.active_session_title.as_deref(),
        Some("sanitized-claude-session")
    );

    assert_eq!(summary.provider_name.as_deref(), Some("Anthropic"));

    let sources = core.list_sources().expect("sources");
    assert_eq!(
        sources
            .iter()
            .find(|source| source.id == "claude-code-session")
            .and_then(|source| source.health_status.as_deref()),
        Some("healthy")
    );

    // The Providers view is populated from the derived provider rather than left
    // empty as it was before real usage flowed into it.
    let providers = core.list_providers().expect("providers");
    let anthropic = providers
        .iter()
        .find(|provider| provider.id == "anthropic")
        .expect("derived Anthropic provider");
    assert_eq!(anthropic.display_name, "Anthropic");
    assert_eq!(anthropic.request_count, 2);
    core.shutdown().expect("core stops");
}

#[test]
fn core_wakes_on_claude_session_append_and_keeps_cursor_incremental() {
    let (home, path) = claude_fixture_home("simple_session.jsonl");
    let database = tempfile::tempdir().expect("database directory");
    let mut config = CoreConfig::new(database.path().join("tokenbuddy.sqlite3"), None)
        .with_claude_home(Some(home.path().to_owned()));
    config.poll_interval = Duration::from_millis(50);
    let core = Core::start(config).expect("core starts");
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("append Claude fixture");
    writeln!(
        file,
        "{{\"type\":\"assistant\",\"sessionId\":\"claude-simple-session\",\"timestamp\":\"2026-07-26T08:00:04Z\",\"message\":{{\"id\":\"message-003\",\"model\":\"claude-3-7-sonnet\",\"usage\":{{\"input_tokens\":10,\"cache_creation_input_tokens\":2,\"cache_read_input_tokens\":3,\"output_tokens\":4}}}}}}"
    )
    .expect("write Claude record");

    let started = Instant::now();
    loop {
        if core
            .list_usage_events(None, 100, 0)
            .expect("read Claude events")
            .total
            == 3
        {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(2));
        thread::sleep(Duration::from_millis(15));
    }
    assert_eq!(
        core.list_usage_events(None, 100, 0)
            .expect("read appended event")
            .events
            .last()
            .and_then(|event| event.usage.input_tokens_total),
        Some(15)
    );
    core.shutdown().expect("core stops");
}

#[test]
fn claude_adapter_failure_does_not_block_codex_import() {
    let codex_home = tempfile::tempdir().expect("Codex home");
    let codex_sessions = codex_home.path().join("sessions");
    fs::create_dir_all(&codex_sessions).expect("Codex sessions directory");
    let codex_fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/codex/simple_session.jsonl");
    fs::copy(codex_fixture, codex_sessions.join("simple_session.jsonl"))
        .expect("copy Codex fixture");
    let claude_home = NamedTempFile::new().expect("invalid Claude home");
    let database = tempfile::tempdir().expect("database directory");
    let mut config = CoreConfig::new(
        database.path().join("tokenbuddy.sqlite3"),
        Some(codex_home.path().to_owned()),
    )
    .with_claude_home(Some(claude_home.path().to_owned()));
    config.poll_interval = Duration::from_secs(60);
    config.enable_file_watcher = false;
    let core = Core::start(config).expect("core starts");

    assert_eq!(
        core.list_usage_events(None, 100, 0)
            .expect("Codex events")
            .total,
        2
    );
    let sources = core.list_sources().expect("sources");
    assert_eq!(
        sources
            .iter()
            .find(|source| source.id == "claude-code-session")
            .and_then(|source| source.health_status.as_deref()),
        Some("error")
    );
    assert!(
        sources
            .iter()
            .find(|source| source.id == "claude-code-session")
            .and_then(|source| source.last_error.as_deref())
            .is_some_and(|error| error.contains("not a directory"))
    );
    assert_eq!(
        core.quick_summary().expect("QuickSummary").active_app,
        Some(AppKind::Codex)
    );
    core.shutdown().expect("core stops");
}
