//! The Core's configuration and query surface.
//!
//! Every panel reaches the data through these methods, so their contract is
//! what the product actually guarantees: a setting change takes effect without
//! a restart, an unconfigured source degrades instead of failing, and a
//! launcher database contributes attribution without contributing tokens.

use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use rusqlite::Connection;
use tokenbuddy_core::{Core, CoreConfig};
use tokenbuddy_domain::{AppSettings, UsageFilters};

fn codex_home_with(fixture: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("codex home");
    let sessions = home.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions directory");
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/codex")
        .join(fixture);
    fs::copy(source, sessions.join(fixture)).expect("copy fixture");
    home
}

fn start_core(codex_home: Option<&Path>) -> (Arc<Core>, tempfile::TempDir) {
    let database = tempfile::tempdir().expect("database directory");
    let mut config = CoreConfig::new(
        database.path().join("tokenbuddy.sqlite3"),
        codex_home.map(Path::to_path_buf),
    );
    config.poll_interval = Duration::from_millis(50);
    config.enable_file_watcher = false;
    (Core::start(config).expect("core starts"), database)
}

/// A Cockpit request log with one account, mirroring the real schema.
fn cockpit_database() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("cockpit dir");
    let connection =
        Connection::open(dir.path().join("codex_local_access_logs.sqlite")).expect("create");
    connection
        .execute_batch(
            "CREATE TABLE request_logs (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 event_key TEXT NOT NULL UNIQUE,
                 timestamp INTEGER NOT NULL,
                 account_id TEXT NOT NULL DEFAULT '',
                 email TEXT NOT NULL DEFAULT '',
                 gateway_mode TEXT NOT NULL DEFAULT ''
             );
             INSERT INTO request_logs (event_key, timestamp, account_id, email, gateway_mode)
                 VALUES ('evt-1', 1785000000, 'codex_plus', 'plus@example.com', 'proxy');",
        )
        .expect("seed");
    dir
}

/// A CC-Switch database with one provider and one proxied request.
fn cc_switch_database() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("cc-switch dir");
    let connection = Connection::open(dir.path().join("cc-switch.db")).expect("create");
    connection
        .execute_batch(
            "CREATE TABLE providers (
                 id TEXT NOT NULL, app_type TEXT NOT NULL, name TEXT NOT NULL,
                 website_url TEXT, provider_type TEXT, PRIMARY KEY (id, app_type));
             CREATE TABLE provider_endpoints (
                 provider_id TEXT NOT NULL, app_type TEXT NOT NULL, url TEXT NOT NULL);
             CREATE TABLE proxy_request_logs (
                 request_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, app_type TEXT NOT NULL,
                 session_id TEXT, created_at INTEGER NOT NULL, data_source TEXT NOT NULL,
                 total_cost_usd TEXT);
             INSERT INTO providers (id, app_type, name, website_url, provider_type)
                 VALUES ('relay-1', 'codex', 'Relay One', 'https://relay.example', 'openai');
             INSERT INTO provider_endpoints (provider_id, app_type, url)
                 VALUES ('relay-1', 'codex', 'https://relay.example/v1');
             INSERT INTO proxy_request_logs
                 (request_id, provider_id, app_type, session_id, created_at, data_source, total_cost_usd)
                 VALUES ('req-1', 'relay-1', 'codex', 'simple-session', 1785000000, 'proxy', '0.01');",
        )
        .expect("seed");
    dir
}

