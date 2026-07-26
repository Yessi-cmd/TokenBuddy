//! Mechanics for reading a *foreign* SQLite database safely.
//!
//! TokenBuddy reads databases owned by other applications (CC-Switch, Cockpit
//! Tools). Those files may be any version, may be missing tables or columns
//! entirely, and must never be written to (AGENTS.md: third-party integrations
//! are read-only). This crate holds the part of that job that is identical for
//! every such source — open read-only, probe before reading, index columns by
//! name, tolerate a missing column — so each adapter can spend its own code on
//! what only it knows: which tables and columns mean what.
//!
//! Deliberately *not* here: table names, column names, and any interpretation of
//! their values. Keeping those in the adapters is what lets one third-party
//! schema change without touching another adapter.
#![warn(missing_docs)]

use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, OpenFlags, Row, Statement};

/// Open a foreign database read-only.
///
/// The read-only flag is the guarantee that TokenBuddy cannot corrupt another
/// application's data even if a query is wrong; it also means opening never
/// creates a file that was not there.
pub fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
}

/// Whether `table` exists, so a caller can skip it instead of erroring out when
/// the other application's schema does not have it (yet, or any more).
pub fn table_exists(connection: &Connection, table: &str) -> rusqlite::Result<bool> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// The column names of `table`, for checking that the fields an adapter depends
/// on are actually present before it reads a single row.
///
/// `table` is interpolated into a `PRAGMA`, which takes no parameters. Callers
/// pass their own compile-time table names, never user input.
pub fn column_set(connection: &Connection, table: &str) -> rusqlite::Result<HashSet<String>> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    let mut columns = HashSet::new();
    for name in rows {
        columns.insert(name?);
    }
    Ok(columns)
}

/// Map a prepared statement's column names to their positions.
///
/// Adapters select `*` so that a schema which gained or reordered columns still
/// works; this is how they then reach a column by name.
pub fn column_names(statement: &Statement<'_>) -> HashMap<String, usize> {
    statement
        .column_names()
        .into_iter()
        .enumerate()
        .map(|(index, name)| (name.to_owned(), index))
        .collect()
}

/// A text column, or `None` if the column is absent, NULL, or not text.
///
/// A column TokenBuddy cannot read stays unknown; it never becomes an empty
/// string that later reads as a real (blank) value.
pub fn string_col(row: &Row<'_>, names: &HashMap<String, usize>, name: &str) -> Option<String> {
    let index = *names.get(name)?;
    row.get::<_, Option<String>>(index).ok().flatten()
}

/// An integer column, or `None` if the column is absent, NULL, or not an integer.
pub fn int_col(row: &Row<'_>, names: &HashMap<String, usize>, name: &str) -> Option<i64> {
    let index = *names.get(name)?;
    row.get::<_, Option<i64>>(index).ok().flatten()
}

/// A floating-point column, or `None` if the column is absent, NULL, or not a
/// number.
pub fn float_col(row: &Row<'_>, names: &HashMap<String, usize>, name: &str) -> Option<f64> {
    let index = *names.get(name)?;
    row.get::<_, Option<f64>>(index).ok().flatten()
}

/// Interpret an epoch timestamp that may be in seconds or milliseconds.
///
/// Third-party tools disagree on the unit and sometimes change it between
/// versions. Values at or below zero are rejected rather than mapped to the
/// epoch, because "0" in these logs means "not recorded", and an event dated
/// 1970 would silently distort every window the UI computes.
pub fn epoch_to_utc(value: i64) -> Option<DateTime<Utc>> {
    if value <= 0 {
        return None;
    }
    // ~1973 in milliseconds, ~5138 in seconds: past this point a value can only
    // sensibly be milliseconds.
    if value > 100_000_000_000 {
        Utc.timestamp_millis_opt(value).single()
    } else {
        Utc.timestamp_opt(value, 0).single()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        column_names, column_set, epoch_to_utc, float_col, int_col, open_read_only, string_col,
        table_exists,
    };
    use rusqlite::Connection;

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("foreign.sqlite");
        let connection = Connection::open(&path).expect("create");
        connection
            .execute_batch(
                "CREATE TABLE rows_of_interest (
                     id INTEGER PRIMARY KEY,
                     label TEXT,
                     amount REAL,
                     recorded_at INTEGER
                 );
                 INSERT INTO rows_of_interest (label, amount, recorded_at)
                     VALUES ('first', 1.5, 1785000000), (NULL, NULL, NULL);",
            )
            .expect("seed");
        (dir, path)
    }

    #[test]
    fn a_foreign_database_opens_read_only() {
        let (_dir, path) = fixture();
        let connection = open_read_only(&path).expect("open");

        assert!(table_exists(&connection, "rows_of_interest").expect("probe"));
        assert!(!table_exists(&connection, "not_there").expect("probe"));

        // Read-only is the point: a write must be refused, not silently applied.
        assert!(
            connection
                .execute("DELETE FROM rows_of_interest", [])
                .is_err()
        );
    }

    #[test]
    fn opening_a_missing_file_fails_instead_of_creating_one() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("absent.sqlite");
        assert!(open_read_only(&path).is_err());
        assert!(!path.exists(), "read-only open must not create the file");
    }

    #[test]
    fn columns_are_reachable_by_name_and_missing_values_stay_missing() {
        let (_dir, path) = fixture();
        let connection = open_read_only(&path).expect("open");

        let columns = column_set(&connection, "rows_of_interest").expect("columns");
        assert!(columns.contains("label"));
        assert!(columns.contains("recorded_at"));
        assert!(!columns.contains("nonexistent"));
        // A table that is not there has no columns rather than erroring.
        assert!(
            column_set(&connection, "not_there")
                .expect("probe")
                .is_empty()
        );

        let mut statement = connection
            .prepare("SELECT * FROM rows_of_interest ORDER BY id")
            .expect("prepare");
        let names = column_names(&statement);
        let mut rows = statement.query([]).expect("query");

        let first = rows.next().expect("row").expect("present");
        assert_eq!(string_col(first, &names, "label"), Some("first".to_owned()));
        assert_eq!(float_col(first, &names, "amount"), Some(1.5));
        assert_eq!(int_col(first, &names, "recorded_at"), Some(1_785_000_000));
        // A column the schema does not have is unknown, not a default.
        assert_eq!(string_col(first, &names, "absent_column"), None);
        assert_eq!(int_col(first, &names, "absent_column"), None);
        assert_eq!(float_col(first, &names, "absent_column"), None);
        // A text accessor over a numeric column does not invent a string.
        assert_eq!(int_col(first, &names, "label"), None);

        let second = rows.next().expect("row").expect("present");
        assert_eq!(string_col(second, &names, "label"), None);
        assert_eq!(float_col(second, &names, "amount"), None);
        assert_eq!(int_col(second, &names, "recorded_at"), None);
    }

    #[test]
    fn epoch_values_are_read_in_either_unit_and_never_as_the_epoch_itself() {
        let seconds = epoch_to_utc(1_785_000_000).expect("seconds");
        let millis = epoch_to_utc(1_785_000_000_000).expect("milliseconds");
        assert_eq!(seconds, millis, "both units must name the same instant");
        assert_eq!(seconds.to_rfc3339(), "2026-07-25T17:20:00+00:00");

        // "Not recorded" must not become 1970-01-01.
        assert_eq!(epoch_to_utc(0), None);
        assert_eq!(epoch_to_utc(-1), None);
    }
}
