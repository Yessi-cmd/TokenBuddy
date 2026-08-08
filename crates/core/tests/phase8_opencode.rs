use std::{path::Path, path::PathBuf, time::Duration};

use rusqlite::Connection;
use tempfile::TempDir;
use tokenbuddy_core::{Core, CoreConfig};
use tokenbuddy_domain::{AppKind, PrecisionLevel};

/// Sanitized millisecond epoch used by the fixture.
const MS: i64 = 1_786_114_981_086;

/// Build a sanitized OpenCode database: one session with two step-finish
/// parts, one subagent session, and noise that must not produce events.
fn opencode_fixture(dir: &TempDir) -> PathBuf {
    let path = dir.path().join("opencode.db");
    let connection = Connection::open(&path).expect("open fixture");
    connection
        .execute_batch(&format!(
            "CREATE TABLE session (
                     id TEXT PRIMARY KEY,
                     project_id TEXT NOT NULL,
                     parent_id TEXT,
                     directory TEXT NOT NULL,
                     title TEXT NOT NULL,
                     model TEXT,
                     agent TEXT,
                     cost REAL NOT NULL DEFAULT 0,
                     tokens_input INTEGER NOT NULL DEFAULT 0,
                     tokens_output INTEGER NOT NULL DEFAULT 0,
                     time_created INTEGER NOT NULL,
                     time_updated INTEGER NOT NULL
                 );
                 CREATE TABLE message (
                     id TEXT PRIMARY KEY,
                     session_id TEXT NOT NULL,
                     time_created INTEGER NOT NULL,
                     time_updated INTEGER NOT NULL,
                     data TEXT NOT NULL
                 );
                 CREATE TABLE part (
                     id TEXT PRIMARY KEY,
                     message_id TEXT NOT NULL,
                     session_id TEXT NOT NULL,
                     time_created INTEGER NOT NULL,
                     time_updated INTEGER NOT NULL,
                     data TEXT NOT NULL
                 );
                 INSERT INTO session VALUES
                     ('ses_main', 'proj', NULL, '/work/demo', 'sanitized-opencode-session',
                      '{{\"id\":\"deepseek-v4-flash\",\"providerID\":\"opencode-go\"}}',
                      'build', 0.0022799308, 29357, 528, {main_start}, {main_end}),
                     ('ses_sub', 'proj', 'ses_main', '/work/demo', 'subagent task',
                      '{{\"id\":\"claude-opus-5\",\"providerID\":\"anthropic\"}}',
                      'build', 0.0, 0, 0, {sub_start}, {sub_end});
                 INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES
                     ('msg_1', 'ses_main', {main_start}, {main_start},
                      '{{\"role\":\"assistant\",\"model\":{{\"modelID\":\"deepseek-v4-flash\"}}}}'),
                     ('msg_2', 'ses_main', {msg2}, {msg2}, '{{\"role\":\"assistant\"}}'),
                     ('msg_3', 'ses_sub', {sub_start}, {sub_start}, '{{\"role\":\"assistant\"}}');
                 INSERT INTO part VALUES
                     ('prt_1', 'msg_1', 'ses_main', {main_start}, {main_start},
                      '{{\"type\":\"step-finish\",\"reason\":\"tool-calls\",
                        \"tokens\":{{\"input\":1425,\"output\":224,\"reasoning\":215,
                                    \"cache\":{{\"write\":0,\"read\":30080}}}},
                        \"cost\":0.000203322}}'),
                     ('prt_2', 'msg_2', 'ses_main', {msg2}, {msg2},
                      '{{\"type\":\"step-finish\",\"reason\":\"tool-calls\",
                        \"tokens\":{{\"input\":596,\"output\":83,\"reasoning\":18,
                                    \"cache\":{{\"write\":0,\"read\":31872}}}}}}'),
                     ('prt_3', 'msg_3', 'ses_sub', {sub_start}, {sub_start},
                      '{{\"type\":\"step-finish\",
                        \"tokens\":{{\"input\":500,\"output\":100,\"reasoning\":0,
                                    \"cache\":{{\"write\":200,\"read\":200}}}}}}'),
                     ('prt_noise', 'msg_1', 'ses_main', {noise}, {noise},
                      '{{\"type\":\"text\",\"text\":\"not a usage record\"}}');
                ",
            main_start = MS,
            main_end = MS + 60_000,
            sub_start = MS + 70_000,
            sub_end = MS + 80_000,
            msg2 = MS + 1000,
            noise = MS + 2000,
        ))
        .expect("seed fixture");
    drop(connection);
    path
}

fn core_with_opencode(database: &TempDir, opencode_db: PathBuf) -> std::sync::Arc<Core> {
    let mut config = CoreConfig::new(database.path().join("tokenbuddy.sqlite3"), None)
        .with_opencode_db(Some(opencode_db));
    config.poll_interval = Duration::from_secs(60);
    config.enable_file_watcher = false;
    Core::start(config).expect("core starts")
}

#[test]
fn core_imports_opencode_events_into_shared_queries_and_summary() {
    let data = tempfile::tempdir().expect("opencode data dir");
    let opencode_db = opencode_fixture(&data);
    let database = tempfile::tempdir().expect("database directory");
    let core = core_with_opencode(&database, opencode_db.clone());

    let events = core.list_usage_events(None, 100, 0).expect("events");
    assert_eq!(events.total, 3);
    assert!(
        events
            .events
            .iter()
            .all(|event| event.app == AppKind::OpenCode)
    );
    let main = events
        .events
        .iter()
        .find(|event| event.model.as_deref() == Some("deepseek-v4-flash"))
        .expect("main-session request");
    assert_eq!(main.usage.input_tokens_total, Some(31505));
    assert_eq!(main.usage.cache_read_tokens, Some(30080));
    // OpenCode's own computed cost is an estimate, stored as such.
    assert_eq!(main.estimated_cost, Some(0.000203322));
    assert_eq!(main.provider_reported_cost, None);
    assert_eq!(main.precision_token, PrecisionLevel::ExactSession);
    // The default privacy policy does not persist request metadata.
    assert_eq!(main.raw_usage_json, None);

    let summary = core.quick_summary().expect("QuickSummary");
    assert_eq!(summary.active_app, Some(AppKind::OpenCode));
    // The newest request belongs to the subagent session, so the tray follows
    // it; the subagent inherits the parent chain rather than inventing one.
    assert_eq!(
        summary.active_session_title.as_deref(),
        Some("subagent task")
    );

    let sessions = core
        .list_sessions(&Default::default(), 100, 0)
        .expect("sessions");
    assert_eq!(sessions.total, 2);
    let main_session = sessions
        .sessions
        .iter()
        .find(|summary| summary.session.title.as_deref() == Some("sanitized-opencode-session"))
        .expect("main session");
    assert_eq!(main_session.totals.input_tokens_total, Some(63973));
    assert_eq!(main_session.totals.output_tokens_total, Some(307));

    let sources = core.list_sources().expect("sources");
    assert_eq!(
        sources
            .iter()
            .find(|source| source.id == "opencode")
            .and_then(|source| source.health_status.as_deref()),
        Some("healthy")
    );
    assert_eq!(
        sources
            .iter()
            .find(|source| source.id == "opencode")
            .map(|source| source.adapter_type.as_str()),
        Some("opencode")
    );
}

#[test]
fn opencode_reimport_is_idempotent_and_misses_nothing_after_an_append() {
    let data = tempfile::tempdir().expect("opencode data dir");
    let opencode_db = opencode_fixture(&data);
    let database = tempfile::tempdir().expect("database directory");
    let core = core_with_opencode(&database, opencode_db.clone());

    let first = core.list_usage_events(None, 100, 0).expect("events");
    let report = core.rescan_opencode(None).expect("refresh");
    assert_eq!(report.inserted_events, 0, "no event may be counted twice");
    let after = core.list_usage_events(None, 100, 0).expect("events");
    assert_eq!(after.total, first.total);
    assert_eq!(after.total, 3);

    // A new step-finish appended to the live database is picked up by the next
    // pass without rescanning history.
    let connection =
        Connection::open(core.opencode_db().expect("configured db").expect("path")).expect("open");
    connection
        .execute(
            "INSERT INTO part VALUES ('prt_new', 'msg_1', 'ses_main', ?1, ?1,
              '{\"type\":\"step-finish\",
                \"tokens\":{\"input\":10,\"output\":5,\"reasoning\":1,
                            \"cache\":{\"write\":2,\"read\":0}}}')",
            [MS + 90_000],
        )
        .expect("append part");
    drop(connection);
    let report = core.rescan_opencode(None).expect("refresh after append");
    assert_eq!(report.inserted_events, 1);
    let after = core.list_usage_events(None, 100, 0).expect("events");
    assert_eq!(after.total, 4);
}

#[test]
fn opencode_source_is_detected_and_missing_after_removal() {
    let data = tempfile::tempdir().expect("opencode data dir");
    let opencode_db = opencode_fixture(&data);
    let database = tempfile::tempdir().expect("database directory");
    let core = core_with_opencode(&database, opencode_db.clone());

    let detection = core.detect_opencode_path().expect("detection");
    assert!(detection.detected);
    assert_eq!(detection.detected_version.as_deref(), Some("sqlite"));

    let sources = core.list_sources().expect("sources");
    let source = sources
        .iter()
        .find(|source| source.id == "opencode")
        .expect("opencode source row");
    assert_eq!(source.health_status.as_deref(), Some("healthy"));
    assert_eq!(source.detected_version.as_deref(), Some("sqlite"));

    // Removing the database makes the source degrade to not_found on the next
    // pass instead of crashing the Core.
    std::fs::remove_file(Path::new(&opencode_db)).expect("remove database");
    core.rescan_opencode(None).expect("refresh after removal");
    let sources = core.list_sources().expect("sources");
    let source = sources
        .iter()
        .find(|source| source.id == "opencode")
        .expect("opencode source row");
    assert_eq!(source.health_status.as_deref(), Some("not_found"));
}

#[test]
fn opencode_sessions_and_events_are_attributable_but_not_invented() {
    let data = tempfile::tempdir().expect("opencode data dir");
    let opencode_db = opencode_fixture(&data);
    let database = tempfile::tempdir().expect("database directory");
    let core = core_with_opencode(&database, opencode_db.clone());

    // The subagent event carries the parent chain through the shared model.
    let events = core.list_usage_events(None, 100, 0).expect("events");
    let sub = events
        .events
        .iter()
        .find(|event| event.usage.input_tokens_uncached == Some(500))
        .expect("subagent event");
    assert_eq!(sub.precision_session, PrecisionLevel::ExactSession);
    assert!(sub.parent_session_id.is_some());

    // Provider and account attribution stays explicitly unavailable; the
    // derived placeholder provider exists so the Providers view can render, but
    // it never claims a real vendor.
    let providers = core.list_providers().expect("providers");
    let unknown = providers
        .iter()
        .find(|provider| provider.id == "unknown")
        .expect("derived placeholder provider");
    assert_eq!(unknown.provider_family, "unknown");
    // The model-prefix guess attributes the claude-named subagent request to
    // Anthropic; everything else stays explicitly unknown.
    assert_eq!(unknown.request_count, 2);
    assert_eq!(
        providers
            .iter()
            .find(|provider| provider.id == "anthropic")
            .map(|provider| provider.request_count),
        Some(1)
    );
}

#[test]
fn unsupported_opencode_database_reports_a_source_error_not_a_crash() {
    let data = tempfile::tempdir().expect("data dir");
    let path = data.path().join("opencode.db");
    let connection = Connection::open(&path).expect("open");
    connection
        .execute_batch("CREATE TABLE unrelated (id INTEGER);")
        .expect("seed");
    drop(connection);
    let database = tempfile::tempdir().expect("database directory");
    let core = core_with_opencode(&database, path);

    let report = core.rescan_opencode(None).expect("refresh");
    assert!(report.warning.is_some());
    let sources = core.list_sources().expect("sources");
    let source = sources
        .iter()
        .find(|source| source.id == "opencode")
        .expect("opencode source row");
    assert_eq!(source.health_status.as_deref(), Some("error"));
    assert!(
        source
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("session")),
        "unexpected error: {:?}",
        source.last_error
    );
    // The Core stays healthy: Codex/Claude sources are unaffected.
    assert!(
        core.list_usage_events(None, 100, 0)
            .expect("events")
            .events
            .is_empty()
    );
}
