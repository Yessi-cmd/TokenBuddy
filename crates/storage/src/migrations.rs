use rusqlite::Connection;

use crate::{Result, StorageError};

const MIGRATIONS: &[(i64, &str, &str)] = &[
    (
        1,
        "0001_initial",
        include_str!("../migrations/0001_initial.sql"),
    ),
    (
        2,
        "0002_app_settings",
        include_str!("../migrations/0002_app_settings.sql"),
    ),
    (
        3,
        "0003_cursor_session_identity",
        include_str!("../migrations/0003_cursor_session_identity.sql"),
    ),
    (
        4,
        "0004_session_provider_attributions",
        include_str!("../migrations/0004_session_provider_attributions.sql"),
    ),
    (
        5,
        "0005_local_fingerprint_salt",
        include_str!("../migrations/0005_local_fingerprint_salt.sql"),
    ),
    (
        6,
        "0006_account_activity_windows",
        include_str!("../migrations/0006_account_activity_windows.sql"),
    ),
    (
        7,
        "0007_model_cursor_and_reimport",
        include_str!("../migrations/0007_model_cursor_and_reimport.sql"),
    ),
    (
        8,
        "0008_model_context_backfill",
        include_str!("../migrations/0008_model_context_backfill.sql"),
    ),
    (
        9,
        "0009_opencode_db_path",
        include_str!("../migrations/0009_opencode_db_path.sql"),
    ),
    (
        10,
        "0010_claude_streamed_usage_reimport",
        include_str!("../migrations/0010_claude_streamed_usage_reimport.sql"),
    ),
    (
        11,
        "0011_dsh_home",
        include_str!("../migrations/0011_dsh_home.sql"),
    ),
];

pub fn run(conn: &mut Connection) -> Result<()> {
    let current_version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

    for (version, name, sql) in MIGRATIONS {
        if *version <= current_version {
            continue;
        }

        let transaction = conn.transaction()?;
        transaction.execute_batch(sql)?;
        transaction.execute(
            "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
            rusqlite::params![version, name],
        )?;
        transaction.pragma_update(None, "user_version", version)?;
        transaction.commit()?;
    }

    let final_version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if final_version != MIGRATIONS.last().map(|migration| migration.0).unwrap_or(0) {
        return Err(StorageError::MigrationVersion(final_version));
    }

    Ok(())
}
