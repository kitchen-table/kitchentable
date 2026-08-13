//! The session-signing key, and where it lives between runs.
//!
//! kt-certs owns the drawer, kt-auth owns the cryptography, and the policy for
//! what to do when the drawer is stuck lives here - which is the daemon's job,
//! because it is the only layer that knows starting up anyway is better than
//! not starting at all.

use std::sync::Arc;
use std::time::Duration;

use kt_auth::SessionKeys;
use kt_certs::SecretStore;

/// The name the key is filed under. Stable: changing it strands the old entry
/// and re-pairs every viewer in the house.
const SESSION_KEY: &str = "session-key";

/// Ed25519 seeds are 32 bytes; `SessionKeys::from_bytes` takes exactly that.
const KEY_LEN: usize = 32;

/// How long the keystore gets to answer before the daemon goes on without it.
///
/// A local keystore answers in milliseconds when it answers at all, so this is
/// not a performance budget - it is a liveness one. kt-certs refuses keychain
/// dialogs precisely so this cannot happen, and this is the second lock on the
/// same door: nothing about fetching a key is worth not serving over.
const KEYSTORE_BUDGET: Duration = Duration::from_secs(5);

/// Load the session-signing key, creating and saving one on first run.
///
/// Never fatal, and never slow, on purpose. Every failure degrades to a fresh
/// in-memory key, which is exactly the behaviour that shipped before this
/// existed: serving keeps working and viewers pair again. Refusing to start -
/// or worse, hanging - because a keychain is unhappy would turn a re-pair into
/// an outage.
pub fn load_or_create(secrets: Arc<dyn SecretStore>) -> Arc<SessionKeys> {
    load_or_create_within(secrets, KEYSTORE_BUDGET)
}

fn load_or_create_within(secrets: Arc<dyn SecretStore>, budget: Duration) -> Arc<SessionKeys> {
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = Arc::clone(&secrets);
    // Deliberately never joined. If the OS has parked this thread inside a
    // keychain call, it will still be parked at the deadline, and waiting on
    // it is the exact failure the deadline exists to avoid. It holds nothing
    // the daemon needs.
    std::thread::spawn(move || {
        let _ = tx.send(resolve(worker.as_ref()));
    });

    match rx.recv_timeout(budget) {
        Ok(keys) => keys,
        Err(_) => {
            tracing::warn!(
                store = secrets.describe(),
                "the keystore did not answer in time; serving with a key that \
                 lasts until the next restart"
            );
            Arc::new(SessionKeys::generate())
        }
    }
}

