//! The daemon's two long-lived keys, and where they live between runs.
//!
//! kt-certs owns the drawer, kt-auth and kt-tunnel-proto own the cryptography,
//! and the policy for what to do when the drawer is stuck lives here - which is
//! the daemon's job, because it is the only layer that knows starting up anyway
//! is better than not starting at all.
//!
//! # Two keys, and deliberately opposite answers when the keystore fails
//!
//! The **session key** signs viewer cookies. If it cannot be kept, the daemon
//! makes a fresh one and carries on: the cost is that everyone pairs again,
//! which is a nuisance and not a loss, and refusing to serve over it would turn
//! a nuisance into an outage.
//!
//! The **install key** is this machine's identity. It is what the relay knows
//! this install by and what an account is linked to, so a fresh one is not a
//! degraded version of the old one - it is a different machine as far as
//! everything upstream is concerned. Inventing one silently would unlink the
//! owner's account and give no sign of having done it.
//!
//! So when the keystore will not answer, the session key is replaced and the
//! install key is not. No identity this run is a state the daemon can report
//! honestly and recover from on the next start. A quietly different identity is
//! neither.

use std::sync::Arc;
use std::time::Duration;

use kt_auth::SessionKeys;
use kt_certs::SecretStore;
use kt_tunnel_proto::InstallIdentity;

/// The name the key is filed under. Stable: changing it strands the old entry
/// and re-pairs every viewer in the house.
const SESSION_KEY: &str = "session-key";

/// Where the install identity is filed. Changing this name orphans the entry
/// and, with it, whatever account this install was linked to.
const INSTALL_KEY: &str = "install-key";

/// Ed25519 seeds are 32 bytes; `SessionKeys::from_bytes` takes exactly that.
const KEY_LEN: usize = 32;

/// How long the keystore gets to answer before the daemon goes on without it.
///
/// A local keystore answers in milliseconds when it answers at all, so this is
/// not a performance budget - it is a liveness one. kt-certs refuses keychain
/// dialogs precisely so this cannot happen, and this is the second lock on the
/// same door: nothing about fetching a key is worth not serving over.
const KEYSTORE_BUDGET: Duration = Duration::from_secs(5);

/// Both long-lived keys, as they stand this run.
pub struct Keys {
    /// Always present. Serving cannot wait on a keystore.
    pub session: Arc<SessionKeys>,
    /// Absent when the keystore could not give one back and inventing one
    /// would have been worse than having none. See the module docs.
    pub install: Option<Arc<InstallIdentity>>,
}

/// Load both keys, making and saving whatever is missing.
///
/// Never fatal, and never slow, on purpose. Both keys are resolved in one visit
/// to the keystore under one deadline, so a wedged keychain costs the same five
/// seconds whether there is one key to fetch or ten.
pub fn load_or_create(secrets: Arc<dyn SecretStore>) -> Keys {
    load_or_create_within(secrets, KEYSTORE_BUDGET)
}

fn load_or_create_within(secrets: Arc<dyn SecretStore>, budget: Duration) -> Keys {
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = Arc::clone(&secrets);
    // Deliberately never joined. If the OS has parked this thread inside a
    // keychain call, it will still be parked at the deadline, and waiting on
    // it is the exact failure the deadline exists to avoid. It holds nothing
    // the daemon needs.
    std::thread::spawn(move || {
        let _ = tx.send((resolve(worker.as_ref()), resolve_install(worker.as_ref())));
    });

    match rx.recv_timeout(budget) {
        Ok(((session, _), install)) => Keys { session, install },
        Err(_) => {
            tracing::warn!(
                store = secrets.describe(),
                "the keystore did not answer in time; serving with a key that \
                 lasts until the next restart, and with no relay identity"
            );
            Keys {
                session: Arc::new(SessionKeys::generate()),
                install: None,
            }
        }
    }
}

