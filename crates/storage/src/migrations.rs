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
