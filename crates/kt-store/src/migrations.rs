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
    // 2: devices, invites, and the access log that makes sharing legible.
    r#"
    CREATE TABLE devices (
        id          TEXT PRIMARY KEY,
        name        TEXT NOT NULL,
        status      TEXT NOT NULL,
        user_agent  TEXT NOT NULL DEFAULT '',
        fingerprint TEXT NOT NULL DEFAULT '',
        first_seen  INTEGER NOT NULL,
        last_seen   INTEGER NOT NULL
    );

    CREATE TABLE invites (
        token          TEXT PRIMARY KEY,
        app_slug       TEXT NOT NULL,
        label          TEXT NOT NULL DEFAULT '',
        expires_at     INTEGER,
        pin_to_first   INTEGER NOT NULL DEFAULT 1,
        auto_approve   INTEGER NOT NULL DEFAULT 0,
        pinned_device  TEXT REFERENCES devices(id),
        redemptions    INTEGER NOT NULL DEFAULT 0,
        created_at     INTEGER NOT NULL,
        revoked_at     INTEGER
    );
    CREATE INDEX invites_app_idx ON invites(app_slug);

    -- Append-only. Every open, pairing decision and deploy, so the owner can
    -- always answer "who has seen this".
    CREATE TABLE access_log (
        id        INTEGER PRIMARY KEY AUTOINCREMENT,
        at        INTEGER NOT NULL,
        app_slug  TEXT,
        device_id TEXT,
        actor     TEXT NOT NULL,
        action    TEXT NOT NULL,
        detail    TEXT
    );
    CREATE INDEX access_log_at_idx ON access_log(at DESC);
    CREATE INDEX access_log_app_idx ON access_log(app_slug, at DESC);
    "#,
    // 3: where a device's name came from, so the owner's own wording is never
    // overwritten by a later network lookup, and the pairing prompt can say
    // whether the name is the device's or our guess.
    r#"
    ALTER TABLE devices ADD COLUMN named_by TEXT NOT NULL DEFAULT 'guess';
    "#,
    // 4: the Danger zone. An app can be taken offline without being deleted,
    // and it can be forgotten without its folder being touched.
    r#"
    ALTER TABLE apps ADD COLUMN paused INTEGER NOT NULL DEFAULT 0;

    -- Folders the owner told Kitchen Table to forget.
    --
    -- Keyed by path rather than slug because the slug is derived from the
    -- folder and a forgotten app has no record left to derive it from. Kept
    -- here rather than in the folder's own app.json because "Kitchen Table
    -- forgets it" is a fact about Kitchen Table, and the promise made in the
    -- window is that the folder on disk is left alone.
    CREATE TABLE forgotten (
        path        TEXT PRIMARY KEY,
        forgotten_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
    );
    "#,
    // 5: whether an app is published to the relay, and how.
    r#"
    -- A column rather than a key inside `extra`, for the same reason `paused`
    -- got one: it is queried, it is a fixed vocabulary, and burying it in a
    -- JSON blob would make "which apps are published" a full scan and a parse.
    --
    -- Defaults to 'off', which is what every existing app is. Appearing in the
    -- workspace has never put anything on the internet, and this migration
    -- must not be the moment that changes.
    ALTER TABLE apps ADD COLUMN relay TEXT NOT NULL DEFAULT 'off';
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
        // Build a database that genuinely stopped partway: run only the first
        // migration by hand, then let `apply` finish the job. Rerunning an
        // already-applied CREATE TABLE fails, so reaching LATEST proves the
        // skip works rather than the statements happening to be idempotent.
        let conn = Connection::open_in_memory().expect("opens");
        let first = MIGRATIONS[0];
        conn.execute_batch(&format!("BEGIN; {first} PRAGMA user_version = 1; COMMIT;"))
            .expect("applies the first migration by hand");
        assert_eq!(current_version(&conn).expect("reads"), 1);

        apply(&conn).expect("catches up");
        assert_eq!(current_version(&conn).expect("reads"), LATEST);
    }

    #[test]
    fn a_fresh_database_gets_every_migration() {
        // The upgrade path new users take, and the one that must never break.
        let conn = Connection::open_in_memory().expect("opens");
        apply(&conn).expect("applies");
        assert_eq!(current_version(&conn).expect("reads"), LATEST);
        assert_eq!(
            current_version(&conn).expect("reads") as usize,
            MIGRATIONS.len(),
            "every migration should have run"
        );
    }
}
