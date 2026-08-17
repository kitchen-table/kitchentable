//! The sample app, written into the workspace on first run.
//!
//! Onboarding's fourth step says "we put a little Welcome app in your workspace
//! and started serving it", and until this existed it did not: the step showed
//! whatever folder happened to be first in the workspace, which on a machine
//! whose workspace is a code directory meant a folder with no `index.html`, a
//! QR code pointing at it, and a 404 for anybody who scanned it.
//!
//! It is also what makes the step after it possible. Pairing cannot be taught
//! without something to open, and the phone that opens it is the whole two
//! minutes the product is sold on.
//!
//! # Writing into somebody's folder
//!
//! This is the only thing Kitchen Table writes into a workspace uninvited, so
//! it is hedged three ways:
//!
//! - **Once, ever.** A settings flag records that we have done it. Delete the
//!   app and it stays deleted; that is an answer, not an accident to correct.
//! - **Never over anything.** If a `Welcome` folder is already there - theirs,
//!   or ours from a previous install whose database has since been lost - it is
//!   left exactly as it is.
//! - **Never fatal.** A read-only workspace, a full disk, a permission the OS
//!   has not granted yet: all of them mean no sample app, which is a smaller
//!   problem than a daemon that will not start.

use std::path::Path;

use kt_store::Store;

/// Set once the sample app has been offered. Its presence is what stops this
/// happening twice, so it is written whether or not the write succeeded.
const SEEDED_SETTING: &str = "welcome_seeded";

/// The folder the app is created in, and the name onboarding shows.
const FOLDER: &str = "Welcome";

/// Baked into the binary rather than read from `examples/` at runtime: the
/// daemon in a shipped bundle has no repository to read from.
const INDEX: &str = include_str!("../../../examples/welcome/index.html");
const MANIFEST: &str = include_str!("../../../examples/welcome/app.json");

/// Create the sample app, if this install has never had one.
///
/// Called before the first scan so the app is already there when the registry
/// looks, which is what lets onboarding show it without waiting for a watcher
/// to settle.
pub fn seed(workspace: &Path, store: &Store) {
    // The end-to-end fixtures assert on a workspace with nothing in it, and a
    // sample app appearing in every one of them would be an app they never put
    // there. Same escape hatch as `KT_NO_MDNS`.
    if std::env::var_os("KT_NO_SAMPLE").is_some() {
        return;
    }

    match store.setting(SEEDED_SETTING) {
        Ok(Some(_)) => return,
        Ok(None) => {}
        Err(e) => {
            // Without a readable settings table there is no way to know whether
            // this has been done before, and doing it twice is worse than not
            // doing it: it would put a folder back that somebody deleted.
            tracing::warn!(error = %e, "cannot tell whether the sample app was created; skipping");
            return;
        }
    }

    let folder = workspace.join(FOLDER);
    if folder.exists() {
        tracing::info!(path = %folder.display(), "a Welcome folder is already there; leaving it alone");
    } else if let Err(e) = write(&folder) {
        tracing::warn!(error = %e, path = %folder.display(), "could not create the sample app");
    } else {
        tracing::info!(path = %folder.display(), "created the sample app");
    }

    // Marked even when the write failed. A workspace that could not be written
    // to this time will usually fail the same way next time, and retrying on
    // every start would mean a folder appearing days later, long after the
    // onboarding that explained it.
    if let Err(e) = store.set_setting(SEEDED_SETTING, "1") {
        tracing::warn!(error = %e, "could not record that the sample app was created");
    }
}

fn write(folder: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    std::fs::write(folder.join("index.html"), INDEX)?;
    std::fs::write(folder.join("app.json"), MANIFEST)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::in_memory().expect("opens")
    }

    #[test]
    fn the_sample_app_is_created_on_a_first_run() {
        let workspace = tempfile::tempdir().expect("temp dir");
        let store = store();

        seed(workspace.path(), &store);

        let folder = workspace.path().join(FOLDER);
        assert!(folder.join("index.html").is_file(), "something to serve");
        assert!(folder.join("app.json").is_file(), "and a manifest");
    }

    #[test]
    fn it_does_not_come_back_once_it_is_deleted() {
        // Deleting the sample app is an answer, not a mistake to correct.
        let workspace = tempfile::tempdir().expect("temp dir");
        let store = store();

        seed(workspace.path(), &store);
        std::fs::remove_dir_all(workspace.path().join(FOLDER)).expect("removes");
        seed(workspace.path(), &store);

        assert!(!workspace.path().join(FOLDER).exists());
    }

    #[test]
    fn it_never_writes_over_a_folder_that_is_already_there() {
        let workspace = tempfile::tempdir().expect("temp dir");
        let store = store();
        let folder = workspace.path().join(FOLDER);
        std::fs::create_dir_all(&folder).expect("creates");
        std::fs::write(folder.join("index.html"), "mine").expect("writes");

        seed(workspace.path(), &store);

        assert_eq!(
            std::fs::read_to_string(folder.join("index.html")).expect("reads"),
            "mine",
            "somebody else's file, left alone",
        );
    }

    #[test]
    fn a_workspace_that_cannot_be_written_to_is_not_fatal() {
        // No panic, no error out: an install with nowhere to put the sample app
        // still has a daemon.
        let store = store();
        seed(Path::new("/nonexistent/nowhere"), &store);

        assert_eq!(
            store.setting(SEEDED_SETTING).expect("reads").as_deref(),
            Some("1")
        );
    }

    #[test]
    fn the_bundled_app_is_a_real_page_with_a_manifest_that_parses() {
        assert!(
            INDEX.contains("<!doctype html>"),
            "something a browser renders"
        );

        let manifest: serde_json::Value = serde_json::from_str(MANIFEST).expect("parses");
        assert_eq!(manifest["name"], "Welcome");
        // Invited, not the usual Private default: the step after this one is
        // the phone opening it and the owner approving the device, and Private
        // refuses an unknown device outright rather than asking.
        assert_eq!(manifest["visibility"], "invited");
    }
}