#[test]
fn configuration_changes_take_effect_without_restarting_the_core() {
    let home = codex_home_with("simple_session.jsonl");
    let (core, _database) = start_core(None);

    // Nothing configured: the Core reports that rather than failing.
    assert_eq!(core.codex_home().expect("codex home"), None);
    assert!(!core.detect_codex_path().expect("detection").detected);
    assert_eq!(
        core.list_usage_events(None, 10, 0).expect("events").total,
        0
    );

    // Pointing at a home imports it immediately.
    let report = core
        .rescan_codex(Some(home.path().to_owned()))
        .expect("rescan with a new home");
    assert_eq!(report.inserted_events, 2);
    assert_eq!(
        core.codex_home().expect("home").as_deref(),
        Some(home.path())
    );
    assert!(core.detect_codex_path().expect("detection").detected);

    // The path is persisted, so a later read sees it too.
    let settings = core.get_app_settings().expect("settings");
    assert_eq!(
        settings.codex_home.as_deref(),
        Some(home.path().to_string_lossy().as_ref())
    );

    core.shutdown().expect("core stops");
}

#[test]
fn every_source_path_can_be_set_and_read_back() {
    let (core, _database) = start_core(None);
    let cc_switch = cc_switch_database();
    let cockpit = cockpit_database();
    let cc_switch_path = cc_switch.path().join("cc-switch.db");
    let cockpit_path = cockpit.path().join("codex_local_access_logs.sqlite");

    core.set_claude_home(Some(cockpit.path().to_owned()))
        .expect("set claude home");
    core.set_cc_switch_db(Some(cc_switch_path.clone()))
        .expect("set cc-switch");
    core.set_cockpit_db(Some(cockpit_path.clone()))
        .expect("set cockpit");

    assert_eq!(
        core.claude_home().expect("claude home").as_deref(),
        Some(cockpit.path())
    );
    assert_eq!(
        core.cc_switch_db().expect("cc-switch").as_deref(),
        Some(cc_switch_path.as_path())
    );
    assert_eq!(
        core.cockpit_db().expect("cockpit").as_deref(),
        Some(cockpit_path.as_path())
    );

    // Detection now reaches real databases.
    assert!(core.detect_cc_switch_path().expect("detect").detected);
    assert!(core.detect_cockpit_path().expect("detect").detected);
    // The Claude home points at a directory with no `projects/`, so detection
    // reports not-found rather than claiming success.
    assert!(!core.detect_claude_path().expect("detect").detected);

    // Clearing a path returns the source to unconfigured rather than erroring.
    core.set_cc_switch_db(None).expect("clear cc-switch");
    assert_eq!(core.cc_switch_db().expect("cc-switch"), None);
    assert!(!core.detect_cc_switch_path().expect("detect").detected);

    core.shutdown().expect("core stops");
}

#[test]
fn updating_settings_reconfigures_every_source_at_once() {
    let home = codex_home_with("simple_session.jsonl");
    let cockpit = cockpit_database();
    let (core, _database) = start_core(None);

    core.update_app_settings(AppSettings {
        codex_home: Some(home.path().to_string_lossy().into_owned()),
        cockpit_path: Some(
            cockpit
                .path()
                .join("codex_local_access_logs.sqlite")
                .to_string_lossy()
                .into_owned(),
        ),
        data_retention_days: Some(365),
        otel_port: Some(4318),
        ..AppSettings::default()
    })
    .expect("update settings");

    assert_eq!(
        core.codex_home().expect("home").as_deref(),
        Some(home.path())
    );
    assert!(core.cockpit_db().expect("cockpit").is_some());
    let stored = core.get_app_settings().expect("settings");
    assert_eq!(stored.data_retention_days, Some(365));
    assert_eq!(stored.otel_port, Some(4318));
    // Sources the settings left unset are cleared, not remembered.
    assert_eq!(core.cc_switch_db().expect("cc-switch"), None);

    core.shutdown().expect("core stops");
}

