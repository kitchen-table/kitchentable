//! Somewhere to keep bytes that must outlive a restart and must not be read
//! by anything else on the machine.
//!
//! The session-signing key is the first tenant; the local CA's private key is
//! the one this was really built for. Both are small, both are catastrophic to
//! leak and merely inconvenient to lose, which is why the trait deals in
//! opaque byte strings and has no opinion about what is in them.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("the OS keystore refused: {0}")]
    Keystore(String),
    #[error("could not read or write the secret: {0}")]
    File(#[from] std::io::Error),
    #[error("{0:?} is not a usable secret name")]
    BadName(String),
}

/// Persists small secrets across restarts.
///
/// One implementation per platform, plus [`MemoryStore`] for tests. The daemon
/// holds a `dyn SecretStore` so nothing above this crate has a
/// `cfg(target_os)` in it.
pub trait SecretStore: Send + Sync {
    /// The secret stored under `name`, or `None` if there is not one yet.
    ///
    /// A missing secret is `Ok(None)`, not an error: first run is the normal
    /// case, and callers should not have to tell it apart from a real failure.
    fn get(&self, name: &str) -> Result<Option<Vec<u8>>, SecretError>;

    /// Store `secret` under `name`, replacing whatever was there.
    fn set(&self, name: &str, secret: &[u8]) -> Result<(), SecretError>;

    /// Where this implementation keeps things, for the startup log. Used to
    /// tell "saved in the Keychain" apart from "saved in a file because the
    /// Keychain was unavailable", which is the difference between a normal run
    /// and one worth asking about.
    fn describe(&self) -> &'static str;
}

/// Rejects anything that could escape the directory a [`FileStore`] owns.
///
/// Names are compile-time constants today, so this is belt and braces - but it
/// is the kind of belt that costs four lines and saves a CVE.
fn check_name(name: &str) -> Result<(), SecretError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(SecretError::BadName(name.to_string()))
    }
}

/// Secrets as files, one per secret, mode 0600 in a 0700 directory.
///
/// The portable implementation: it is what Linux and Windows get until they
/// have a keystore wired up, and what the test suite uses so a `cargo test`
/// never touches the developer's real Keychain.
pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    /// `state_dir` is the daemon's private directory; secrets go in a
    /// subdirectory of it so a stray `ls` of the state directory does not put
    /// key material next to the database.
    pub fn new(state_dir: &Path) -> Self {
        Self {
            dir: state_dir.join("secrets"),
        }
    }

    fn path(&self, name: &str) -> Result<PathBuf, SecretError> {
        check_name(name)?;
        Ok(self.dir.join(format!("{name}.key")))
    }
}

impl SecretStore for FileStore {
    fn get(&self, name: &str) -> Result<Option<Vec<u8>>, SecretError> {
        match std::fs::read(self.path(name)?) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn set(&self, name: &str, secret: &[u8]) -> Result<(), SecretError> {
        use std::io::Write;
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

        let path = self.path(name)?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)?;

        // Written to a neighbouring file and renamed, so a crash midway leaves
        // the old key intact rather than a truncated one. A half-written key
        // is indistinguishable from a corrupt one and costs everybody a
        // re-pair; an untouched old key costs nothing.
        let temp = path.with_extension("key.tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(secret)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, &path)?;
        Ok(())
    }

    fn describe(&self) -> &'static str {
        "a file in the state directory"
    }
}

/// Keeps secrets for as long as the process lives. Tests only.
#[derive(Default)]
pub struct MemoryStore {
    items: Mutex<Vec<(String, Vec<u8>)>>,
    fail: Mutex<Option<String>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Make every operation fail, for testing the degraded path - a locked
    /// Keychain, or a user who clicked Deny.
    pub fn failing(reason: impl Into<String>) -> Self {
        let store = Self::default();
        *store.fail.lock().expect("not poisoned") = Some(reason.into());
        store
    }

