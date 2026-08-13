//! macOS: secrets in the login Keychain, through Security.framework.
//!
//! Deliberately the generic-password API rather than a crate. It is four C
//! functions taking plain pointers and lengths, whereas the modern `SecItem*`
//! route means hand-building CFDictionaries and getting CFRelease discipline
//! right for every key and value in them - more unsafe code, in the crate that
//! will hold the CA's private key, to reach the same drawer. `dns_sd` in
//! kt-mdns is here for the same reason: no dependency, one file, and the
//! platform behaviour is the point.
//!
//! `SecKeychainAddGenericPassword` and friends have carried a deprecation
//! notice since 10.10 and still work in macOS 26. If Apple ever removes them,
//! everything that has to change is behind [`SecretStore`] in this one file.

use std::ffi::{c_char, c_void};

use crate::secret::{SecretError, SecretStore};

/// What a person browsing Keychain Access sees. Stable: renaming it would
/// orphan the existing item and quietly re-pair every viewer in the house.
const SERVICE: &str = "Kitchen Table";

type OsStatus = i32;
type SecKeychainItemRef = *mut c_void;
type CfTypeRef = *const c_void;

const ERR_SEC_SUCCESS: OsStatus = 0;
const ERR_SEC_DUPLICATE_ITEM: OsStatus = -25299;
const ERR_SEC_ITEM_NOT_FOUND: OsStatus = -25300;
const ERR_SEC_AUTH_FAILED: OsStatus = -25293;
const ERR_SEC_INTERACTION_NOT_ALLOWED: OsStatus = -25308;
const ERR_SEC_USER_CANCELED: OsStatus = -128;

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecKeychainAddGenericPassword(
        keychain: *mut c_void,
        service_name_length: u32,
        service_name: *const c_char,
        account_name_length: u32,
        account_name: *const c_char,
        password_length: u32,
        password_data: *const c_void,
        item_ref: *mut SecKeychainItemRef,
    ) -> OsStatus;

    fn SecKeychainFindGenericPassword(
        keychain_or_array: CfTypeRef,
        service_name_length: u32,
        service_name: *const c_char,
        account_name_length: u32,
        account_name: *const c_char,
        password_length: *mut u32,
        password_data: *mut *mut c_void,
        item_ref: *mut SecKeychainItemRef,
    ) -> OsStatus;

    fn SecKeychainItemFreeContent(attr_list: *mut c_void, data: *mut c_void) -> OsStatus;

    fn SecKeychainItemDelete(item_ref: SecKeychainItemRef) -> OsStatus;

    fn SecKeychainSetUserInteractionAllowed(state: u8) -> OsStatus;
    fn SecKeychainGetUserInteractionAllowed(state: *mut u8) -> OsStatus;
}

/// Stops Security.framework putting a dialog up, for as long as it is alive.
///
/// Found by running the daemon: without this, `SecKeychainAddGenericPassword`
/// walks into `makeLoginAuthUI` -> `AuthorizationCopyRights` -> `mach_msg` and
/// blocks forever waiting for a click that a background daemon has no way to
/// deliver. Startup never got as far as binding the HTTP port, so the entire
/// product was down - a strictly worse failure than the one persisting the key
/// was meant to fix.
///
/// With interaction refused, the same situation returns
/// `errSecInteractionNotAllowed` instead, which is a value we can log and
/// degrade on. It is also the right behaviour on the merits: a daemon that
/// starts at login has no business interrupting that login with a keychain
/// prompt, and the login keychain is already unlocked in the normal case.
///
/// The setting is process-global, which is tolerable because the daemon has
/// exactly one keychain user. The previous value is restored on drop so this
/// stays true if that ever stops being the case.
struct NoInteraction {
    previous: u8,
}

impl NoInteraction {
    fn enter() -> Self {
        let mut previous: u8 = 1;
        // SAFETY: both take plain scalars; `previous` is a live local. A
        // failure to read the old value leaves the default of "allowed", which
        // is what the process started with anyway.
        unsafe {
            SecKeychainGetUserInteractionAllowed(&mut previous);
            SecKeychainSetUserInteractionAllowed(0);
        }
        Self { previous }
    }
}

