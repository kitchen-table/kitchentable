//! Resolving a request path to a file inside an app folder.
//!
//! This is the security boundary for serving. Every served path is
//! canonicalised and checked to still be inside the app root, and symlinks
//! pointing outside the folder are refused (docs/architecture.md section 14).
//! Any change here ships with a traversal test.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// The path escapes the app folder, or resolves through a symlink that
    /// leaves it.
    Escapes,
    /// Nothing is there.
    NotFound,
}

/// Resolve `requested` inside `root`, or refuse.
///
/// `root` must already be canonical; the daemon canonicalises app roots once at
/// registration rather than per request.
pub fn resolve(root: &Path, requested: &str) -> Result<PathBuf, ResolveError> {
    // A NUL would be truncated by the syscall, so a path containing one is
    // never what it appears to be.
    if requested.contains('\0') {
        return Err(ResolveError::Escapes);
    }

    // Build the path lexically first and refuse anything that climbs, so we
    // never even stat a path outside the folder.
    let mut safe = PathBuf::new();
    for component in Path::new(requested).components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            // `..`, `/`, and Windows prefixes all mean this request was trying
            // to leave, whether or not the target happens to exist.
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ResolveError::Escapes)
            }
        }
    }

    let candidate = root.join(&safe);

    // Canonicalise to collapse any symlink in the middle of the path, then
    // confirm we are still inside. This is what catches a symlink in the app
    // folder pointing at ~/.ssh.
    let canonical = candidate
        .canonicalize()
        .map_err(|_| ResolveError::NotFound)?;
    if !canonical.starts_with(root) {
        return Err(ResolveError::Escapes);
    }

    Ok(canonical)
}

/// Resolve to a file, following the app's entry point for directories.
///
/// `/` and `/subdir/` both land on `entry` inside that directory, which is what
/// makes a plain folder of HTML work with no configuration.
pub fn resolve_file(root: &Path, requested: &str, entry: &str) -> Result<PathBuf, ResolveError> {
    let resolved = resolve(root, requested.trim_start_matches('/'))?;

    if resolved.is_dir() {
        let with_entry = resolved.join(entry);
        let canonical = with_entry
            .canonicalize()
            .map_err(|_| ResolveError::NotFound)?;
        if !canonical.starts_with(root) {
            return Err(ResolveError::Escapes);
        }
        if canonical.is_dir() {
            return Err(ResolveError::NotFound);
        }
        return Ok(canonical);
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        outside: PathBuf,
        _dir: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static SEQ: AtomicU32 = AtomicU32::new(0);

            let base = std::env::temp_dir().join(format!(
                "kt-server-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&base);

            let root = base.join("app");
            let outside = base.join("secrets");
            std::fs::create_dir_all(root.join("sub")).expect("creates app dirs");
            std::fs::create_dir_all(&outside).expect("creates outside dir");

            std::fs::write(root.join("index.html"), "<h1>home</h1>").expect("writes");
            std::fs::write(root.join("sub/index.html"), "<h1>sub</h1>").expect("writes");
            std::fs::write(root.join("sub/style.css"), "body{}").expect("writes");
            std::fs::write(outside.join("id_rsa"), "PRIVATE KEY").expect("writes");

            Self {
                root: root.canonicalize().expect("canonicalises"),
                outside: outside.canonicalize().expect("canonicalises"),
                _dir: base,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self._dir);
        }
    }

    #[test]
    fn serves_a_file_inside_the_app() {
        let f = Fixture::new();
        let got = resolve(&f.root, "sub/style.css").expect("resolves");
        assert_eq!(got, f.root.join("sub/style.css"));
    }

    #[test]
    fn a_directory_resolves_to_the_entry_point() {
        let f = Fixture::new();
        assert_eq!(
            resolve_file(&f.root, "/", "index.html").expect("resolves"),
            f.root.join("index.html")
        );
        assert_eq!(
            resolve_file(&f.root, "/sub/", "index.html").expect("resolves"),
            f.root.join("sub/index.html")
        );
    }

    #[test]
    fn missing_files_are_not_found_rather_than_forbidden() {
        let f = Fixture::new();
        assert_eq!(
            resolve(&f.root, "nope.html"),
            Err(ResolveError::NotFound),
            "a 404 must not be reported as a traversal attempt"
        );
    }

    // ---- traversal suite ----

    #[test]
    fn refuses_dot_dot() {
        let f = Fixture::new();
        for attempt in [
            "../secrets/id_rsa",
            "../../etc/passwd",
            "sub/../../secrets/id_rsa",
            "..",
            "sub/..",
        ] {
            assert_eq!(
                resolve(&f.root, attempt),
                Err(ResolveError::Escapes),
                "{attempt:?} should have been refused"
            );
        }
    }

    #[test]
    fn refuses_absolute_paths() {
        let f = Fixture::new();
        for attempt in ["/etc/passwd", "/", "//etc/passwd"] {
            assert_eq!(
                resolve(&f.root, attempt),
                Err(ResolveError::Escapes),
                "{attempt:?} should have been refused"
            );
        }
    }

    #[test]
    fn refuses_a_null_byte() {
        let f = Fixture::new();
        assert_eq!(
            resolve(&f.root, "index.html\0.png"),
            Err(ResolveError::Escapes)
        );
    }

    #[test]
    fn refuses_a_symlink_that_leaves_the_app() {
        let f = Fixture::new();
        std::os::unix::fs::symlink(f.outside.join("id_rsa"), f.root.join("leak")).expect("links");

        assert_eq!(
            resolve(&f.root, "leak"),
            Err(ResolveError::Escapes),
            "a symlink out of the app folder must be refused"
        );
    }

    #[test]
    fn refuses_a_symlinked_directory_that_leaves_the_app() {
        let f = Fixture::new();
        std::os::unix::fs::symlink(&f.outside, f.root.join("out")).expect("links");

        assert_eq!(resolve(&f.root, "out/id_rsa"), Err(ResolveError::Escapes));
    }

    #[test]
    fn allows_a_symlink_that_stays_inside() {
        let f = Fixture::new();
        std::os::unix::fs::symlink(f.root.join("sub/style.css"), f.root.join("alias.css"))
            .expect("links");

        assert_eq!(
            resolve(&f.root, "alias.css").expect("resolves"),
            f.root.join("sub/style.css")
        );
    }

    #[test]
    fn a_directory_without_an_entry_point_is_not_found() {
        let f = Fixture::new();
        std::fs::create_dir_all(f.root.join("empty")).expect("creates");
        assert_eq!(
            resolve_file(&f.root, "/empty/", "index.html"),
            Err(ResolveError::NotFound)
        );
    }

    #[test]
    fn a_prefix_sharing_the_roots_name_does_not_count_as_inside() {
        // /tmp/x/app-secrets must not pass a check against /tmp/x/app.
        let f = Fixture::new();
        let sibling = f.root.with_file_name("app-secrets");
        std::fs::create_dir_all(&sibling).expect("creates");
        std::fs::write(sibling.join("k"), "secret").expect("writes");
        std::os::unix::fs::symlink(sibling.join("k"), f.root.join("k")).expect("links");

        assert_eq!(resolve(&f.root, "k"), Err(ResolveError::Escapes));
    }
}