    fn refuse(&self) -> Result<(), SecretError> {
        match self.fail.lock().expect("not poisoned").clone() {
            Some(reason) => Err(SecretError::Keystore(reason)),
            None => Ok(()),
        }
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> Result<Option<Vec<u8>>, SecretError> {
        self.refuse()?;
        check_name(name)?;
        Ok(self
            .items
            .lock()
            .expect("not poisoned")
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone()))
    }

    fn set(&self, name: &str, secret: &[u8]) -> Result<(), SecretError> {
        self.refuse()?;
        check_name(name)?;
        let mut items = self.items.lock().expect("not poisoned");
        match items.iter_mut().find(|(k, _)| k == name) {
            Some((_, v)) => *v = secret.to_vec(),
            None => items.push((name.to_string(), secret.to_vec())),
        }
        Ok(())
    }

    fn describe(&self) -> &'static str {
        "memory"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same contract, run against every implementation that does not need
    /// a real keystore. A store that passes this is a store the daemon can use.
    fn behaves_like_a_store(store: &dyn SecretStore) {
        assert!(
            store.get("session-key").expect("reads").is_none(),
            "nothing is stored before anything is stored"
        );

        store.set("session-key", &[7u8; 32]).expect("writes");
        assert_eq!(
            store.get("session-key").expect("reads"),
            Some(vec![7u8; 32])
        );

        // Rotation has to work, or a corrupt key can never be replaced.
        store.set("session-key", &[9u8; 32]).expect("overwrites");
        assert_eq!(
            store.get("session-key").expect("reads"),
            Some(vec![9u8; 32])
        );

        // Two secrets do not collide: the CA key lands next to this one.
        store.set("ca-key", &[1u8; 8]).expect("writes");
        assert_eq!(
            store.get("session-key").expect("reads"),
            Some(vec![9u8; 32])
        );
    }

    #[test]
    fn a_file_store_behaves_like_a_store() {
        let dir = scratch_dir("file-store");
        behaves_like_a_store(&FileStore::new(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_memory_store_behaves_like_a_store() {
        behaves_like_a_store(&MemoryStore::new());
    }

    #[test]
    fn a_secret_file_is_not_readable_by_anyone_else() {
        use std::os::unix::fs::PermissionsExt;

        let dir = scratch_dir("modes");
        let store = FileStore::new(&dir);
        store.set("session-key", &[3u8; 32]).expect("writes");

        let file = std::fs::metadata(dir.join("secrets").join("session-key.key"))
            .expect("the key file exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file, 0o600, "the key file is readable by others");

        let folder = std::fs::metadata(dir.join("secrets"))
            .expect("the directory exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(folder, 0o700, "the secrets directory is listable by others");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_that_would_escape_the_directory_is_refused() {
        let dir = scratch_dir("names");
        let store = FileStore::new(&dir);

        for bad in ["../escape", "a/b", "", "UPPER", "with space", "dot.dot"] {
            assert!(
                store.set(bad, &[0u8; 4]).is_err(),
                "{bad:?} should not be a usable name"
            );
            assert!(store.get(bad).is_err(), "{bad:?} should not be readable");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refusing_store_reports_it_rather_than_pretending_to_be_empty() {
        // The distinction the daemon acts on: "nothing saved yet" means write
        // one, "the keystore said no" means do not keep asking.
        let store = MemoryStore::failing("the keychain is locked");
        assert!(store.get("session-key").is_err());
        assert!(store.set("session-key", &[0u8; 32]).is_err());
    }

    /// `/tmp` rather than `std::env::temp_dir()` for the same reason the e2e
    /// suite uses it: short paths, and no `/var/folders` surprises.
    fn scratch_dir(what: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEQ: AtomicU32 = AtomicU32::new(0);

        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            PathBuf::from("/tmp").join(format!("kt-secret-{what}-{}-{seq}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }
}
