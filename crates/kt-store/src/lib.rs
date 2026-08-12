//! SQLite storage: `system.db` now, per-app stores in D5.
//!
//! One connection behind a mutex. Kitchen Table is a single-user daemon serving
//! a household, so contention is not the problem to solve; a pool would be
//! ceremony. SQLite runs in WAL mode so a slow read never blocks serving.

use std::path::Path;
use std::sync::Mutex;

use kt_types::{AppManifest, AppRecord, Visibility};
use rusqlite::{params, Connection, OptionalExtension};

mod migrations;

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
    #[error("could not create {path}: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
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
            "INSERT INTO apps (slug, name, icon, entry, visibility, version, path, extra)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(slug) DO UPDATE SET
                name = excluded.name,
                icon = excluded.icon,
                entry = excluded.entry,
                visibility = excluded.visibility,
                version = excluded.version,
                path = excluded.path,
                extra = excluded.extra,
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
            ],
        )?;
        Ok(())
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
            "SELECT slug, name, icon, entry, visibility, version, path, extra
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
            "SELECT slug, name, icon, entry, visibility, version, path, extra
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

    /// A poisoned mutex means a previous caller panicked mid-statement; the
    /// connection itself is still sound, so recovering beats taking the process
    /// down.
    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
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

    Ok(AppRecord {
        manifest: AppManifest {
            name: row.get(1)?,
            slug,
            icon: row.get(2)?,
            entry: row.get(3)?,
            visibility: visibility_from_str(&row.get::<_, String>(4)?),
            version: row.get(5)?,
            extra,
        },
        path: row.get(6)?,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn record(slug: &str, name: &str) -> AppRecord {
        AppRecord {
            manifest: AppManifest {
                name: name.into(),
                slug: slug.into(),
                icon: None,
                entry: "index.html".into(),
                visibility: Visibility::Private,
                version: 1,
                extra: serde_json::Map::new(),
            },
            path: format!("/tmp/ws/{slug}"),
        }
    }

    #[test]
    fn a_fresh_database_is_at_the_latest_migration() {
        let store = Store::in_memory().expect("opens");
        assert_eq!(store.schema_version().expect("reads"), migrations::LATEST);
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
