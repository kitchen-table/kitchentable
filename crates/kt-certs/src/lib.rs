//! Local CA, leaf certs, keychain trust behind a per-OS trait.
//!
//! Only the keystore half exists so far. It is here rather than in kt-auth
//! because the CA's private key needs exactly the same drawer, and because
//! keeping every `cfg(target_os)` in this crate is what stops platform code
//! leaking into the security core (CLAUDE.md rule 7).

mod secret;

pub use secret::{FileStore, MemoryStore, SecretError, SecretStore};

#[cfg(target_os = "macos")]
mod keychain;
#[cfg(target_os = "macos")]
pub use keychain::Keychain;

/// The keystore for this platform.
///
/// macOS gets the Keychain. Everywhere else gets files, which is honest rather
/// than ideal: Linux and Windows keystores land with their platform ports, and
/// until then a 0600 file next to the database is no weaker than the database,
/// which already holds every device and invite in plaintext.
pub fn for_this_platform(state_dir: &std::path::Path) -> Box<dyn SecretStore> {
    #[cfg(target_os = "macos")]
    {
        let _ = state_dir;
        Box::new(Keychain::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(FileStore::new(state_dir))
    }
}
