//! Making apps: a new empty one, or an existing folder brought in.
//!
//! Dropping a folder into the workspace by hand has always worked - the
//! registry watcher notices within seconds. These are the same act performed
//! from the socket, so the window, the CLI and (in D6) an agent can all do what
//! a person with a Finder window can.
//!
//! Everything here works in folder names and files, and deliberately writes no
//! `app.json`. The registry generates manifests for folders that lack one, and
//! having a second place that mints them is how the two drift.

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum AuthorError {
    #[error("an app needs a name")]
    NameEmpty,
    #[error("{0:?} cannot be a folder name")]
    NameUnusable(String),
    #[error("{0:?} is not a folder")]
    NotAFolder(String),
    #[error("{0:?} does not exist")]
    Missing(String),
    #[error("{0:?} is already in your workspace")]
    AlreadyInWorkspace(String),
    #[error("{0:?} contains your workspace")]
    ContainsWorkspace(String),
    #[error("could not write {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// The starter page a new empty app gets.
///
/// Deliberately one self-contained file with no build step, because that is the
/// claim the product makes. It also names the folder it lives in, so the first
/// thing someone does - rename the heading - teaches where the content is.
fn starter(name: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{name}</title>
<style>
  body {{
    margin: 0;
    min-height: 100vh;
    display: grid;
    place-items: center;
    font: 16px/1.6 system-ui, sans-serif;
    background: #fbfaf8;
    color: #1a1a1a;
  }}
  main {{ max-width: 32rem; padding: 2rem; text-align: center; }}
  h1 {{ font-size: 2rem; margin: 0 0 .5rem; letter-spacing: -.02em; }}
  p {{ color: #4a463f; margin: 0; }}
  code {{ font-family: ui-monospace, monospace; font-size: .9em; }}
</style>
</head>
<body>
<main>
  <h1>{name}</h1>
  <p>
    This page is <code>index.html</code> in your {name} folder.
    Edit it and refresh - there is nothing to rebuild.
  </p>
</main>
</body>
</html>
"#
    )
}

/// Turn what someone typed into something that can be a folder name.
///
/// Rejects rather than mangles where mangling would surprise: a name that is
/// only punctuation has no obvious folder name, and silently inventing one
/// means the app is not where they expect it.
fn folder_name(raw: &str) -> Result<String, AuthorError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AuthorError::NameEmpty);
    }

    // Path separators, control characters and the Windows-reserved set, so a
    // workspace stays portable across the three platforms we ship on.
    let cleaned: String = trimmed
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '-'
            } else {
                c
            }
        })
        .collect();

    // A leading dot makes the folder hidden, and the registry skips hidden
    // folders - the app would be created and then never appear.
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());

    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return Err(AuthorError::NameUnusable(raw.to_string()));
    }

    // Long enough for any real name, short enough to stay under the 255-byte
    // limit every filesystem we target shares. Truncated on a char boundary.
    Ok(cleaned
        .chars()
        .take(64)
        .collect::<String>()
        .trim()
        .to_string())
}

/// `Name`, or `Name 2` if that is taken. Matches what Finder does, because
/// this is the same act.
fn unique_in(workspace: &Path, wanted: &str) -> String {
    if !workspace.join(wanted).exists() {
        return wanted.to_string();
    }
    for n in 2..1000 {
        let candidate = format!("{wanted} {n}");
        if !workspace.join(&candidate).exists() {
            return candidate;
        }
    }
    // A thousand folders of the same name is not a real workspace; suffixing
    // with the clock beats failing.
    format!("{wanted} {}", super::rpc::now())
}

/// Create an empty app. Returns the folder that was made.
pub fn create(workspace: &Path, name: &str) -> Result<PathBuf, AuthorError> {
    let wanted = folder_name(name)?;
    let folder = workspace.join(unique_in(workspace, &wanted));

    std::fs::create_dir_all(&folder).map_err(|source| AuthorError::Io {
        path: folder.display().to_string(),
        source,
    })?;

    let index = folder.join("index.html");
    let display = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| wanted.clone());

    std::fs::write(&index, starter(&display)).map_err(|source| AuthorError::Io {
        path: index.display().to_string(),
        source,
    })?;

    Ok(folder)
}

