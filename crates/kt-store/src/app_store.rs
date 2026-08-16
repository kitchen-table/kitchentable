//! Per-app key-value storage: one SQLite file per app.
//!
//! **Separate files rather than one table with an app column.** The product
//! promises an app can never read another app's data, and a file boundary keeps
//! that true even if somebody later writes a query that forgets the `WHERE`.
//! There is no join that reaches across, and forgetting an app is one
//! `remove_file` rather than a delete that has to get its predicate right.
//!
//! **One table does both halves of the storage story.** `scope` is either the
//! app itself - the shared checklist everyone sees - or a single device, which
//! is the private note inside a shared app. Two tables would have been two
//! copies of the same four queries, and the interesting operations (what is in
//! here, how big is it) would have had to union them back together anyway.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension};

use crate::StoreError;

/// How much one app may keep before writes start failing.
///
/// Generous for a checklist and small enough that a runaway app cannot fill a
/// laptop. Counted as the bytes of keys plus values, not the file on disk:
/// SQLite's page overhead and free pages are not something an app author can
/// reason about, so billing them for it would make the limit unpredictable.
pub const DEFAULT_QUOTA_BYTES: i64 = 50 * 1024 * 1024;

/// The schema of a per-app store. Its own sequence, in its own file, tracked by
/// its own `user_version` - nothing to do with `system.db`'s migrations.
const MIGRATIONS: &[&str] = &[
    // `WITHOUT ROWID` because this is a pure key-value table: the primary key
    // *is* the row, and the extra indirection buys nothing.
    "
    CREATE TABLE kv (
        scope      TEXT NOT NULL,
        key        TEXT NOT NULL,
        value      TEXT NOT NULL,
        updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
        PRIMARY KEY (scope, key)
    ) WITHOUT ROWID;
    ",
];

/// Whose data this is.
///
/// The device form is prefixed rather than stored bare, so the two can never
/// collide. A device id is minted at random and nothing stops one being the
/// three letters `app`; a bare encoding would hand that device the shared
/// store.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Everyone who can open the app reads and writes the same values.
    Shared,
    /// One device's own.
    Device(String),
}

impl Scope {
    /// How it is written in the `scope` column.
    pub fn as_text(&self) -> String {
        match self {
            Scope::Shared => "app".to_string(),
            Scope::Device(id) => format!("device:{id}"),
        }
    }

    /// The inverse, for reading a row back.
    pub fn parse(text: &str) -> Scope {
        match text.strip_prefix("device:") {
            Some(id) => Scope::Device(id.to_string()),
            // Anything else is the shared store. `app` is what we write; a
            // value we do not recognise is not worth inventing a scope for.
            None => Scope::Shared,
        }
    }
}

/// One entry, as the storage API and the window both want it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    /// JSON, as the app sent it. Never parsed here: this store does not care
    /// what shape an app's values are, and parsing would be a second place that
    /// could disagree with the app about them.
    pub value: String,
    pub updated_at: i64,
}

/// One app's store.
pub struct AppStore {
    conn: Mutex<Connection>,
    quota_bytes: i64,
}

impl AppStore {
    /// Open (creating if needed) the store for `slug` under `dir`.
    pub fn open(dir: &Path, slug: &str) -> Result<Self, StoreError> {
        let slug = validate_slug(slug)?;
        std::fs::create_dir_all(dir).map_err(|source| StoreError::CreateDir {
            path: dir.display().to_string(),
            source,
        })?;
        Self::from_connection(Connection::open(dir.join(format!("{slug}.db")))?)
    }

