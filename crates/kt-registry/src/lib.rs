//! Watches the workspace and turns folders into apps.
//!
//! One folder in the workspace is one app. A folder with no `app.json` gets one
//! generated, with the name taken from the folder and visibility Private, so
//! nothing becomes reachable merely by appearing on disk
//! (docs/architecture.md section 8).

use std::path::{Path, PathBuf};
use std::time::Duration;

use kt_types::{AppManifest, AppRecord, Visibility};
use notify::{RecursiveMode, Watcher};

pub mod slug;

/// How long to wait for the filesystem to settle before rescanning. Editors
/// and agents write several files in a burst; rescanning per event would
/// serve half-written apps.
const DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("could not read the workspace at {path}: {source}")]
    ReadWorkspace {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not watch the workspace: {0}")]
    Watch(#[from] notify::Error),
}

pub struct Registry {
    workspace: PathBuf,
}

impl Registry {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// Create the workspace if it does not exist yet. First run does this so
    /// onboarding can point at a folder that is really there.
    pub fn ensure_workspace(&self) -> Result<(), RegistryError> {
        std::fs::create_dir_all(&self.workspace).map_err(|source| RegistryError::Write {
            path: self.workspace.display().to_string(),
            source,
        })
    }

    /// Read every app folder, generating manifests where they are missing.
    ///
    /// Sorted by slug so two scans of an unchanged workspace agree, which is
    /// what makes the diff in [`diff`] meaningful.
    pub fn scan(&self) -> Result<Vec<AppRecord>, RegistryError> {
        let entries =
            std::fs::read_dir(&self.workspace).map_err(|source| RegistryError::ReadWorkspace {
                path: self.workspace.display().to_string(),
                source,
            })?;

        let mut folders: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir() && !is_hidden(p))
            .collect();
        folders.sort();

        let mut taken: Vec<String> = Vec::with_capacity(folders.len());
        let mut records = Vec::with_capacity(folders.len());

        for folder in folders {
            match self.load_or_create(&folder, &taken) {
                Ok(record) => {
                    taken.push(record.manifest.slug.clone());
                    records.push(record);
                }
                // One unreadable folder must not hide the rest of the library.
                Err(e) => tracing::warn!(path = %folder.display(), error = %e, "skipping folder"),
            }
        }

        records.sort_by(|a, b| a.manifest.slug.cmp(&b.manifest.slug));
        Ok(records)
    }

    fn load_or_create(&self, folder: &Path, taken: &[String]) -> Result<AppRecord, RegistryError> {
        let manifest_path = folder.join("app.json");
        let folder_name = folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "app".to_string());

        let manifest = match std::fs::read_to_string(&manifest_path) {
            Ok(raw) => match serde_json::from_str::<AppManifest>(&raw) {
                Ok(m) => m,
                // A hand-edited manifest with a typo should not delete the
                // app from the library; fall back to defaults and say so.
                Err(e) => {
                    tracing::warn!(
                        path = %manifest_path.display(),
                        error = %e,
                        "unreadable app.json, using defaults"
                    );
                    self.generate(&folder_name, taken, &manifest_path)?
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.generate(&folder_name, taken, &manifest_path)?
            }
            Err(source) => {
                return Err(RegistryError::Write {
                    path: manifest_path.display().to_string(),
                    source,
                })
            }
        };

        Ok(AppRecord {
            manifest,
            path: folder.display().to_string(),
        })
    }

    /// Write a default manifest next to the content, so the app is
    /// self-describing and portable: copy the folder elsewhere and it is still
    /// the same app.
    fn generate(
        &self,
        folder_name: &str,
        taken: &[String],
        manifest_path: &Path,
    ) -> Result<AppManifest, RegistryError> {
        let manifest = AppManifest {
            name: folder_name.to_string(),
            slug: slug::deduplicate(&slug::slugify(folder_name), taken),
            icon: None,
            entry: "index.html".to_string(),
            visibility: Visibility::Private,
            version: 1,
            extra: serde_json::Map::new(),
        };

        let json = serde_json::to_string_pretty(&manifest).unwrap_or_default();
        std::fs::write(manifest_path, format!("{json}\n")).map_err(|source| {
            RegistryError::Write {
                path: manifest_path.display().to_string(),
                source,
            }
        })?;
        tracing::info!(slug = %manifest.slug, "generated app.json");

        Ok(manifest)
    }

    /// Watch the workspace, calling `on_change` once the filesystem settles.
    ///
    /// Deliberately coarse: any event triggers a full rescan rather than
    /// per-path bookkeeping. A workspace is tens of folders, a rescan is
    /// microseconds, and the state machine that per-path updates would need is
    /// where this kind of code usually goes wrong.
    ///
    /// Trailing edge, not leading. Firing on the first event and ignoring the
    /// rest of the burst loses whatever happened during the quiet window - a
    /// folder deleted just after one was added would stay in the library until
    /// some unrelated change came along. So events are coalesced and the rescan
    /// runs after things go quiet.
    ///
    /// The returned watcher must be kept alive; dropping it stops the watch.
    pub fn watch(
        &self,
        mut on_change: impl FnMut() + Send + 'static,
    ) -> Result<impl Watcher, RegistryError> {
        let (tx, rx) = std::sync::mpsc::channel::<()>();

        let mut watcher = notify::recommended_watcher(
            move |res: Result<notify::Event, notify::Error>| match res {
                Ok(event) if is_interesting(&event) => {
                    // A closed channel means the coalescing thread is gone,
                    // which happens only during shutdown.
                    let _ = tx.send(());
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "watch error"),
            },
        )?;

        watcher.watch(&self.workspace, RecursiveMode::Recursive)?;

        std::thread::Builder::new()
            .name("kt-workspace".into())
            .spawn(move || {
                use std::sync::mpsc::RecvTimeoutError;

                while rx.recv().is_ok() {
                    // Drain the burst: keep waiting until nothing has arrived
                    // for a whole debounce interval.
                    loop {
                        match rx.recv_timeout(DEBOUNCE) {
                            Ok(()) => continue,
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    on_change();
                }
            })
            .map(|_| ())
            .unwrap_or_else(|e| tracing::warn!(error = %e, "could not start the watch thread"));

        Ok(watcher)
    }
}

/// What changed between two scans.
#[derive(Debug, Default, PartialEq)]
pub struct Diff {
    pub changed: Vec<AppRecord>,
    pub removed: Vec<String>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty()
    }
}

/// Compare a previous scan with a new one. Both must be slug-sorted, which
/// [`Registry::scan`] guarantees.
pub fn diff(previous: &[AppRecord], current: &[AppRecord]) -> Diff {
    let changed = current
        .iter()
        .filter(|c| {
            !previous
                .iter()
                .any(|p| p.manifest.slug == c.manifest.slug && p == *c)
        })
        .cloned()
        .collect();

    let removed = previous
        .iter()
        .filter(|p| !current.iter().any(|c| c.manifest.slug == p.manifest.slug))
        .map(|p| p.manifest.slug.clone())
        .collect();

    Diff { changed, removed }
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.'))
}

/// Ignore noise the editor makes: dotfiles, swap files, and macOS metadata.
fn is_interesting(event: &notify::Event) -> bool {
    event.paths.iter().any(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| !n.starts_with('.') && !n.ends_with('~'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_with(folders: &[(&str, &[(&str, &str)])]) -> tempdir::TempDir {
        let dir = tempdir::TempDir::new();
        for (name, files) in folders {
            let folder = dir.path().join(name);
            std::fs::create_dir_all(&folder).expect("creates folder");
            for (file, contents) in *files {
                std::fs::write(folder.join(file), contents).expect("writes file");
            }
        }
        dir
    }

    #[test]
    fn a_folder_becomes_an_app() {
        let ws = workspace_with(&[("Trip Planner", &[("index.html", "<h1>hi</h1>")])]);
        let apps = Registry::new(ws.path()).scan().expect("scans");

        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].manifest.slug, "trip-planner");
        assert_eq!(apps[0].manifest.name, "Trip Planner");
    }

    #[test]
    fn a_generated_manifest_is_written_next_to_the_content() {
        let ws = workspace_with(&[("Notes", &[("index.html", "")])]);
        Registry::new(ws.path()).scan().expect("scans");

        let written = std::fs::read_to_string(ws.path().join("Notes/app.json")).expect("exists");
        let parsed: AppManifest = serde_json::from_str(&written).expect("parses");
        assert_eq!(parsed.slug, "notes");
        assert_eq!(parsed.visibility, Visibility::Private);
    }

    #[test]
    fn nothing_is_shared_just_by_existing() {
        let ws = workspace_with(&[("Anything", &[("index.html", "")])]);
        let apps = Registry::new(ws.path()).scan().expect("scans");
        assert_eq!(apps[0].manifest.visibility, Visibility::Private);
    }

    #[test]
    fn an_existing_manifest_wins_over_defaults() {
        let ws = workspace_with(&[(
            "whatever",
            &[(
                "app.json",
                r#"{"name":"Real Name","slug":"real","visibility":"network","version":9}"#,
            )],
        )]);
        let apps = Registry::new(ws.path()).scan().expect("scans");

        assert_eq!(apps[0].manifest.name, "Real Name");
        assert_eq!(apps[0].manifest.slug, "real");
        assert_eq!(apps[0].manifest.visibility, Visibility::Network);
        assert_eq!(apps[0].manifest.version, 9);
    }

    #[test]
    fn a_broken_manifest_does_not_lose_the_app() {
        let ws = workspace_with(&[("Notes", &[("app.json", "{ this is not json")])]);
        let apps = Registry::new(ws.path()).scan().expect("scans");

        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].manifest.slug, "notes");
    }

    #[test]
    fn colliding_folder_names_get_distinct_slugs() {
        let ws = workspace_with(&[("My App", &[]), ("my-app", &[]), ("my_app", &[])]);
        let apps = Registry::new(ws.path()).scan().expect("scans");

        let mut slugs: Vec<_> = apps.iter().map(|a| a.manifest.slug.as_str()).collect();
        slugs.sort();
        assert_eq!(slugs, ["my-app", "my-app-2", "my-app-3"]);
    }

    #[test]
    fn hidden_folders_and_loose_files_are_not_apps() {
        let ws = workspace_with(&[(".git", &[("config", "")]), ("real", &[])]);
        std::fs::write(ws.path().join("README.md"), "not an app").expect("writes");

        let apps = Registry::new(ws.path()).scan().expect("scans");
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].manifest.slug, "real");
    }

    #[test]
    fn scanning_twice_gives_the_same_answer() {
        let ws = workspace_with(&[("b", &[]), ("a", &[]), ("c", &[])]);
        let registry = Registry::new(ws.path());
        assert_eq!(
            registry.scan().expect("first"),
            registry.scan().expect("second")
        );
    }

    #[test]
    fn diff_reports_additions_and_removals() {
        let ws = workspace_with(&[("one", &[])]);
        let registry = Registry::new(ws.path());
        let first = registry.scan().expect("scans");

        std::fs::create_dir_all(ws.path().join("two")).expect("creates");
        let second = registry.scan().expect("scans");

        let d = diff(&first, &second);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].manifest.slug, "two");
        assert!(d.removed.is_empty());

        let back = diff(&second, &first);
        assert_eq!(back.removed, ["two"]);
    }

    #[test]
    fn an_unchanged_workspace_diffs_to_nothing() {
        let ws = workspace_with(&[("one", &[]), ("two", &[])]);
        let registry = Registry::new(ws.path());
        let a = registry.scan().expect("scans");
        let b = registry.scan().expect("scans");
        assert!(diff(&a, &b).is_empty());
    }

    #[test]
    fn a_missing_workspace_is_an_error_not_an_empty_library() {
        // Silently reporting zero apps would look identical to "you have no
        // apps", which is exactly the wrong thing to tell someone whose
        // workspace folder went missing.
        let registry = Registry::new("/nonexistent/kitchen/table");
        assert!(matches!(
            registry.scan(),
            Err(RegistryError::ReadWorkspace { .. })
        ));
    }

    /// Minimal scoped temp directory; avoids a dependency for eight tests.
    mod tempdir {
        use std::path::{Path, PathBuf};

        pub struct TempDir(PathBuf);

        impl TempDir {
            pub fn new() -> Self {
                use std::sync::atomic::{AtomicU32, Ordering};
                static SEQ: AtomicU32 = AtomicU32::new(0);

                let mut path = std::env::temp_dir();
                path.push(format!(
                    "kt-registry-{}-{}",
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
}