/// Copy an existing folder into the workspace. Returns the folder that was made.
///
/// A copy rather than a move: the source is someone's own directory, in their
/// own filesystem, and quietly relocating it is not ours to do. The UI says so.
pub fn import(workspace: &Path, source: &Path) -> Result<PathBuf, AuthorError> {
    let source = source
        .canonicalize()
        .map_err(|_| AuthorError::Missing(source.display().to_string()))?;

    if !source.is_dir() {
        return Err(AuthorError::NotAFolder(source.display().to_string()));
    }

    // Canonical on both sides, or a workspace reached through a symlink would
    // read as a different tree and the containment checks below would pass.
    let root = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());

    if source.starts_with(&root) {
        return Err(AuthorError::AlreadyInWorkspace(
            source.display().to_string(),
        ));
    }
    // Copying a parent of the workspace into the workspace never terminates.
    if root.starts_with(&source) {
        return Err(AuthorError::ContainsWorkspace(source.display().to_string()));
    }

    let wanted = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| AuthorError::NameUnusable(source.display().to_string()))?;
    let wanted = folder_name(&wanted)?;

    let destination = root.join(unique_in(&root, &wanted));
    copy_tree(&source, &destination)?;
    Ok(destination)
}

/// Names never worth copying: version control and dependency trees.
///
/// Neither is ever served, and both routinely dwarf the app - a 200 KB site
/// with a `.git` and a `node_modules` copies as hundreds of megabytes. Skipping
/// them is the difference between instant and apparently-hung.
const SKIP: [&str; 3] = [".git", "node_modules", ".DS_Store"];