    /// An ephemeral store, for tests.
    pub fn in_memory() -> Result<Self, StoreError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        apply(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            quota_bytes: DEFAULT_QUOTA_BYTES,
        })
    }

    /// Override the default quota. For tests, and for the day this is a setting.
    pub fn with_quota(mut self, bytes: i64) -> Self {
        self.quota_bytes = bytes;
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn get(&self, scope: &Scope, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                "SELECT value FROM kv WHERE scope = ?1 AND key = ?2",
                params![scope.as_text(), key],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Write a value, refusing once the app is over its quota.
    ///
    /// The check counts what the store *would* hold afterwards, so replacing a
    /// large value with a small one always succeeds - an app that has filled its
    /// quota must never be unable to shrink itself back out of trouble.
    pub fn set(&self, scope: &Scope, key: &str, value: &str) -> Result<(), StoreError> {
        let conn = self.lock();
        let scope_text = scope.as_text();

        let total: i64 = conn.query_row(
            "SELECT COALESCE(SUM(length(key) + length(value)), 0) FROM kv",
            [],
            |row| row.get(0),
        )?;
        let replacing: i64 = conn
            .query_row(
                "SELECT length(key) + length(value) FROM kv WHERE scope = ?1 AND key = ?2",
                params![scope_text, key],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);

        let after = total - replacing + key.len() as i64 + value.len() as i64;
        if after > self.quota_bytes {
            return Err(StoreError::Quota {
                limit: self.quota_bytes,
            });
        }

        conn.execute(
            "INSERT INTO kv (scope, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(scope, key) DO UPDATE SET
                 value = excluded.value,
                 updated_at = strftime('%s','now')",
            params![scope_text, key, value],
        )?;
        Ok(())
    }

    /// Remove one key. `false` means it was not there, which is not an error:
    /// deleting something already gone is the state the caller asked for.
    pub fn delete(&self, scope: &Scope, key: &str) -> Result<bool, StoreError> {
        let removed = self.lock().execute(
            "DELETE FROM kv WHERE scope = ?1 AND key = ?2",
            params![scope.as_text(), key],
        )?;
        Ok(removed > 0)
    }

    /// Every entry in one scope, oldest key first, optionally narrowed to a
    /// prefix. Sorted by key rather than by time so a list is stable between
    /// calls - a checklist that reorders itself on every write is a bug the app
    /// author cannot fix from outside.
    pub fn list(&self, scope: &Scope, prefix: &str) -> Result<Vec<Entry>, StoreError> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT key, value, updated_at FROM kv
             WHERE scope = ?1 AND key LIKE ?2 ESCAPE '\\'
             ORDER BY key",
        )?;
        let rows = statement.query_map(
            params![scope.as_text(), format!("{}%", escape_like(prefix))],
            |row| {
                Ok(Entry {
                    key: row.get(0)?,
                    value: row.get(1)?,
                    updated_at: row.get(2)?,
                })
            },
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Bytes of keys and values, which is what the quota counts and what the
    /// window shows.
    pub fn bytes(&self) -> Result<i64, StoreError> {
        Ok(self.lock().query_row(
            "SELECT COALESCE(SUM(length(key) + length(value)), 0) FROM kv",
            [],
            |row| row.get(0),
        )?)
    }

    /// Which devices have written anything, and how many keys each holds.
    ///
    /// This is the per-viewer half of the Storage tab. The shared scope is
    /// deliberately not in the result: it is the other half of that screen and
    /// mixing them would make one list mean two things.
    pub fn device_scopes(&self) -> Result<Vec<(String, i64)>, StoreError> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT scope, COUNT(*) FROM kv
             WHERE scope LIKE 'device:%'
             GROUP BY scope ORDER BY scope",
        )?;
        let rows = statement.query_map([], |row| {
            let scope: String = row.get(0)?;
            Ok((scope, row.get::<_, i64>(1)?))
        })?;
        Ok(rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter_map(|(scope, count)| match Scope::parse(&scope) {
                Scope::Device(id) => Some((id, count)),
                Scope::Shared => None,
            })
            .collect())
    }

    /// Forget everything one device wrote. What a revoked device's data does
    /// when the owner asks for it to go.
    pub fn forget_device(&self, device: &str) -> Result<usize, StoreError> {
        Ok(self.lock().execute(
            "DELETE FROM kv WHERE scope = ?1",
            params![Scope::Device(device.to_string()).as_text()],
        )?)
    }
}

/// `LIKE` treats these as wildcards, so a prefix containing one would match
/// more than the caller asked for. `list("a_")` must not return `ab`.
fn escape_like(prefix: &str) -> String {
    prefix
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Refuse a slug that would not be a filename inside `dir`.
///
/// Slugs arrive from the registry lowercase, alphanumeric and hyphenated, so
/// this never fires in practice. It is here because the storage API resolves an
/// app from the `Host` header, which is attacker-supplied: a slug is one
/// `../` away from being a path, and the check belongs at the boundary that
/// builds the path rather than in whichever caller is currently trusted.
fn validate_slug(slug: &str) -> Result<&str, StoreError> {
    let ok = !slug.is_empty()
        && slug.len() <= 63
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(slug)
    } else {
        Err(StoreError::BadSlug(slug.to_string()))
    }
}

fn apply(conn: &Connection) -> Result<(), rusqlite::Error> {
    let version: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        conn.execute_batch(sql)?;
        conn.pragma_update(None, "user_version", index as u32 + 1)?;
    }
    Ok(())
}

/// Every app's store, opened on demand.
///
/// Connections are cached because opening a SQLite file per request would put a
/// filesystem round trip in front of every `storage.get` an app makes, and a
/// checklist makes one per item.
pub struct Storage {
    root: PathBuf,
    open: Mutex<HashMap<String, Arc<AppStore>>>,
}

