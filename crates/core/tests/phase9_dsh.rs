use std::{fs, path::Path};

use tempfile::TempDir;
use tokenbuddy_core::{Core, CoreConfig};
use tokenbuddy_domain::{AppKind, PrecisionLevel};

/// Build a sanitized DeepSeek Harness home: one main session with two usage
/// records and one subagent child under `sessions/--sanitized--/`.
fn dsh_fixture_home(dir: &TempDir) {
    let root = dir.path().join("sessions").join("--sanitized--");
    let main = root.join("ses-dsh-main");
    fs::create_dir_all(&main).expect("main session dir");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dsh/simple_session.jsonl"),
        main.join("session.jsonl"),
    )
    .expect("copy main fixture");
    let child = root.join("ses-dsh-child");
    fs::create_dir_all(&child).expect("child session dir");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/dsh/subagent.jsonl"),
        child.join("session.jsonl"),
    )
    .expect("copy child fixture");
}

fn start_core(home: &TempDir) -> std::sync::Arc<Core> {
    let database = tempfile::tempdir().expect("database directory");
    let config = CoreConfig::new(database.path().join("tokenbuddy.sqlite3"), None)
        .with_dsh_home(Some(home.path().to_owned()));
    Core::start(config).expect("core starts")
}

#[test]
fn dsh_sessions_import_through_the_core_and_support_rescan() {
    let home = tempfile::tempdir().expect("dsh home");
    dsh_fixture_home(&home);
    let core = start_core(&home);

    let events = core.list_usage_events(None, 100, 0).expect("events");
    assert_eq!(events.total, 3, "main fixture has 2 records, child has 1");
    assert!(
        events
            .events
            .iter()
            .all(|event| event.app == AppKind::DeepseekHarness)
    );
    assert!(
        events
            .events
            .iter()
            .all(|event| event.precision_token == PrecisionLevel::ExactSession)
    );
    assert_eq!(
        events
            .events
            .iter()
            .filter(|event| event.query_source.as_deref() == Some("subagent"))
            .count(),
        1
    );

    let summary = core.quick_summary().expect("summary");
    assert_eq!(summary.active_app, Some(AppKind::DeepseekHarness));
    assert_eq!(summary.model.as_deref(), Some("deepseek-v4-pro"));

    // A rescan over unchanged input must not grow the counts.
    let report = core.rescan_dsh(None).expect("rescan");
    assert_eq!(report.inserted_events, 0);
    let after = core.list_usage_events(None, 100, 0).expect("events again");
    assert_eq!(after.total, 3);

    // Pointing the adapter at a path without a session root degrades the source
    // but leaves the Core usable.
    let empty = tempfile::tempdir().expect("empty home");
    let report = core
        .rescan_dsh(Some(empty.path().to_owned()))
        .expect("rescan empty");
    assert_eq!(report.inserted_events, 0);
    let sources = core.list_sources().expect("sources");
    let dsh = sources
        .iter()
        .find(|source| source.id == "dsh-session")
        .expect("dsh source");
    assert_eq!(dsh.health_status.as_deref(), Some("not_found"));

    core.shutdown().expect("core stops");
}

#[test]
fn dsh_imports_are_idempotent_across_restarts() {
    let home = tempfile::tempdir().expect("dsh home");
    dsh_fixture_home(&home);
    let database = tempfile::tempdir().expect("database directory");
    let database_path = database.path().join("tokenbuddy.sqlite3");
    let config = CoreConfig::new(&database_path, None).with_dsh_home(Some(home.path().to_owned()));

    let core = Core::start(config).expect("core starts");
    assert_eq!(
        core.list_usage_events(None, 100, 0).expect("events").total,
        3
    );
    core.shutdown().expect("core stops");

    // Restart on the same database and fixture: nothing is counted twice.
    let core = Core::start(
        CoreConfig::new(&database_path, None).with_dsh_home(Some(home.path().to_owned())),
    )
    .expect("core restarts");
    let events = core.list_usage_events(None, 100, 0).expect("events");
    assert_eq!(events.total, 3);
    core.shutdown().expect("core stops");
}

#[test]
fn dsh_source_failure_does_not_block_other_sources() {
    let codex_home = tempfile::tempdir().expect("codex home");
    let sessions = codex_home.path().join("sessions");
    fs::create_dir_all(&sessions).expect("codex sessions");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/codex/simple_session.jsonl"),
        sessions.join("simple_session.jsonl"),
    )
    .expect("copy codex fixture");

    let database = tempfile::tempdir().expect("database directory");
    let config = CoreConfig::new(
        database.path().join("tokenbuddy.sqlite3"),
        Some(codex_home.path().to_owned()),
    )
    .with_dsh_home(Some(std::path::PathBuf::from("Z:/does/not/exist/dsh-home")));
    let core = Core::start(config).expect("core starts");

    // Codex events still import; the DSH source just reports `not_found`.
    let events = core.list_usage_events(None, 100, 0).expect("events");
    assert!(
        events
            .events
            .iter()
            .all(|event| event.app == AppKind::Codex)
    );
    core.shutdown().expect("core stops");
}