impl Drop for NoInteraction {
    fn drop(&mut self) {
        // SAFETY: a plain scalar back to where it was.
        unsafe { SecKeychainSetUserInteractionAllowed(self.previous) };
    }
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: CfTypeRef);
}

/// Turn an `OSStatus` into something worth putting in a log line.
///
/// The numeric code is kept in every case: the named ones are the states a
/// person can actually be in, and the rest are rare enough that the number is
/// what you would search for anyway.
fn describe(status: OsStatus) -> String {
    let meaning = match status {
        ERR_SEC_AUTH_FAILED => "authentication failed",
        ERR_SEC_INTERACTION_NOT_ALLOWED => "the keychain is locked and cannot prompt",
        ERR_SEC_USER_CANCELED => "the request was cancelled",
        _ => "unexpected keychain error",
    };
    format!("{meaning} (OSStatus {status})")
}

/// The login Keychain.
pub struct Keychain;

impl Keychain {
    pub fn new() -> Self {
        Self
    }

    /// File a new item, returning the raw status so `set` can tell a name that
    /// is already taken apart from a real failure.
    ///
    /// Callers hold [`NoInteraction`] for the whole sequence rather than this
    /// taking it, so a set that adds, removes and adds again cannot have a
    /// dialog appear between the steps.
    fn add(&self, name: &str, secret: &[u8]) -> OsStatus {
        // SAFETY: every pointer is borrowed for the duration of the call, with
        // lengths passed explicitly. Passing null for `item_ref` means nothing
        // is returned for us to release.
        unsafe {
            SecKeychainAddGenericPassword(
                std::ptr::null_mut(),
                SERVICE.len() as u32,
                SERVICE.as_ptr().cast(),
                name.len() as u32,
                name.as_ptr().cast(),
                secret.len() as u32,
                secret.as_ptr().cast(),
                std::ptr::null_mut(),
            )
        }
    }

    /// Remove an item that already exists, so `set` can add a fresh one.
    ///
    /// Deliberately *not* `SecKeychainItemModifyAttributesAndData`, which was
    /// the obvious implementation and the wrong one. A keychain item carries an
    /// access control list naming the applications allowed to read it, and that
    /// list belongs to the item rather than to its contents - so modifying in
    /// place keeps the old ACL, and the daemon that just wrote the key still
    /// cannot read it back. The failure is quiet and expensive: the write
    /// returns `errSecSuccess`, and only the next start finds out.
    ///
    /// This bites whenever the writing binary's identity changes, which for an
    /// unsigned development build is every rebuild, and in production would be
    /// any change of signing identity. Adding fresh gives the item an ACL that
    /// names the binary doing it now.
    ///
    /// Finding the item does not need the ACL: asking for the reference alone
    /// decrypts nothing, which is why this works even when reading the same
    /// item does not.
    fn remove(&self, name: &str) -> Result<(), SecretError> {
        let mut item: SecKeychainItemRef = std::ptr::null_mut();

        // SAFETY: both strings are borrowed for the duration of the call and
        // their lengths are passed explicitly, so no NUL terminator is needed.
        // Asking only for `item_ref` is allowed; the password is not copied out.
        let status = unsafe {
            SecKeychainFindGenericPassword(
                std::ptr::null(),
                SERVICE.len() as u32,
                SERVICE.as_ptr().cast(),
                name.len() as u32,
                name.as_ptr().cast(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut item,
            )
        };
        if status != ERR_SEC_SUCCESS {
            return Err(SecretError::Keystore(describe(status)));
        }

        // SAFETY: `item` is a live reference handed back by the call above.
        let status = unsafe { SecKeychainItemDelete(item) };
        // SAFETY: the find above followed the Create Rule, so this reference is
        // ours to release, and nothing reads `item` after this point. Deleting
        // an item does not release the reference to it.
        unsafe { CFRelease(item.cast()) };

        if status == ERR_SEC_SUCCESS {
            Ok(())
        } else {
            Err(SecretError::Keystore(describe(status)))
        }
    }
}

impl Default for Keychain {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for Keychain {
    fn get(&self, name: &str) -> Result<Option<Vec<u8>>, SecretError> {
        let _no_ui = NoInteraction::enter();
        let mut length: u32 = 0;
        let mut data: *mut c_void = std::ptr::null_mut();

        // SAFETY: as in `replace`. On success the framework allocates the
        // password and hands us ownership until SecKeychainItemFreeContent.
        let status = unsafe {
            SecKeychainFindGenericPassword(
                std::ptr::null(),
                SERVICE.len() as u32,
                SERVICE.as_ptr().cast(),
                name.len() as u32,
                name.as_ptr().cast(),
                &mut length,
                &mut data,
                std::ptr::null_mut(),
            )
        };

        match status {
            ERR_SEC_SUCCESS => {
                if data.is_null() {
                    // Documented not to happen on success, but a null here
                    // would be a segfault rather than a bad key, so it is
                    // worth the two lines.
                    return Ok(None);
                }
                // SAFETY: on success the framework guarantees `length` readable
                // bytes at `data`; the copy happens before anything frees it.
                let bytes =
                    unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length as usize) }
                        .to_vec();
                // SAFETY: `data` came from the call above and is freed exactly
                // once here. `bytes` already owns its own copy.
                unsafe { SecKeychainItemFreeContent(std::ptr::null_mut(), data) };
                Ok(Some(bytes))
            }
            ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            other => Err(SecretError::Keystore(describe(other))),
        }
    }