/// The install key, which is replaced far more reluctantly than the session one.
///
/// The distinction that matters is between "there is no key" and "I could not
/// read the key". The first is a fact and the answer is to make one. The second
/// is an absence of information, and on this machine it is usually temporary -
/// a locked keychain, or a rebuilt binary whose ACL no longer matches. Treating
/// it as permission to generate a replacement would discard an identity that is
/// linked to somebody's account, on the strength of a failure that will very
/// likely have cleared by the next start.
///
/// So a read error means no identity this run and nothing written. The relay
/// stays off, the daemon says so, and the next start tries again.
fn resolve_install(secrets: &dyn SecretStore) -> Option<Arc<InstallIdentity>> {
    match secrets.get(INSTALL_KEY) {
        Ok(Some(bytes)) => match <[u8; KEY_LEN]>::try_from(bytes.as_slice()) {
            Ok(seed) => {
                let identity = InstallIdentity::from_bytes(&seed);
                tracing::info!(
                    store = secrets.describe(),
                    install = %identity.public(),
                    "install identity loaded"
                );
                Some(Arc::new(identity))
            }
            // Not readable as a key by anyone, so nothing is lost by replacing
            // it - the identity it stood for is already gone.
            Err(_) => {
                tracing::warn!(
                    store = secrets.describe(),
                    found = bytes.len(),
                    expected = KEY_LEN,
                    "the stored install identity is the wrong size; replacing it"
                );
                make_and_save_install(secrets)
            }
        },
        Ok(None) => {
            tracing::info!(
                store = secrets.describe(),
                "no install identity yet; making one"
            );
            make_and_save_install(secrets)
        }
        Err(e) => {
            tracing::warn!(
                store = secrets.describe(),
                "could not read the install identity ({e}); leaving it alone. \
                 The relay is unavailable this run; nothing else is affected"
            );
            None
        }
    }
}

fn make_and_save_install(secrets: &dyn SecretStore) -> Option<Arc<InstallIdentity>> {
    let identity = InstallIdentity::generate();

    if let Err(e) = secrets.set(INSTALL_KEY, &identity.to_bytes()) {
        tracing::warn!(
            store = secrets.describe(),
            "could not save the install identity ({e}); the relay is \
             unavailable this run"
        );
        return None;
    }

    // Read it back rather than trusting the write, for the reason set out in
    // make_and_save below - and with more at stake here, because an identity
    // that does not survive the restart is one the owner would link to an
    // account and find unlinked in the morning.
    match secrets.get(INSTALL_KEY) {
        Ok(Some(back)) if back.as_slice() == identity.to_bytes().as_slice() => {
            tracing::info!(
                store = secrets.describe(),
                install = %identity.public(),
                "install identity saved"
            );
            Some(Arc::new(identity))
        }
        other => {
            tracing::warn!(
                store = secrets.describe(),
                readable = matches!(other, Ok(Some(_))),
                "the keystore accepted the install identity but did not give it \
                 back; the relay is unavailable this run"
            );
            None
        }
    }
}

/// What became of the key. Returned so the outcome can be asserted on; the
/// reasons behind it are logged where they are discovered.
#[derive(Debug, PartialEq, Eq)]
enum Persistence {
    /// Came back from the keystore, so sessions minted before the restart
    /// still verify.
    Loaded,
    /// Newly made, saved, and read back to prove it.
    Saved,
    /// Newly made and not persisted. Serving is unaffected; the next restart
    /// asks everyone to pair again.
    Ephemeral,
}

/// The part that talks to the keystore, and may block for as long as it likes.
fn resolve(secrets: &dyn SecretStore) -> (Arc<SessionKeys>, Persistence) {
    match secrets.get(SESSION_KEY) {
        Ok(Some(bytes)) => match <[u8; KEY_LEN]>::try_from(bytes.as_slice()) {
            Ok(seed) => {
                tracing::info!(
                    store = secrets.describe(),
                    "session key loaded; existing sessions still work"
                );
                return (
                    Arc::new(SessionKeys::from_bytes(&seed)),
                    Persistence::Loaded,
                );
            }
            Err(_) => tracing::warn!(
                store = secrets.describe(),
                found = bytes.len(),
                expected = KEY_LEN,
                "the stored session key is the wrong size; replacing it"
            ),
        },
        Ok(None) => tracing::info!(store = secrets.describe(), "no session key yet; making one"),
        // Deliberately falls through to replacing it rather than giving up. An
        // entry we cannot read is worth no more than no entry at all, and
        // leaving it there means every future start degrades the same way.
        Err(e) => tracing::warn!(
            store = secrets.describe(),
            "could not read the stored session key ({e}); replacing it"
        ),
    }

    make_and_save(secrets)
}

