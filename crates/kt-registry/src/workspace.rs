//! Whether a folder can be the workspace.
//!
//! Separate from [`crate::Registry`] because this runs *before* anything is a
//! workspace: the owner has just picked a folder in a native panel and the
//! daemon is the only end that can say whether it is a reasonable thing to
//! watch. The shell cannot - it does not know where the state directory is,
//! and it has no business knowing.
//!
//! Every refusal names the folder and says what is wrong with it, because the
//! owner is standing in front of a file picker and the useful answer is "not
//! that one, because —" rather than "invalid".

use std::path::{Path, PathBuf};

/// Why a folder cannot be the workspace.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkspaceError {
    #[error("a workspace has to be a full path, and {0:?} is not")]
    NotAbsolute(String),
    #[error("{0:?} does not exist")]
    Missing(String),
    #[error("{0:?} is a file, not a folder")]
    NotAFolder(String),
    /// The trap that has cost this project real damage twice.
    ///
    /// The default workspace is `~/KitchenTable` and macOS is case-insensitive,
    /// so on a machine holding the project at `~/kitchentable` they are the
    /// same directory. A daemon started there registers `docs/`, `crates/` and
    /// the repo itself as apps and writes an `app.json` into each - which is
    /// how a generated file once got swept into a commit.
    ///
    /// The general rule is worth more than that one case: every folder in the
    /// workspace becomes an app and gets a manifest written into it, so
    /// pointing it at a checkout means writing into somebody's source tree.
    #[error("{0:?} is a git repository, and every folder inside a workspace gets an app.json written into it")]
    IsRepository(String),
    #[error("{0:?} is inside Kitchen Table's own data folder")]
    InsideStateDir(String),
    #[error("{0:?} cannot be read: {1}")]
    Unreadable(String, String),
}

/// Check a folder the owner picked, and return the path to store.
///
/// The returned path is canonical: symlinks resolved and `..` removed, so the
/// value written to the settings table is the one the watcher will actually
/// see, and two spellings of the same folder cannot disagree.
///
/// `state_dir` is Kitchen Table's own directory. Watching it would have the
/// daemon serving its own database and announcing `storage.local` on the
/// network, and the first rescan would write manifests among the files it
/// depends on.
pub fn validate(picked: &Path, state_dir: &Path) -> Result<PathBuf, WorkspaceError> {
    let shown = || picked.display().to_string();

    if !picked.is_absolute() {
        return Err(WorkspaceError::NotAbsolute(shown()));
    }
    if !picked.exists() {
        return Err(WorkspaceError::Missing(shown()));
    }
    if !picked.is_dir() {
        return Err(WorkspaceError::NotAFolder(shown()));
    }

    // Canonicalise before the containment check: `~/Documents/../Documents` and
    // a symlink into the state directory both have to be resolved before two
    // paths can honestly be compared.
    let path = picked
        .canonicalize()
        .map_err(|e| WorkspaceError::Unreadable(shown(), e.to_string()))?;

    // Reading it once is also the permission check. A folder macOS will not let
    // this process into - Desktop and Documents both need consent - fails here
    // with the real reason rather than looking like an empty workspace.
    std::fs::read_dir(&path).map_err(|e| WorkspaceError::Unreadable(shown(), e.to_string()))?;

    if path.join(".git").exists() {
        return Err(WorkspaceError::IsRepository(shown()));
    }

    // The state directory may not exist yet on a first run, in which case
    // nothing can be inside it.
    if let Ok(state) = state_dir.canonicalize() {
        if path == state || path.starts_with(&state) {
            return Err(WorkspaceError::InsideStateDir(shown()));
        }
    }

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory with a counter in the name.
    ///
    /// Two tests sharing one path pass alone and fail together, which is the
    /// flake shape this repo has already been bitten by.
    fn scratch(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("kt-ws-{label}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("creates");
        dir
    }

    #[test]
    fn an_ordinary_folder_is_fine() {
        let dir = scratch("ok");
        let state = scratch("state");
        assert_eq!(
            validate(&dir, &state).expect("accepted"),
            dir.canonicalize().expect("canonical")
        );
    }

    #[test]
    fn a_repository_is_refused_by_name() {
        // The whole reason this function exists. Every folder in a workspace
        // gets an app.json written into it, so a checkout is the one folder
        // that must never be one.
        let dir = scratch("repo");
        std::fs::create_dir_all(dir.join(".git")).expect("creates");

        assert_eq!(
            validate(&dir, &scratch("state")),
            Err(WorkspaceError::IsRepository(dir.display().to_string()))
        );
    }

    #[test]
    fn a_repository_is_still_refused_when_git_is_a_file() {
        // A worktree or a submodule has `.git` as a file pointing elsewhere.
        // `is_dir()` would have let both through.
        let dir = scratch("worktree");
        std::fs::write(dir.join(".git"), "gitdir: /elsewhere").expect("writes");

        assert!(matches!(
            validate(&dir, &scratch("state")),
            Err(WorkspaceError::IsRepository(_))
        ));
    }

    #[test]
    fn the_state_directory_is_refused() {
        let state = scratch("state-self");
        assert!(matches!(
            validate(&state, &state),
            Err(WorkspaceError::InsideStateDir(_))
        ));
    }

    #[test]
    fn a_folder_inside_the_state_directory_is_refused() {
        let state = scratch("state-parent");
        let inside = state.join("storage");
        std::fs::create_dir_all(&inside).expect("creates");

        assert!(matches!(
            validate(&inside, &state),
            Err(WorkspaceError::InsideStateDir(_))
        ));
    }

    #[test]
    fn a_symlink_into_the_state_directory_is_refused() {
        // Canonicalising first is what catches this. Comparing the picked path
        // would compare a name that says nothing about where it points.
        #[cfg(unix)]
        {
            let state = scratch("state-link");
            let inside = state.join("real");
            std::fs::create_dir_all(&inside).expect("creates");
            let link = scratch("link-holder").join("looks-innocent");
            std::os::unix::fs::symlink(&inside, &link).expect("links");

            assert!(matches!(
                validate(&link, &state),
                Err(WorkspaceError::InsideStateDir(_))
            ));
        }
    }

    #[test]
    fn a_file_is_not_a_workspace() {
        let dir = scratch("file");
        let file = dir.join("notes.txt");
        std::fs::write(&file, "hello").expect("writes");

        assert!(matches!(
            validate(&file, &scratch("state")),
            Err(WorkspaceError::NotAFolder(_))
        ));
    }

    #[test]
    fn a_missing_folder_says_so_rather_than_being_created() {
        // The picker only returns folders that exist, so a missing one means
        // something else went wrong and creating it silently would hide it.
        let missing = scratch("missing").join("nope");

        assert!(matches!(
            validate(&missing, &scratch("state")),
            Err(WorkspaceError::Missing(_))
        ));
    }

    #[test]
    fn a_relative_path_is_refused() {
        assert!(matches!(
            validate(Path::new("Documents/apps"), &scratch("state")),
            Err(WorkspaceError::NotAbsolute(_))
        ));
    }
}
