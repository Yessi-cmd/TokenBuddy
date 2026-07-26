//! The official Codex account reaches every surface through the Core: the
//! account identity from `auth.json`, the official quota windows from the
//! rollout log, and the tray's `QuickSummary` — without a second scan and
//! without a repeated import changing anything.

use std::{fs, path::Path};

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

#[test]
fn codex_official_account_and_quota_reach_the_tray_and_stay_idempotent() {
    let home = fixture_home();
    let database = tempfile::tempdir().expect("database directory");
    let core = Core::start(CoreConfig::new(
        database.path().join("tokenbuddy.sqlite3"),
        Some(home.path().to_owned()),
    ))
    .expect("core starts");

    let accounts = core.list_accounts().expect("accounts");
    let official = accounts
        .iter()
        .find(|summary| summary.account.auth_mode == "chatgpt")
        .expect("official Codex account");
    assert_eq!(official.account.plan.as_deref(), Some("pro"));
    assert_eq!(official.provider_name.as_deref(), Some("OpenAI"));
    // The fingerprint is salted per install, so it is not the raw account id
    // that sits in auth.json.
    assert_ne!(official.account.account_fingerprint, "acct-fixture-0001");

    let quotas = core.list_quota_snapshots(None, 100).expect("quota rows");
    assert_eq!(quotas.len(), 3, "two windows, then the changed primary");
    assert_eq!(quotas[0].used_percent, Some(18.75));
    assert_eq!(quotas[0].precision, PrecisionLevel::Correlated);

    let summary = core.quick_summary().expect("quick summary");
    let quota_summary = summary.quota_summary.expect("tray quota window");
    assert_eq!(quota_summary.window_type, "primary_5h");
    assert_eq!(quota_summary.used_percent, Some(18.75));
    assert_eq!(quota_summary.remaining_percent, Some(81.25));
    // Quota percentages never become token counts (spec §8.4): the three quota
    // rows added no usage events, and the tokens still come only from the three
    // `token_count` rows of the session log.
    assert_eq!(
        core.list_usage_events(None, 100, 0)
            .expect("usage events")
            .total,
        3
    );

    let report = core.rescan_codex(None).expect("rescan");
    assert_eq!(report.inserted_events, 0);
    assert_eq!(report.inserted_quota_snapshots, 0);
    assert_eq!(
        core.list_quota_snapshots(None, 100)
            .expect("quota rows")
            .len(),
        3
    );

    core.shutdown().expect("core stops");
}
