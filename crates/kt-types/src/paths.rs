//! Where Kitchen Table keeps things.
//!
//! Shared because the daemon creates these paths and the CLI has to find them
//! (docs/architecture.md section 7). Every path is derived from `$HOME` so a
//! test can point the whole daemon somewhere else.

use std::path::PathBuf;

/// The user-visible workspace: drop a folder in here and it becomes an app.
pub fn default_workspace(home: &str) -> PathBuf {
    PathBuf::from(home).join("KitchenTable")
}

/// Private state: the socket, the databases, certificates, logs.
pub fn state_dir(home: &str) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("KitchenTable")
    }
    #[cfg(not(target_os = "macos"))]
    {
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("kitchentable")
    }
}

/// The JSON-RPC socket. Mode 0600: possession of it is the authorisation.
pub fn socket_path(home: &str) -> PathBuf {
    state_dir(home).join("kt.sock")
}

/// Guards against two daemons owning one workspace.
pub fn lock_path(home: &str) -> PathBuf {
    state_dir(home).join("kt.lock")
}

/// Apps, devices, invites, access log, deploys, settings.
pub fn system_db_path(home: &str) -> PathBuf {
    state_dir(home).join("system.db")
}

/// Where each app's own data lives, one database per app.
///
/// A directory rather than more tables in `system.db`: an app's data is the
/// app's, and keeping it in its own file is what makes "one app can never read
/// another's" a property of the filesystem rather than of every query.
pub fn storage_dir(home: &str) -> PathBuf {
    state_dir(home).join("storage")
}

/// Reads `$HOME`, which every supported platform sets.
pub fn home() -> Result<String, std::env::VarError> {
    std::env::var("HOME")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_private_lives_under_one_state_directory() {
        let dir = state_dir("/Users/ada");
        for p in [
            socket_path("/Users/ada"),
            lock_path("/Users/ada"),
            system_db_path("/Users/ada"),
        ] {
            assert!(p.starts_with(&dir), "{p:?} escaped {dir:?}");
        }
    }

    #[test]
    fn the_workspace_is_not_inside_the_state_directory() {
        // It is the user's folder, and deliberately not under Desktop,
        // Documents or Downloads, which trigger extra macOS prompts.
        let ws = default_workspace("/Users/ada");
        assert!(!ws.starts_with(state_dir("/Users/ada")));
        assert_eq!(ws, PathBuf::from("/Users/ada/KitchenTable"));
    }
}
