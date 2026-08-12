//! The set of apps currently being served.
//!
//! Held behind an `RwLock` and replaced wholesale on rescan, so a request
//! either sees the whole old library or the whole new one, never a half-applied
//! update.

use std::collections::HashMap;
use std::sync::RwLock;

use kt_server::{AppSource, ServedApp};
use kt_types::AppRecord;

#[derive(Default)]
pub struct Library {
    apps: RwLock<HashMap<String, Entry>>,
}

struct Entry {
    record: AppRecord,
    served: ServedApp,
}

impl Library {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the whole library.
    ///
    /// A folder whose path will not canonicalise is dropped with a warning
    /// rather than failing the swap: one bad app must not take the library
    /// down.
    pub fn replace(&self, records: Vec<AppRecord>) {
        let mut next = HashMap::with_capacity(records.len());

        for record in records {
            let root = match std::path::Path::new(&record.path).canonicalize() {
                Ok(root) => root,
                Err(e) => {
                    tracing::warn!(path = %record.path, error = %e, "cannot serve app");
                    continue;
                }
            };

            let served = ServedApp {
                slug: record.manifest.slug.clone(),
                name: record.manifest.name.clone(),
                root,
                entry: record.manifest.entry.clone(),
            };
            next.insert(record.manifest.slug.clone(), Entry { record, served });
        }

        *self.write() = next;
    }

    pub fn records(&self) -> Vec<AppRecord> {
        let apps = self.read();
        let mut out: Vec<_> = apps.values().map(|e| e.record.clone()).collect();
        out.sort_by_key(|r| r.manifest.name.to_lowercase());
        out
    }

    pub fn record(&self, slug: &str) -> Option<AppRecord> {
        self.read().get(slug).map(|e| e.record.clone())
    }

    pub fn len(&self) -> usize {
        self.read().len()
    }

    /// Paired with `len` so the type reads normally; the UI asks for this
    /// to show the empty-workspace state in D3.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A poisoned lock means a reader panicked; the data is still consistent
    /// because every write is a whole-map swap.
    fn read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, Entry>> {
        self.apps.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, Entry>> {
        self.apps.write().unwrap_or_else(|e| e.into_inner())
    }
}

impl AppSource for Library {
    fn get(&self, slug: &str) -> Option<ServedApp> {
        self.read().get(slug).map(|e| e.served.clone())
    }

    fn list(&self) -> Vec<ServedApp> {
        let apps = self.read();
        let mut out: Vec<_> = apps.values().map(|e| e.served.clone()).collect();
        out.sort_by_key(|a| a.name.to_lowercase());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kt_types::{AppManifest, Visibility};

    fn record(slug: &str, name: &str, path: &str) -> AppRecord {
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
            path: path.into(),
        }
    }

    #[test]
    fn an_unservable_path_is_skipped_without_losing_the_rest() {
        let dir = std::env::temp_dir();
        let library = Library::new();
        library.replace(vec![
            record("real", "Real", &dir.display().to_string()),
            record("ghost", "Ghost", "/nonexistent/path/for/sure"),
        ]);

        assert_eq!(library.len(), 1);
        assert!(library.get("real").is_some());
        assert!(library.get("ghost").is_none());
    }

    #[test]
    fn replacing_removes_what_is_gone() {
        let dir = std::env::temp_dir().display().to_string();
        let library = Library::new();
        library.replace(vec![record("a", "A", &dir), record("b", "B", &dir)]);
        assert_eq!(library.len(), 2);

        library.replace(vec![record("a", "A", &dir)]);
        assert_eq!(library.len(), 1);
        assert!(library.get("b").is_none());
    }

    #[test]
    fn listings_are_name_ordered() {
        let dir = std::env::temp_dir().display().to_string();
        let library = Library::new();
        library.replace(vec![
            record("z", "Alpha", &dir),
            record("a", "zulu", &dir),
            record("m", "Middle", &dir),
        ]);

        let names: Vec<_> = library.list().into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["Alpha", "Middle", "zulu"]);
    }
}