fn make_and_save(secrets: &dyn SecretStore) -> (Arc<SessionKeys>, Persistence) {
    let keys = Arc::new(SessionKeys::generate());
    let seed = keys.to_bytes();

    if let Err(e) = secrets.set(SESSION_KEY, &seed) {
        tracing::warn!(
            store = secrets.describe(),
            "could not save the session key ({e}); \
             viewers will have to pair again after the next restart"
        );
        return (keys, Persistence::Ephemeral);
    }

    // Read it back rather than trusting the write. A keychain item carries its
    // own access control list, so a write can be accepted and the result still
    // be unreadable by the very process that wrote it - which is how this
    // arrived at claiming "sessions will survive a restart" when they did not.
    // The only honest evidence of persistence is getting the key back.
    match secrets.get(SESSION_KEY) {
        Ok(Some(back)) if back.as_slice() == seed.as_slice() => {
            tracing::info!(
                store = secrets.describe(),
                "session key saved; sessions will survive a restart"
            );
            (keys, Persistence::Saved)
        }
        other => {
            tracing::warn!(
                store = secrets.describe(),
                readable = matches!(other, Ok(Some(_))),
                "the keystore accepted the session key but did not give it back; \
                 viewers will have to pair again after the next restart"
            );
            (keys, Persistence::Ephemeral)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kt_auth::DeviceId;
    use kt_certs::{MemoryStore, SecretError};

    const NOW: i64 = 1_700_000_000;

    /// A keystore that takes a write and then will not give it back, the way a
    /// keychain item whose ACL names a different binary does.
    struct WriteOnly;

    impl SecretStore for WriteOnly {
        fn get(&self, _: &str) -> Result<Option<Vec<u8>>, SecretError> {
            Err(SecretError::Keystore("authentication failed".into()))
        }

        fn set(&self, _: &str, _: &[u8]) -> Result<(), SecretError> {
            Ok(())
        }

        fn describe(&self) -> &'static str {
            "a keystore that accepts writes it will not return"
        }
    }

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

        let cookie = load_or_create(store.clone()).session.mint(&device, NOW);
        let after_restart = load_or_create(store).session;

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
        assert_eq!(
            saved,
            keys.session.to_bytes(),
            "saved the key it is actually using"
        );
    }

    #[test]
    fn a_write_that_cannot_be_read_back_is_not_called_saved() {
        // The keychain accepts a write to an item whose ACL names a different
        // binary, and the result is still unreadable. Trusting the write meant
        // logging "sessions will survive a restart" when they would not - a
        // line that asserts the opposite of the truth is worse than no line.
        let (_, persistence) = resolve(&WriteOnly);
        assert_eq!(persistence, Persistence::Ephemeral);
    }

    #[test]
    fn an_unreadable_entry_is_replaced_rather_than_left_to_fail_forever() {
        // A read failure used to give up, so a single bad entry degraded every
        // future start. There is nothing to lose by overwriting: an entry we
        // cannot read is worth exactly as much as no entry.
        struct UnreadableOnce {
            inner: MemoryStore,
            poisoned: std::sync::atomic::AtomicBool,
        }

        impl SecretStore for UnreadableOnce {
            fn get(&self, name: &str) -> Result<Option<Vec<u8>>, SecretError> {
                if self.poisoned.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err(SecretError::Keystore("authentication failed".into()));
                }
                self.inner.get(name)
            }

            fn set(&self, name: &str, secret: &[u8]) -> Result<(), SecretError> {
                // Writing clears the bad entry, the way delete-and-add does.
                self.poisoned
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                self.inner.set(name, secret)
            }

            fn describe(&self) -> &'static str {
                "a keystore holding one unreadable entry"
            }
        }

        let store = UnreadableOnce {
            inner: MemoryStore::new(),
            poisoned: std::sync::atomic::AtomicBool::new(true),
        };

        let (keys, persistence) = resolve(&store);
        assert_eq!(persistence, Persistence::Saved, "it healed itself");

        // And the next start loads what the healing run wrote.
        let device = DeviceId::generate();
        let cookie = keys.mint(&device, NOW);
        let (after_restart, persistence) = resolve(&store);
        assert_eq!(persistence, Persistence::Loaded);
        assert!(after_restart.verify(&cookie, NOW).is_ok());
    }

    #[test]
    fn a_keystore_that_never_answers_does_not_hold_up_the_daemon() {
        // Found by running it: SecKeychainAddGenericPassword blocked in
        // AuthorizationCopyRights waiting for a dialog, and startup never
        // reached the point of binding the HTTP port. kt-certs refuses those
        // dialogs now; this is the backstop for whatever the next one is.
        let started = std::time::Instant::now();
        let keys = load_or_create_within(Arc::new(Wedged), Duration::from_millis(150)).session;

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

        let keys = load_or_create(store.clone()).session;
        let cookie = keys.mint(&device, NOW);
        assert!(
            keys.verify(&cookie, NOW).is_ok(),
            "still mints and verifies"
        );

        // And it really is ephemeral, rather than silently shared.
        let next = load_or_create(store).session;
        assert!(
            next.verify(&cookie, NOW).is_err(),
            "a key that could not be saved cannot come back"
        );
    }

    #[test]
    fn a_corrupt_entry_is_replaced_rather_than_fatal() {
        let store = Arc::new(MemoryStore::new());
        store.set(SESSION_KEY, b"too short").expect("writes");

        let keys = load_or_create(store.clone()).session;

        let saved = store.get(SESSION_KEY).expect("reads").expect("still there");
        assert_eq!(saved.len(), KEY_LEN, "the bad entry was overwritten");
        assert_eq!(saved, keys.to_bytes());

        // And the replacement persists, so this happens once rather than
        // every start.
        let device = DeviceId::generate();
        let cookie = keys.mint(&device, NOW);
        assert!(load_or_create(store).session.verify(&cookie, NOW).is_ok());
    }

    // The install identity, whose answers are deliberately not the session
    // key's. See the module docs.

    #[test]
    fn the_install_identity_is_the_same_machine_after_a_restart() {
        // The whole point of persisting it: an account is linked to this value,
        // so a restart must not change who this machine is.
        let store = Arc::new(MemoryStore::new());

        let first = load_or_create(store.clone()).install.expect("made one");
        let second = load_or_create(store).install.expect("loaded it");

        assert_eq!(first.public(), second.public());
    }

    #[test]
    fn the_first_run_saves_an_install_identity() {
        let store = Arc::new(MemoryStore::new());
        let keys = load_or_create(store.clone());

        let saved = store.get(INSTALL_KEY).expect("reads").expect("was saved");
        assert_eq!(saved.len(), KEY_LEN);
        assert_eq!(
            InstallIdentity::from_bytes(&saved.try_into().expect("right size")).public(),
            keys.install.expect("made one").public(),
            "saved the identity it is actually using"
        );
    }

    #[test]
    fn a_keystore_that_cannot_be_read_keeps_its_identity_rather_than_replacing_it() {
        // The difference from the session key, and the reason this function
        // exists separately. A read failure here is usually temporary - a
        // locked keychain, a rebuilt binary - and generating a replacement
        // would discard an identity linked to somebody's account on the
        // strength of a failure that clears by the next start.
        let store = Arc::new(MemoryStore::failing("the keychain is locked"));

        let keys = load_or_create(store.clone());

        assert!(
            keys.install.is_none(),
            "no identity is the honest answer; a new one would be a silent unlink"
        );
        assert!(
            store.get(INSTALL_KEY).is_err(),
            "and nothing was written over whatever is in there"
        );
        assert!(
            keys.session
                .verify(&keys.session.mint(&DeviceId::generate(), NOW), NOW)
                .is_ok(),
            "meanwhile serving carries on, which is the session key's opposite answer"
        );
    }

    #[test]
    fn a_write_that_cannot_be_read_back_yields_no_identity() {
        // Same keychain ACL trap as the session key, with more at stake: an
        // identity that does not survive the restart is one the owner links to
        // an account in the evening and finds unlinked in the morning.
        assert!(resolve_install(&WriteOnly).is_none());
    }

    #[test]
    fn a_corrupt_install_entry_is_replaced_because_nothing_is_lost_by_it() {
        // Unreadable-as-a-key is different from could-not-read. The identity
        // these bytes stood for is already gone, so replacing them costs
        // nothing that has not already been lost.
        let store = Arc::new(MemoryStore::new());
        store.set(INSTALL_KEY, b"too short").expect("writes");

        let identity = load_or_create(store.clone()).install.expect("replaced");

        let saved = store.get(INSTALL_KEY).expect("reads").expect("still there");
        assert_eq!(saved.len(), KEY_LEN, "the bad entry was overwritten");
        assert_eq!(
            load_or_create(store).install.expect("loads").public(),
            identity.public(),
            "and the replacement is the one that persists"
        );
    }

    #[test]
    fn a_wedged_keystore_costs_one_deadline_for_both_keys() {
        // Both keys are resolved in one visit under one budget, so a keychain
        // that hangs does not cost five seconds per key.
        let started = std::time::Instant::now();
        let keys = load_or_create_within(Arc::new(Wedged), Duration::from_millis(150));

        assert!(
            started.elapsed() < Duration::from_secs(1),
            "startup waited {:?} on the keystore",
            started.elapsed()
        );
        assert!(
            keys.install.is_none(),
            "and reports no identity rather than inventing one"
        );
    }

    #[test]
    fn the_two_keys_are_not_the_same_key() {
        // They are stored under different names and used for different jobs;
        // sharing one would mean rotating either rotates both.
        let store = Arc::new(MemoryStore::new());
        load_or_create(store.clone());

        let session = store.get(SESSION_KEY).expect("reads").expect("saved");
        let install = store.get(INSTALL_KEY).expect("reads").expect("saved");
        assert_ne!(session, install);
    }

    #[test]
    fn the_identity_can_answer_a_challenge() {
        // What the key is for. The edge verifies with the public half alone.
        use kt_tunnel_proto::{Nonce, ServerHello};

        let store = Arc::new(MemoryStore::new());
        let identity = load_or_create(store).install.expect("made one");

        let server = ServerHello::new(Nonce::generate());
        let hello = identity.hello(&server, "0.1.0", "macos").expect("signs");

        let accepted = server.accept(&hello).expect("the edge accepts it");
        assert_eq!(accepted.install, identity.public());
    }
}
