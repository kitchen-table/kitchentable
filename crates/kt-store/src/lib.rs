//! SQLite storage: `system.db` now, per-app stores in D5.
//!
//! One connection behind a mutex. Kitchen Table is a single-user daemon serving
//! a household, so contention is not the problem to solve; a pool would be
//! ceremony. SQLite runs in WAL mode so a slow read never blocks serving.

use std::path::Path;
use std::sync::Mutex;

use kt_types::{AppManifest, AppRecord, RelayMode, StorageMode, Visibility};
use rusqlite::{params, Connection, OptionalExtension};

mod app_store;
mod migrations;
mod trust;

pub use app_store::{AppStore, Entry, Scope, Storage, DEFAULT_QUOTA_BYTES};
pub use trust::AccessEvent;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored manifest for {slug:?} is not valid JSON: {source}")]
    CorruptManifest {
        slug: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("stored {kind} {value:?} is not the shape we mint")]
    CorruptIdentifier { kind: &'static str, value: String },
    #[error("could not create {path}: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("this app has used its {limit} bytes of storage")]
    Quota { limit: i64 },
    /// A slug that would not be a filename. The storage API resolves an app
    /// from the `Host` header, so this is reachable from outside.
    #[error("{0:?} is not a slug this daemon would have minted")]
    BadSlug(String),
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (creating if needed) the system database, applying any pending
    /// migrations.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| StoreError::CreateDir {
                path: parent.display().to_string(),
                source,
            })?;
        }
        Self::from_connection(Connection::open(path)?)
    }

    /// An ephemeral database, for tests.
    pub fn in_memory() -> Result<Self, StoreError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrations::apply(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// The migration level this database is at.
    pub fn schema_version(&self) -> Result<u32, StoreError> {
        let conn = self.lock();
        Ok(migrations::current_version(&conn)?)
    }

    /// Insert or update by slug. Idempotent, so a rescan of an unchanged
    /// workspace is a no-op.
    pub fn upsert_app(&self, record: &AppRecord) -> Result<(), StoreError> {
        let m = &record.manifest;
        let extra = serde_json::Value::Object(m.extra.clone()).to_string();
        self.lock().execute(
            "INSERT INTO apps (slug, name, icon, entry, visibility, version, path, extra, paused, relay, public_label, storage, storage_backup)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(slug) DO UPDATE SET
                name = excluded.name,
                icon = excluded.icon,
                entry = excluded.entry,
                visibility = excluded.visibility,
                version = excluded.version,
                path = excluded.path,
                extra = excluded.extra,
                paused = excluded.paused,
                relay = excluded.relay,
                public_label = excluded.public_label,
                storage = excluded.storage,
                storage_backup = excluded.storage_backup,
                updated_at = strftime('%s','now')",
            params![
                m.slug,
                m.name,
                m.icon,
                m.entry,
                visibility_str(m.visibility),
                m.version,
                record.path,
                extra,
                m.paused as i64,
                relay_str(m.relay),
                m.public_label,
                storage_str(m.storage),
                m.storage_backup as i64,
            ],
        )?;
        Ok(())
    }

    /// Stop treating a folder as an app, without touching the folder.
    ///
    /// Recorded here rather than in the folder because the promise the window
    /// makes is that your files are left alone - and because a scan has to be
    /// able to skip it before anything in it has been read.
    pub fn forget_path(&self, path: &str) -> Result<(), StoreError> {
        self.lock().execute(
            "INSERT INTO forgotten (path) VALUES (?1) ON CONFLICT(path) DO NOTHING",
            params![path],
        )?;
        Ok(())
    }

    /// Take a folder off the forgotten list, so the next scan picks it up.
    pub fn remember_path(&self, path: &str) -> Result<bool, StoreError> {
        let n = self
            .lock()
            .execute("DELETE FROM forgotten WHERE path = ?1", params![path])?;
        Ok(n > 0)
    }

    /// Every folder the owner told Kitchen Table to forget.
    pub fn forgotten_paths(&self) -> Result<Vec<String>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT path FROM forgotten")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Returns whether a row was actually removed.
    pub fn remove_app(&self, slug: &str) -> Result<bool, StoreError> {
        let n = self
            .lock()
            .execute("DELETE FROM apps WHERE slug = ?1", params![slug])?;
        Ok(n > 0)
    }

    /// Every app, name-ordered so the library is stable between calls.
    pub fn list_apps(&self) -> Result<Vec<AppRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT slug, name, icon, entry, visibility, version, path, extra, paused, relay, public_label, storage, storage_backup
             FROM apps ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| Ok(row_to_record(row)))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    pub fn get_app(&self, slug: &str) -> Result<Option<AppRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT slug, name, icon, entry, visibility, version, path, extra, paused, relay, public_label, storage, storage_backup
             FROM apps WHERE slug = ?1",
        )?;
        let found = stmt
            .query_row(params![slug], |row| Ok(row_to_record(row)))
            .optional()?;
        found.transpose()
    }

    /// Drop every app not in `keep`. Used after a workspace rescan, so folders
    /// deleted while the daemon was down do not linger in the library.
    pub fn retain_apps(&self, keep: &[String]) -> Result<Vec<String>, StoreError> {
        let existing: Vec<String> = self
            .list_apps()?
            .into_iter()
            .map(|r| r.manifest.slug)
            .collect();
        let mut removed = Vec::new();
        for slug in existing {
            if !keep.contains(&slug) {
                self.remove_app(&slug)?;
                removed.push(slug);
            }
        }
        Ok(removed)
    }

    /// A daemon-level choice the owner made, or `None` if they never made one.
    ///
    /// `None` is the answer that matters: it means "unchanged", and every
    /// caller is expected to fall back to its own default rather than treat a
    /// missing row as an error. An install that has never opened Settings has
    /// an empty table and must behave exactly as it did before the table
    /// existed.
    pub fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Record a choice, replacing whatever was there.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.lock().execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, strftime('%s','now'))
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value],
        )?;
        Ok(())
    }

    /// Forget a choice, so the default applies again.
    pub fn clear_setting(&self, key: &str) -> Result<(), StoreError> {
        self.lock()
            .execute("DELETE FROM settings WHERE key = ?1", params![key])?;
        Ok(())
    }

    /// A poisoned mutex means a previous caller panicked mid-statement; the
    /// connection itself is still sound, so recovering beats taking the process
    /// down.
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn row_to_record(row: &rusqlite::Row<'_>) -> Result<AppRecord, StoreError> {
    let slug: String = row.get(0)?;
    let extra_raw: String = row.get(7)?;
    let extra = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&extra_raw)
        .map_err(|source| StoreError::CorruptManifest {
            slug: slug.clone(),
            source,
        })?;

    // Size and deploy time are derived from the folder, not stored. The
    // registry measures them on every scan and `app.list` serves records from
    // the registry, so a stored copy would only ever be a staler answer to the
    // same question.
    Ok(AppRecord::unmeasured(
        AppManifest {
            name: row.get(1)?,
            slug,
            icon: row.get(2)?,
            entry: row.get(3)?,
            visibility: visibility_from_str(&row.get::<_, String>(4)?),
            version: row.get(5)?,
            // Column 8. Index 7 is `extra`, and reading that as an
            // integer would quietly make every app look unpaused.
            paused: row.get::<_, i64>(8)? != 0,
            // Column 10, index 9. Appended rather than inserted, deliberately:
            // every index above stays where it was, which is the trap that
            // adding `paused` set (HANDOFF section 7).
            relay: relay_from_str(&row.get::<_, String>(9)?),
            // Column 11, index 10. Appended for the same reason `relay` was.
            public_label: row.get::<_, Option<String>>(10)?,
            // Columns 12 and 13, indices 11 and 12. Appended, same reason
            // again: every index above this line stays where it was.
            storage: storage_from_str(&row.get::<_, String>(11)?),
            storage_backup: row.get::<_, i64>(12)? != 0,
            extra,
        },
        row.get(6)?,
    ))
}