#[test]
fn a_launcher_database_adds_attribution_without_adding_tokens() {
    let home = codex_home_with("simple_session.jsonl");
    let cc_switch = cc_switch_database();
    let cockpit = cockpit_database();
    let database = tempfile::tempdir().expect("database directory");
    let mut config = CoreConfig::new(
        database.path().join("tokenbuddy.sqlite3"),
        Some(home.path().to_owned()),
    )
    .with_cc_switch_db(Some(cc_switch.path().join("cc-switch.db")))
    .with_cockpit_db(Some(cockpit.path().join("codex_local_access_logs.sqlite")));
    config.enable_file_watcher = false;
    let core = Core::start(config).expect("core starts");

    // The token count is the session log's alone: the launchers proxied these
    // same requests, and counting their rows would double them.
    assert_eq!(
        core.list_usage_events(None, 100, 0).expect("events").total,
        2
    );

    // CC-Switch supplies the real provider for the session it proxied.
    let providers = core.list_providers().expect("providers");
    assert!(
        providers
            .iter()
            .any(|provider| provider.display_name.contains("Relay One")),
        "cc-switch provider missing: {:?}",
        providers
            .iter()
            .map(|provider| provider.display_name.clone())
            .collect::<Vec<_>>()
    );

    // Cockpit supplies the account behind the Codex requests.
    let accounts = core.list_accounts().expect("accounts");
    assert!(
        accounts
            .iter()
            .any(|summary| summary.account.auth_mode == "cockpit"),
        "cockpit account missing"
    );

    let report = core.rescan_cockpit(None).expect("rescan cockpit");
    assert_eq!(report.inserted_events, 0);
    let report = core.rescan_cc_switch(None).expect("rescan cc-switch");
    assert_eq!(report.inserted_events, 0);

    core.shutdown().expect("core stops");
}

#[test]
fn queries_share_one_filter_contract() {
    let home = codex_home_with("simple_session.jsonl");
    let (core, _database) = start_core(Some(home.path()));

    let start = "2026-07-25T00:00:00Z".parse().expect("start");
    let end = "2026-07-26T00:00:00Z".parse().expect("end");
    let dashboard = core.dashboard_summary(start, end).expect("dashboard");
    assert_eq!(dashboard.period_start, start);
    assert_eq!(dashboard.period_end, end);
    assert_eq!(dashboard.totals.event_count, 2);

    // The same window through the filtered entry points agrees.
    let filters = UsageFilters {
        period_start: Some(start),
        period_end: Some(end),
        ..UsageFilters::default()
    };
    assert_eq!(
        core.dashboard_summary_filtered(filters.clone())
            .expect("filtered dashboard")
            .totals
            .event_count,
        2
    );
    assert_eq!(
        core.list_usage_events_filtered(None, 100, 0, &filters)
            .expect("filtered events")
            .total,
        2
    );
    assert_eq!(
        core.model_breakdown(filters.clone())
            .expect("breakdown")
            .len(),
        1
    );

    // A window with no data reports zero events rather than failing.
    let empty = UsageFilters {
        period_start: Some("2020-01-01T00:00:00Z".parse().expect("start")),
        period_end: Some("2020-01-02T00:00:00Z".parse().expect("end")),
        ..UsageFilters::default()
    };
    assert_eq!(
        core.list_usage_events_filtered(None, 100, 0, &empty)
            .expect("filtered events")
            .total,
        0
    );

    // Today's window is computed from the local calendar day.
    let today = core.today_dashboard_summary().expect("today");
    assert!(today.period_start < today.period_end);

    core.shutdown().expect("core stops");
}

#[test]
fn summary_listeners_are_notified_when_the_summary_changes() {
    let home = codex_home_with("simple_session.jsonl");
    let (core, _database) = start_core(None);

    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    core.add_summary_listener(move |summary| {
        recorder
            .lock()
            .expect("listener lock")
            .push(summary.collection_status);
    })
    .expect("register listener");

    // Importing a home changes the summary, so the tray is told without
    // polling for it.
    core.rescan_codex(Some(home.path().to_owned()))
        .expect("rescan");

    let statuses = seen.lock().expect("listener lock").clone();
    assert!(
        !statuses.is_empty(),
        "a summary change must reach registered listeners"
    );

    core.shutdown().expect("core stops");
}
