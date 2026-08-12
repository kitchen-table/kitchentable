//! Spawning and supervising kt-daemon.
//!
//! The daemon never depends on the shell: closing the window changes nothing
//! about serving, and a daemon that is already healthy is adopted rather than
//! replaced. The shell only ever starts one that is missing, and restarts one
//! that died (docs/architecture.md section 3).

use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::Duration;

use crate::socket::{self, Handshake};

/// Restart backoff. Climbs quickly then stops climbing, so a daemon crashing
/// on startup does not spin the CPU but a transient failure still recovers
/// fast.
const BACKOFF: &[Duration] = &[
    Duration::from_millis(200),
    Duration::from_millis(500),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(15),
];

/// Give up announcing recovery after this many consecutive failures and let
/// the UI show a persistent error instead of restarting forever.
const MAX_ATTEMPTS: usize = 8;

pub struct Supervisor {
    child: Mutex<Option<Child>>,
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl Supervisor {
    pub fn new() -> Self {
        Self {
            child: Mutex::new(None),
        }
    }

    /// Make sure a healthy daemon is running, spawning one if not.
    ///
    /// Returns whether we ended up starting it, which onboarding uses to decide
    /// whether this is a first run.
    pub fn ensure_running(&self) -> Result<bool, DaemonError> {
        let socket = socket::socket_path();

        match socket::handshake(&socket) {
            // Someone else's daemon, or ours from a previous run. Either way it
            // works; adopt it.
            Handshake::Ready => return Ok(false),
            Handshake::WrongVersion { theirs, ours } => {
                tracing::warn!(
                    theirs,
                    ours,
                    "daemon speaks a different protocol; restarting it"
                );
                self.stop();
            }
            Handshake::Unhealthy(reason) => {
                tracing::warn!(reason, "daemon is unhealthy; restarting it");
                self.stop();
            }
            Handshake::NotRunning => {}
        }

        self.spawn()?;

        // Wait for it to come up rather than returning into a UI that will
        // immediately fail its first call.
        for delay in BACKOFF.iter().take(MAX_ATTEMPTS) {
            std::thread::sleep(*delay);
            if socket::handshake(&socket) == Handshake::Ready {
                tracing::info!("daemon is up");
                return Ok(true);
            }
        }

        Err(DaemonError::DidNotStart)
    }

    fn spawn(&self) -> Result<(), DaemonError> {
        let binary = daemon_binary()?;
        tracing::info!(path = %binary.display(), "starting the daemon");

        let child = Command::new(&binary)
            .spawn()
            .map_err(|source| DaemonError::Spawn {
                path: binary.display().to_string(),
                source,
            })?;

        *self.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
        Ok(())
    }

    /// Stop a daemon we started. One we adopted is left alone: it may be
    /// serving for a terminal session the user still wants.
    pub fn stop(&self) {
        let mut guard = self.child.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut child) = guard.take() {
            tracing::info!("stopping the daemon");
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Whether the daemon we spawned is still alive.
    pub fn is_alive(&self) -> bool {
        let mut guard = self.child.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("could not find the kt-daemon binary; looked next to the app and on PATH")]
    NotFound,
    #[error("could not start {path}: {source}")]
    Spawn {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("the daemon started but never became healthy")]
    DidNotStart,
}

/// Where kt-daemon lives.
///
/// Bundled next to the shell binary in a release build; in development it is
/// the workspace target directory. `KT_DAEMON` overrides both, which is how a
/// test points the shell at a purpose-built daemon.
fn daemon_binary() -> Result<PathBuf, DaemonError> {
    if let Ok(explicit) = std::env::var("KT_DAEMON") {
        let path = PathBuf::from(explicit);
        if path.exists() {
            return Ok(path);
        }
        tracing::warn!(path = %path.display(), "KT_DAEMON is set but not there");
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Bundled: Contents/MacOS/kt-daemon, beside the shell.
            let sibling = dir.join("kt-daemon");
            if sibling.exists() {
                return Ok(sibling);
            }
            // Development: shell/src-tauri/target/debug/ next to the workspace
            // target/debug/ that holds kt-daemon.
            for up in [3, 4, 5] {
                let mut candidate = dir.to_path_buf();
                for _ in 0..up {
                    candidate = match candidate.parent() {
                        Some(p) => p.to_path_buf(),
                        None => break,
                    };
                }
                for profile in ["debug", "release"] {
                    let dev = candidate.join("target").join(profile).join("kt-daemon");
                    if dev.exists() {
                        return Ok(dev);
                    }
                }
            }
        }
    }

    Err(DaemonError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_path_that_does_not_exist_is_not_used() {
        // Safety: single-threaded test, and the variable is removed after.
        unsafe { std::env::set_var("KT_DAEMON", "/nonexistent/kt-daemon") };
        let found = daemon_binary();
        unsafe { std::env::remove_var("KT_DAEMON") };

        // Either it fell through to a real binary in the dev tree, or it found
        // nothing - but never the path that does not exist.
        if let Ok(path) = found {
            assert_ne!(path, PathBuf::from("/nonexistent/kt-daemon"));
        }
    }

    #[test]
    fn a_supervisor_with_no_child_reports_nothing_alive() {
        let supervisor = Supervisor::new();
        assert!(!supervisor.is_alive());
        supervisor.stop(); // must not panic with nothing to stop
    }

    #[test]
    fn backoff_climbs_and_then_settles() {
        assert!(BACKOFF.windows(2).all(|w| w[0] <= w[1]));
        assert!(BACKOFF.last().expect("non-empty") <= &Duration::from_secs(30));
    }
}