fn visibility_str(v: Visibility) -> &'static str {
    match v {
        Visibility::Private => "private",
        Visibility::Network => "network",
        Visibility::Invited => "invited",
        Visibility::Public => "public",
    }
}

/// Unknown values fall back to Private. A row we cannot interpret must never
/// become *more* visible than intended.
fn visibility_from_str(s: &str) -> Visibility {
    match s {
        "network" => Visibility::Network,
        "invited" => Visibility::Invited,
        "public" => Visibility::Public,
        _ => Visibility::Private,
    }
}

fn relay_str(mode: RelayMode) -> &'static str {
    match mode {
        RelayMode::Off => "off",
        RelayMode::Standard => "standard",
        RelayMode::Strict => "strict",
    }
}

/// Unknown values fall back to `Off`, on the same principle as visibility above
/// and with more at stake: a row this build cannot interpret must never become
/// reachable from the internet because of it.
fn relay_from_str(s: &str) -> RelayMode {
    match s {
        "standard" => RelayMode::Standard,
        "strict" => RelayMode::Strict,
        _ => RelayMode::Off,
    }
}

fn storage_str(mode: StorageMode) -> &'static str {
    match mode {
        StorageMode::Synced => "synced",
        StorageMode::PerDevice => "per_device",
    }
}