/// The part that talks to the keystore, and may block for as long as it likes.
fn resolve(secrets: &dyn SecretStore) -> Arc<SessionKeys> {
    let stored = match secrets.get(SESSION_KEY) {
        Ok(stored) => stored,
        Err(e) => {
            tracing::warn!(
                store = secrets.describe(),
                "could not read the session key ({e}); \
                 viewers will have to pair again after this restart"
            );
            return Arc::new(SessionKeys::generate());
        }
    };

    match stored {
        Some(bytes) => match <[u8; KEY_LEN]>::try_from(bytes.as_slice()) {
            Ok(seed) => {
                tracing::info!(
                    store = secrets.describe(),
                    "session key loaded; existing sessions still work"
                );
                return Arc::new(SessionKeys::from_bytes(&seed));
            }
            Err(_) => tracing::warn!(
                store = secrets.describe(),
                found = bytes.len(),
                expected = KEY_LEN,
                "the stored session key is the wrong size; replacing it"
            ),
        },
        None => tracing::info!(store = secrets.describe(), "no session key yet; making one"),
    }

    let keys = SessionKeys::generate();
    match secrets.set(SESSION_KEY, &keys.to_bytes()) {
        Ok(()) => tracing::info!(
            store = secrets.describe(),
            "session key saved; sessions will survive a restart"
        ),
        Err(e) => tracing::warn!(
            store = secrets.describe(),
            "could not save the session key ({e}); \
             viewers will have to pair again after the next restart"
        ),
    }
    Arc::new(keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kt_auth::DeviceId;
    use kt_certs::{MemoryStore, SecretError};

    const NOW: i64 = 1_700_000_000;

    /// A keystore that never answers, the way a keychain waiting on a dialog
    /// nobody can click never answers.
    struct Wedged;

    impl SecretStore for Wedged {
        fn get(&self, _: &str) -> Result<Option<Vec<u8>>, SecretError> {
            std::thread::sleep(Duration::from_secs(60));
            unreachable!("the deadline should have fired long before this")
        }

        fn set(&self, _: &str, _: &[u8]) -> Result<(), SecretError> {
            std::thread::sleep(Duration::from_secs(60));
            unreachable!("the deadline should have fired long before this")
        }

        fn describe(&self) -> &'static str {
            "a keystore that never answers"
        }
    }

    #[test]
    fn the_second_run_reuses_the_first_runs_key() {
        // The whole point: a cookie minted before a restart still verifies
        // after one.
        let store = Arc::new(MemoryStore::new());
        let device = DeviceId::generate();

        let cookie = load_or_create(store.clone()).mint(&device, NOW);
        let after_restart = load_or_create(store);

        let token = after_restart
            .verify(&cookie, NOW)
            .expect("the cookie survives a restart");
        assert_eq!(token.device, device);
    }

    #[test]
    fn the_first_run_saves_a_key() {
        let store = Arc::new(MemoryStore::new());
        let keys = load_or_create(store.clone());

        let saved = store.get(SESSION_KEY).expect("reads").expect("was saved");
        assert_eq!(saved.len(), KEY_LEN);
        assert_eq!(saved, keys.to_bytes(), "saved the key it is actually using");
    }

    #[test]
    fn a_keystore_that_never_answers_does_not_hold_up_the_daemon() {
        // Found by running it: SecKeychainAddGenericPassword blocked in
        // AuthorizationCopyRights waiting for a dialog, and startup never
        // reached the point of binding the HTTP port. kt-certs refuses those
        // dialogs now; this is the backstop for whatever the next one is.
        let started = std::time::Instant::now();
        let keys = load_or_create_within(Arc::new(Wedged), Duration::from_millis(150));

        assert!(
            started.elapsed() < Duration::from_secs(5),
            "startup waited {:?} on the keystore",
            started.elapsed()
        );
        let device = DeviceId::generate();
        let cookie = keys.mint(&device, NOW);
        assert!(
            keys.verify(&cookie, NOW).is_ok(),
            "and it still serves, with a key that lasts this run"
        );
    }

    #[test]
    fn a_keystore_that_refuses_still_yields_a_working_daemon() {
        // A locked keychain is a re-pair, not an outage.
        let store = Arc::new(MemoryStore::failing("the keychain is locked"));
        let device = DeviceId::generate();

        let keys = load_or_create(store.clone());
        let cookie = keys.mint(&device, NOW);
        assert!(
            keys.verify(&cookie, NOW).is_ok(),
            "still mints and verifies"
        );

        // And it really is ephemeral, rather than silently shared.
        let next = load_or_create(store);
        assert!(
            next.verify(&cookie, NOW).is_err(),
            "a key that could not be saved cannot come back"
        );
    }

    #[test]
    fn a_corrupt_entry_is_replaced_rather_than_fatal() {
        let store = Arc::new(MemoryStore::new());
        store.set(SESSION_KEY, b"too short").expect("writes");

        let keys = load_or_create(store.clone());

        let saved = store.get(SESSION_KEY).expect("reads").expect("still there");
        assert_eq!(saved.len(), KEY_LEN, "the bad entry was overwritten");
        assert_eq!(saved, keys.to_bytes());

        // And the replacement persists, so this happens once rather than
        // every start.
        let device = DeviceId::generate();
        let cookie = keys.mint(&device, NOW);
        assert!(load_or_create(store).verify(&cookie, NOW).is_ok());
    }
}