impl Storage {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            open: Mutex::new(HashMap::new()),
        }
    }

    /// The store for one app, opening it the first time it is asked for.
    pub fn app(&self, slug: &str) -> Result<Arc<AppStore>, StoreError> {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = open.get(slug) {
            return Ok(Arc::clone(store));
        }
        let store = Arc::new(AppStore::open(&self.root, slug)?);
        open.insert(slug.to_string(), Arc::clone(&store));
        Ok(store)
    }

    /// Drop an app's data entirely.
    ///
    /// Called when an app is forgotten. Deliberately *not* called when an app
    /// merely disappears from the workspace: a folder that has been moved or is
    /// on an unmounted drive would otherwise take its data with it, and the
    /// window promises that forgetting is the deliberate act.
    pub fn drop_app(&self, slug: &str) -> Result<(), StoreError> {
        let slug = validate_slug(slug)?;
        self.open
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(slug);

        for suffix in ["db", "db-wal", "db-shm"] {
            let path = self.root.join(format!("{slug}.{suffix}"));
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(StoreError::CreateDir {
                        path: path.display().to_string(),
                        source,
                    })
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory, in the shape the other crates here use one rather
    /// than as a new dependency for four tests.
    mod tempdir {
        use std::path::{Path, PathBuf};

        pub struct TempDir(PathBuf);

        impl TempDir {
            pub fn new() -> Self {
                use std::sync::atomic::{AtomicU32, Ordering};
                static SEQ: AtomicU32 = AtomicU32::new(0);

                let mut path = std::env::temp_dir();
                path.push(format!(
                    "kt-store-{}-{}",
                    std::process::id(),
                    SEQ.fetch_add(1, Ordering::Relaxed)
                ));
                let _ = std::fs::remove_dir_all(&path);
                std::fs::create_dir_all(&path).expect("creates temp dir");
                Self(path)
            }

            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    fn store() -> AppStore {
        AppStore::in_memory().expect("opens")
    }

    fn device() -> Scope {
        Scope::Device("dev-1".to_string())
    }

    #[test]
    fn a_value_comes_back_as_it_went_in() {
        let store = store();
        store
            .set(&Scope::Shared, "items", r#"["passports"]"#)
            .unwrap();
        assert_eq!(
            store.get(&Scope::Shared, "items").unwrap().as_deref(),
            Some(r#"["passports"]"#)
        );
    }

    #[test]
    fn a_missing_key_is_none_rather_than_an_error() {
        assert_eq!(store().get(&Scope::Shared, "nothing").unwrap(), None);
    }

    #[test]
    fn one_device_cannot_read_another_devices_scope() {
        // The private half of a shared app. Two viewers of one trip planner
        // keep separate notes and neither is visible to the other.
        let store = store();
        store.set(&device(), "notes", r#""adapters""#).unwrap();

        let other = Scope::Device("dev-2".to_string());
        assert_eq!(store.get(&other, "notes").unwrap(), None);
        assert_eq!(store.get(&Scope::Shared, "notes").unwrap(), None);
    }

    #[test]
    fn a_device_called_app_does_not_land_in_the_shared_store() {
        // Device ids are minted at random and nothing stops one being `app`.
        // Storing the scope bare would have handed that device everyone's data.
        let store = store();
        store.set(&Scope::Shared, "k", r#""shared""#).unwrap();
        store
            .set(&Scope::Device("app".to_string()), "k", r#""mine""#)
            .unwrap();

        assert_eq!(
            store.get(&Scope::Shared, "k").unwrap().as_deref(),
            Some(r#""shared""#)
        );
    }

    #[test]
    fn writing_again_replaces_rather_than_duplicates() {
        let store = store();
        store.set(&Scope::Shared, "k", "1").unwrap();
        store.set(&Scope::Shared, "k", "2").unwrap();

        assert_eq!(
            store.get(&Scope::Shared, "k").unwrap().as_deref(),
            Some("2")
        );
        assert_eq!(store.list(&Scope::Shared, "").unwrap().len(), 1);
    }

    #[test]
    fn a_list_is_narrowed_by_prefix_and_ordered_by_key() {
        let store = store();
        store.set(&Scope::Shared, "day:2", "b").unwrap();
        store.set(&Scope::Shared, "day:1", "a").unwrap();
        store.set(&Scope::Shared, "budget", "c").unwrap();

        let keys: Vec<_> = store
            .list(&Scope::Shared, "day:")
            .unwrap()
            .into_iter()
            .map(|entry| entry.key)
            .collect();
        assert_eq!(keys, vec!["day:1", "day:2"]);
    }

    #[test]
    fn a_prefix_containing_a_wildcard_matches_itself_only() {
        // `_` is a LIKE wildcard. Unescaped, listing `a_` would also return
        // `ab`, which is somebody else's key.
        let store = store();
        store.set(&Scope::Shared, "a_1", "x").unwrap();
        store.set(&Scope::Shared, "ab1", "y").unwrap();

        let keys: Vec<_> = store
            .list(&Scope::Shared, "a_")
            .unwrap()
            .into_iter()
            .map(|entry| entry.key)
            .collect();
        assert_eq!(keys, vec!["a_1"]);
    }

    #[test]
    fn deleting_says_whether_there_was_anything_there() {
        let store = store();
        store.set(&Scope::Shared, "k", "1").unwrap();

        assert!(store.delete(&Scope::Shared, "k").unwrap());
        assert!(!store.delete(&Scope::Shared, "k").unwrap());
    }

    #[test]
    fn a_write_over_quota_is_refused() {
        let store = store().with_quota(64);
        let err = store
            .set(&Scope::Shared, "k", &"x".repeat(100))
            .unwrap_err();

        assert!(matches!(err, StoreError::Quota { .. }));
        assert_eq!(store.get(&Scope::Shared, "k").unwrap(), None);
    }

    #[test]
    fn a_full_store_can_still_be_shrunk() {
        // The check counts what the store would hold afterwards. Counting the
        // new value on top of the old one would leave an app that had filled
        // its quota unable to write its way back out.
        let store = store().with_quota(64);
        store.set(&Scope::Shared, "k", &"x".repeat(60)).unwrap();

        store
            .set(&Scope::Shared, "k", "small")
            .expect("replacing shrinks it");
        assert_eq!(
            store.get(&Scope::Shared, "k").unwrap().as_deref(),
            Some("small")
        );
    }

    #[test]
    fn the_per_viewer_list_names_devices_and_not_the_shared_store() {
        let store = store();
        store.set(&Scope::Shared, "items", "[]").unwrap();
        store.set(&device(), "notes", "1").unwrap();
        store.set(&device(), "draft", "2").unwrap();

        assert_eq!(
            store.device_scopes().unwrap(),
            vec![("dev-1".to_string(), 2)]
        );
    }

    #[test]
    fn forgetting_a_device_leaves_the_shared_store_alone() {
        let store = store();
        store.set(&Scope::Shared, "items", "[]").unwrap();
        store.set(&device(), "notes", "1").unwrap();

        assert_eq!(store.forget_device("dev-1").unwrap(), 1);
        assert_eq!(store.get(&device(), "notes").unwrap(), None);
        assert!(store.get(&Scope::Shared, "items").unwrap().is_some());
    }

    #[test]
    fn bytes_counts_keys_and_values() {
        let store = store();
        store.set(&Scope::Shared, "ab", "cde").unwrap();
        assert_eq!(store.bytes().unwrap(), 5);
    }

    #[test]
    fn a_slug_that_is_a_path_is_refused() {
        // The storage API resolves an app from the Host header, so this is
        // attacker-supplied. `../` must never reach `dir.join`.
        for bad in ["../escape", "a/b", "", "Upper", &"x".repeat(64)] {
            assert!(
                matches!(validate_slug(bad), Err(StoreError::BadSlug(_))),
                "{bad:?} should be refused"
            );
        }
        assert!(validate_slug("chores-rota").is_ok());
    }

    #[test]
    fn two_apps_do_not_share_a_store() {
        let dir = tempdir::TempDir::new();
        let storage = Storage::new(dir.path().to_path_buf());

        storage
            .app("trip")
            .unwrap()
            .set(&Scope::Shared, "k", "1")
            .unwrap();
        assert_eq!(
            storage
                .app("rota")
                .unwrap()
                .get(&Scope::Shared, "k")
                .unwrap(),
            None
        );
    }

    #[test]
    fn the_same_app_asked_for_twice_is_the_same_store() {
        let dir = tempdir::TempDir::new();
        let storage = Storage::new(dir.path().to_path_buf());

        storage
            .app("trip")
            .unwrap()
            .set(&Scope::Shared, "k", "1")
            .unwrap();
        assert_eq!(
            storage
                .app("trip")
                .unwrap()
                .get(&Scope::Shared, "k")
                .unwrap()
                .as_deref(),
            Some("1")
        );
    }

    #[test]
    fn dropping_an_app_takes_its_data_with_it() {
        let dir = tempdir::TempDir::new();
        let storage = Storage::new(dir.path().to_path_buf());
        storage
            .app("trip")
            .unwrap()
            .set(&Scope::Shared, "k", "1")
            .unwrap();

        storage.drop_app("trip").unwrap();

        assert_eq!(
            storage
                .app("trip")
                .unwrap()
                .get(&Scope::Shared, "k")
                .unwrap(),
            None
        );
    }
}