/// Unknown values fall back to `Synced`, which is what every app was before
/// this column existed. The stake here is the mirror of the two above: a row
/// this build cannot interpret must not silently split one household's shared
/// list into a copy per phone.
fn storage_from_str(s: &str) -> StorageMode {
    match s {
        "per_device" => StorageMode::PerDevice,
        _ => StorageMode::Synced,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(slug: &str, name: &str) -> AppRecord {
        AppRecord::unmeasured(
            AppManifest {
                relay: RelayMode::Off,
                storage: StorageMode::Synced,
                storage_backup: true,
                public_label: None,
                name: name.into(),
                slug: slug.into(),
                icon: None,
                entry: "index.html".into(),
                visibility: Visibility::Private,
                version: 1,
                paused: false,
                extra: serde_json::Map::new(),
            },
            format!("/tmp/ws/{slug}"),
        )
    }

    #[test]
    fn a_fresh_database_is_at_the_latest_migration() {
        let store = Store::in_memory().expect("opens");
        assert_eq!(store.schema_version().expect("reads"), migrations::LATEST);
    }

    #[test]
    fn an_unset_setting_is_none_rather_than_an_error() {
        // The distinction the callers depend on: absent means "unchanged, use
        // your default", and an install that has never opened Settings has to
        // behave exactly as it did before the table existed.
        let store = Store::in_memory().expect("opens");
        assert_eq!(store.setting("workspace").expect("reads"), None);
    }

    #[test]
    fn a_setting_can_be_written_replaced_and_cleared() {
        let store = Store::in_memory().expect("opens");

        store
            .set_setting("workspace", "/Users/a/Apps")
            .expect("writes");
        assert_eq!(
            store.setting("workspace").expect("reads").as_deref(),
            Some("/Users/a/Apps")
        );

        // Replacing, not accumulating: the second choice is the choice.
        store
            .set_setting("workspace", "/Users/a/Other")
            .expect("writes");
        assert_eq!(
            store.setting("workspace").expect("reads").as_deref(),
            Some("/Users/a/Other")
        );

        store.clear_setting("workspace").expect("clears");
        assert_eq!(store.setting("workspace").expect("reads"), None);
    }

    #[test]
    fn settings_do_not_collide_with_each_other() {
        let store = Store::in_memory().expect("opens");
        store.set_setting("workspace", "/a").expect("writes");
        store.set_setting("something-else", "/b").expect("writes");
        assert_eq!(
            store.setting("workspace").expect("reads").as_deref(),
            Some("/a")
        );
    }

    #[test]
    fn upsert_is_idempotent() {
        let store = Store::in_memory().expect("opens");
        store.upsert_app(&record("trip", "Trip")).expect("inserts");
        store.upsert_app(&record("trip", "Trip")).expect("updates");
        assert_eq!(store.list_apps().expect("lists").len(), 1);
    }

    #[test]
    fn upsert_updates_rather_than_duplicating() {
        let store = Store::in_memory().expect("opens");
        store.upsert_app(&record("trip", "Trip")).expect("inserts");
        store
            .upsert_app(&record("trip", "Trip planner"))
            .expect("updates");
        let apps = store.list_apps().expect("lists");
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].manifest.name, "Trip planner");
    }

    #[test]
    fn unknown_manifest_keys_survive_the_database() {
        let store = Store::in_memory().expect("opens");
        let mut r = record("trip", "Trip");
        r.manifest
            .extra
            .insert("futureField".into(), serde_json::json!({ "a": 1 }));
        store.upsert_app(&r).expect("inserts");

        let back = store.get_app("trip").expect("reads").expect("exists");
        assert_eq!(back.manifest.extra["futureField"]["a"], 1);
        assert_eq!(back, r);
    }

    #[test]
    fn apps_come_back_ordered_by_name() {
        let store = Store::in_memory().expect("opens");
        for (slug, name) in [("z", "Alpha"), ("a", "zulu"), ("m", "Middle")] {
            store.upsert_app(&record(slug, name)).expect("inserts");
        }
        let names: Vec<_> = store
            .list_apps()
            .expect("lists")
            .into_iter()
            .map(|r| r.manifest.name)
            .collect();
        assert_eq!(names, ["Alpha", "Middle", "zulu"]);
    }

    #[test]
    fn removing_reports_whether_anything_went() {
        let store = Store::in_memory().expect("opens");
        store.upsert_app(&record("trip", "Trip")).expect("inserts");
        assert!(store.remove_app("trip").expect("removes"));
        assert!(!store.remove_app("trip").expect("no-op"));
        assert!(store.get_app("trip").expect("reads").is_none());
    }

    #[test]
    fn retain_drops_apps_that_left_the_workspace() {
        let store = Store::in_memory().expect("opens");
        store.upsert_app(&record("trip", "Trip")).expect("inserts");
        store.upsert_app(&record("gone", "Gone")).expect("inserts");

        let removed = store.retain_apps(&["trip".to_string()]).expect("retains");
        assert_eq!(removed, ["gone"]);
        assert_eq!(store.list_apps().expect("lists").len(), 1);
    }

    #[test]
    fn an_unreadable_visibility_falls_back_to_private() {
        // A row written by a future daemon must never read as more visible
        // than it is.
        assert_eq!(visibility_from_str("household"), Visibility::Private);
        assert_eq!(visibility_from_str(""), Visibility::Private);
        assert_eq!(visibility_from_str("public"), Visibility::Public);
    }
}