    fn set(&self, name: &str, secret: &[u8]) -> Result<(), SecretError> {
        let _no_ui = NoInteraction::enter();

        match self.add(name, secret) {
            ERR_SEC_SUCCESS => Ok(()),
            ERR_SEC_DUPLICATE_ITEM => {
                // Something is already filed under this name. Remove it and add
                // fresh rather than editing it, so the new item's ACL names the
                // binary running now - see `remove`.
                self.remove(name)?;
                match self.add(name, secret) {
                    ERR_SEC_SUCCESS => Ok(()),
                    other => Err(SecretError::Keystore(describe(other))),
                }
            }
            other => Err(SecretError::Keystore(describe(other))),
        }
    }

    fn describe(&self) -> &'static str {
        "the Keychain"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_are_described_with_their_code() {
        // The code is what someone pastes into a search engine at 1am.
        assert!(describe(ERR_SEC_INTERACTION_NOT_ALLOWED).contains("-25308"));
        assert!(describe(ERR_SEC_INTERACTION_NOT_ALLOWED).contains("locked"));
        assert!(describe(-1).contains("-1"));
    }

    /// The real thing, against the real login Keychain.
    ///
    /// Ignored on purpose. It writes to the Keychain of whoever runs it, and
    /// on an unsigned debug binary macOS may put up an access prompt that
    /// would hang CI forever. This is the same category as the mDNS work: the
    /// last mile needs a real machine.
    ///
    ///     cargo test -p kt-certs --  --ignored keychain_round_trip
    ///
    /// Afterwards, `security delete-generic-password -s 'Kitchen Table'
    /// -a kt-certs-selftest` removes what it left behind.
    #[test]
    #[ignore = "writes to the real login Keychain; run by hand"]
    fn keychain_round_trip() {
        let keychain = Keychain::new();
        let name = "kt-certs-selftest";

        keychain.set(name, &[42u8; 32]).expect("writes");
        assert_eq!(keychain.get(name).expect("reads"), Some(vec![42u8; 32]));

        // The duplicate-item path: a second set must replace, not fail.
        keychain.set(name, &[13u8; 32]).expect("replaces");
        assert_eq!(keychain.get(name).expect("reads"), Some(vec![13u8; 32]));

        assert_eq!(
            keychain.get("kt-certs-absent").expect("reads"),
            None,
            "a name that was never stored is None, not an error"
        );
    }
}
