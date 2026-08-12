//! Forward-only schema migrations, tracked in SQLite's `user_version`.
//!
//! Add a statement to [`MIGRATIONS`] and never edit one that has shipped: a
//! user's database has already run it. Deployed daemons upgrade in place on
//! next start, so a migration that fails halfway leaves the database at the
//! last completed version rather than somewhere in between.

use rusqlite::Connection;

/// Each entry is applied in a transaction, in order, once.
const MIGRATIONS: &[&str] = &[
    // 1: apps discovered in the workspace.
    r#"
    CREATE TABLE apps (
        slug        TEXT PRIMARY KEY,
        name        TEXT NOT NULL,
        icon        TEXT,
        entry       TEXT NOT NULL DEFAULT 'index.html',
        visibility  TEXT NOT NULL DEFAULT 'private',
        version     INTEGER NOT NULL DEFAULT 0,
        path        TEXT NOT NULL,
        extra       TEXT NOT NULL DEFAULT '{}',
        created_at  INTEGER NOT NULL DEFAULT (strftime('%s','now')),
        updated_at  INTEGER NOT NULL DEFAULT (strftime('%s','now'))
    );
    CREATE UNIQUE INDEX apps_path_idx ON apps(path);
    "#,
];

/// The version a fresh database ends up at. Read by callers checking for a
/// mid-upgrade database, and by the tests below.
#[allow(dead_code)]
pub const LATEST: u32 = MIGRATIONS.len() as u32;

pub fn current_version(conn: &Connection) -> Result<u32, rusqlite::Error> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
}

pub fn apply(conn: &Connection) -> Result<(), rusqlite::Error> {
    let from = current_version(conn)?;

    for (i, sql) in MIGRATIONS.iter().enumerate().skip(from as usize) {
        let to = i as u32 + 1;
        conn.execute_batch(&format!("BEGIN; {sql} PRAGMA user_version = {to}; COMMIT;"))?;
        tracing::info!(version = to, "applied migration");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applying_twice_is_a_no_op() {
        let conn = Connection::open_in_memory().expect("opens");
        apply(&conn).expect("first");
        apply(&conn).expect("second");
        assert_eq!(current_version(&conn).expect("reads"), LATEST);
    }

    #[test]
    fn an_old_database_catches_up_without_rerunning_what_it_has() {
        let conn = Connection::open_in_memory().expect("opens");
        apply(&conn).expect("applies");
        // Rerunning migration 1 would fail on CREATE TABLE; reaching LATEST
        // proves the skip works rather than the statements being idempotent.
        conn.execute_batch("PRAGMA user_version = 1;")
            .expect("sets");
        apply(&conn).expect("catches up");
        assert_eq!(current_version(&conn).expect("reads"), LATEST);
    }
}