fn copy_tree(source: &Path, destination: &Path) -> Result<(), AuthorError> {
    let io = |path: &Path, source: std::io::Error| AuthorError::Io {
        path: path.display().to_string(),
        source,
    };

    std::fs::create_dir_all(destination).map_err(|e| io(destination, e))?;

    // Iterative, so a deep tree cannot blow the stack.
    let mut pending = vec![(source.to_path_buf(), destination.to_path_buf())];

    while let Some((from, to)) = pending.pop() {
        let entries = std::fs::read_dir(&from).map_err(|e| io(&from, e))?;

        for entry in entries.flatten() {
            let name = entry.file_name();
            if SKIP.iter().any(|skip| *skip == name) {
                continue;
            }

            let child_from = entry.path();
            let child_to = to.join(&name);

            // Does not traverse, so a symlink is examined rather than followed:
            // a link pointing outside the source tree must not drag its target
            // into the workspace.
            let meta = match child_from.symlink_metadata() {
                Ok(meta) => meta,
                Err(_) => continue,
            };

            if meta.is_dir() {
                std::fs::create_dir_all(&child_to).map_err(|e| io(&child_to, e))?;
                pending.push((child_from, child_to));
            } else if meta.is_file() {
                std::fs::copy(&child_from, &child_to).map_err(|e| io(&child_to, e))?;
            }
            // Symlinks are skipped outright. An app that depends on one would
            // break the moment the folder is copied anywhere else anyway.
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempdir::TempDir {
        tempdir::TempDir::new()
    }

    #[test]
    fn a_new_app_gets_a_folder_and_a_page_that_works() {
        let ws = workspace();
        let folder = create(ws.path(), "Packing List").expect("creates");

        assert_eq!(folder.file_name().expect("name"), "Packing List");
        let html = std::fs::read_to_string(folder.join("index.html")).expect("reads");
        assert!(html.contains("<title>Packing List</title>"));
    }

    #[test]
    fn no_manifest_is_written_here() {
        // The registry owns app.json. Two writers would drift.
        let ws = workspace();
        let folder = create(ws.path(), "Notes").expect("creates");
        assert!(!folder.join("app.json").exists());
    }

    #[test]
    fn a_second_app_of_the_same_name_is_suffixed_rather_than_clobbering() {
        let ws = workspace();
        let first = create(ws.path(), "Notes").expect("creates");
        std::fs::write(first.join("mine.txt"), "keep me").expect("writes");

        let second = create(ws.path(), "Notes").expect("creates");

        assert_eq!(second.file_name().expect("name"), "Notes 2");
        assert!(first.join("mine.txt").exists(), "the first app survived");
    }

    #[test]
    fn a_name_that_would_escape_the_workspace_is_flattened() {
        let ws = workspace();
        let folder = create(ws.path(), "../../etc/evil").expect("creates");

        assert_eq!(folder.parent().expect("parent"), ws.path());
        assert!(!folder
            .file_name()
            .expect("name")
            .to_string_lossy()
            .contains('/'));
    }

    #[test]
    fn a_leading_dot_is_stripped_so_the_app_is_not_born_hidden() {
        // The registry skips hidden folders: the app would never appear.
        let ws = workspace();
        let folder = create(ws.path(), ".secret").expect("creates");
        assert_eq!(folder.file_name().expect("name"), "secret");
    }

    #[test]
    fn a_name_with_nothing_usable_in_it_is_refused() {
        let ws = workspace();
        assert!(matches!(
            create(ws.path(), "   "),
            Err(AuthorError::NameEmpty)
        ));
        assert!(matches!(
            create(ws.path(), "..."),
            Err(AuthorError::NameUnusable(_))
        ));
    }

    #[test]
    fn importing_copies_the_tree_and_leaves_the_original_alone() {
        let ws = workspace();
        let outside = workspace();
        let source = outside.path().join("Trip Planner");
        std::fs::create_dir_all(source.join("assets")).expect("creates");
        std::fs::write(source.join("index.html"), "<h1>hi</h1>").expect("writes");
        std::fs::write(source.join("assets/app.css"), "body{}").expect("writes");

        let folder = import(ws.path(), &source).expect("imports");

        assert_eq!(folder.file_name().expect("name"), "Trip Planner");
        assert_eq!(
            std::fs::read_to_string(folder.join("index.html")).expect("reads"),
            "<h1>hi</h1>"
        );
        assert!(
            folder.join("assets/app.css").exists(),
            "nested files came too"
        );
        assert!(
            source.join("index.html").exists(),
            "the original is untouched"
        );
    }

    #[test]
    fn importing_skips_version_control_and_dependencies() {
        let ws = workspace();
        let outside = workspace();
        let source = outside.path().join("Site");
        std::fs::create_dir_all(source.join(".git")).expect("creates");
        std::fs::create_dir_all(source.join("node_modules/left-pad")).expect("creates");
        std::fs::write(source.join(".git/HEAD"), "ref: main").expect("writes");
        std::fs::write(source.join("index.html"), "<h1>hi</h1>").expect("writes");

        let folder = import(ws.path(), &source).expect("imports");

        assert!(folder.join("index.html").exists());
        assert!(!folder.join(".git").exists(), "no repo came with it");
        assert!(!folder.join("node_modules").exists(), "no dependency tree");
    }

    #[test]
    fn importing_something_already_in_the_workspace_is_refused() {
        // It is already an app, or part of one. Copying it would duplicate it.
        let ws = workspace();
        let inside = ws.path().join("Already Here");
        std::fs::create_dir_all(&inside).expect("creates");

        assert!(matches!(
            import(ws.path(), &inside),
            Err(AuthorError::AlreadyInWorkspace(_))
        ));
    }

    #[test]
    fn importing_a_parent_of_the_workspace_is_refused() {
        // Otherwise the copy walks into its own destination, forever.
        let ws = workspace();
        let nested = ws.path().join("inner");
        std::fs::create_dir_all(&nested).expect("creates");

        assert!(matches!(
            import(&nested, ws.path()),
            Err(AuthorError::ContainsWorkspace(_))
        ));
    }

    #[test]
    fn importing_a_file_or_a_missing_path_is_refused() {
        let ws = workspace();
        let outside = workspace();
        let file = outside.path().join("page.html");
        std::fs::write(&file, "<h1>hi</h1>").expect("writes");

        assert!(matches!(
            import(ws.path(), &file),
            Err(AuthorError::NotAFolder(_))
        ));
        assert!(matches!(
            import(ws.path(), &outside.path().join("nope")),
            Err(AuthorError::Missing(_))
        ));
    }

    #[test]
    fn a_symlink_in_the_source_is_not_followed_out_of_the_tree() {
        // A link to / would otherwise copy the disk into the workspace.
        #[cfg(unix)]
        {
            let ws = workspace();
            let outside = workspace();
            let secret = outside.path().join("secret.txt");
            std::fs::write(&secret, "private").expect("writes");

            let source = outside.path().join("Site");
            std::fs::create_dir_all(&source).expect("creates");
            std::fs::write(source.join("index.html"), "<h1>hi</h1>").expect("writes");
            std::os::unix::fs::symlink(&secret, source.join("link.txt")).expect("links");

            let folder = import(ws.path(), &source).expect("imports");

            assert!(folder.join("index.html").exists());
            assert!(
                !folder.join("link.txt").exists(),
                "the link was not followed"
            );
        }
    }

    /// Minimal scoped temp directory, matching kt-registry's: a dependency for
    /// a dozen tests is not worth it (CLAUDE.md rule 6).
    mod tempdir {
        use std::path::{Path, PathBuf};

        pub struct TempDir(PathBuf);

        impl TempDir {
            pub fn new() -> Self {
                use std::sync::atomic::{AtomicU32, Ordering};
                static SEQ: AtomicU32 = AtomicU32::new(0);

                let mut path = std::env::temp_dir();
                path.push(format!(
                    "kt-authoring-{}-{}",
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
