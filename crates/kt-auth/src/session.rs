//! Session cookies: Ed25519-signed, bound to a device.
//!
//! The daemon holds the signing key and is the only thing that can mint a
//! session. The public half is what the relay's edge will later verify against
//! when serving a snapshot, so it can check access without ever being able to
//! grant it (server-architecture.md section 3).

use base64ct::{Base64UrlUnpadded, Encoding};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::device::DeviceId;

/// How long a session lasts before the device has to be seen again.
///
/// Long, on purpose: a family member who opened a link a month ago should not
/// be asked to pair again, and revocation is the control that matters, not
/// expiry.
pub const SESSION_TTL_SECONDS: i64 = 60 * 60 * 24 * 90;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SessionError {
    #[error("the session cookie is malformed")]
    Malformed,
    #[error("the session signature does not verify")]
    BadSignature,
    #[error("the session has expired")]
    Expired,
}

/// The daemon's session-signing keypair.
pub struct SessionKeys {
    signing: SigningKey,
}

impl SessionKeys {
    /// A fresh keypair. Generated once per install and kept in the Keychain;
    /// losing it invalidates every session, which is inconvenient but safe.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("the OS random source must work");
        Self {
            signing: SigningKey::from_bytes(&bytes),
        }
    }

    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        Self {
            signing: SigningKey::from_bytes(bytes),
        }
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.signing.to_bytes()
    }

    /// The half that gets registered with the control plane so the edge can
    /// verify sessions it cannot mint.
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing.verifying_key()
    }

    /// Mint a session for a device.
    pub fn mint(&self, device: &DeviceId, now: i64) -> String {
        let payload = format!("{}.{}", device.as_str(), now + SESSION_TTL_SECONDS);
        let signature = self.signing.sign(payload.as_bytes());
        format!(
            "{payload}.{}",
            Base64UrlUnpadded::encode_string(&signature.to_bytes())
        )
    }

    /// Verify a cookie and return what it claims.
    ///
    /// Order matters: the signature is checked before expiry, so an attacker
    /// cannot learn whether a forged device id would otherwise have been valid.
    pub fn verify(&self, cookie: &str, now: i64) -> Result<SessionToken, SessionError> {
        let mut parts = cookie.rsplitn(2, '.');
        let signature_b64 = parts.next().ok_or(SessionError::Malformed)?;
        let payload = parts.next().ok_or(SessionError::Malformed)?;

        let (device_raw, expiry_raw) = payload.split_once('.').ok_or(SessionError::Malformed)?;
        let device = DeviceId::parse(device_raw).ok_or(SessionError::Malformed)?;
        let expires_at: i64 = expiry_raw.parse().map_err(|_| SessionError::Malformed)?;

        let signature_bytes: [u8; 64] = Base64UrlUnpadded::decode_vec(signature_b64)
            .map_err(|_| SessionError::Malformed)?
            .try_into()
            .map_err(|_| SessionError::Malformed)?;

        self.signing
            .verifying_key()
            .verify(payload.as_bytes(), &Signature::from_bytes(&signature_bytes))
            .map_err(|_| SessionError::BadSignature)?;

        if now >= expires_at {
            return Err(SessionError::Expired);
        }

        Ok(SessionToken { device, expires_at })
    }
}

/// A verified session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionToken {
    pub device: DeviceId,
    pub expires_at: i64,
}

/// A URL-safe random string of `bytes` bytes of entropy.
pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("the OS random source must work");
    Base64UrlUnpadded::encode_string(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    #[test]
    fn a_minted_session_verifies() {
        let keys = SessionKeys::generate();
        let device = DeviceId::generate();

        let cookie = keys.mint(&device, NOW);
        let token = keys.verify(&cookie, NOW).expect("verifies");

        assert_eq!(token.device, device);
        assert_eq!(token.expires_at, NOW + SESSION_TTL_SECONDS);
    }

    #[test]
    fn another_daemons_signature_does_not_verify() {
        let mine = SessionKeys::generate();
        let theirs = SessionKeys::generate();
        let cookie = theirs.mint(&DeviceId::generate(), NOW);

        assert_eq!(mine.verify(&cookie, NOW), Err(SessionError::BadSignature));
    }

    #[test]
    fn changing_the_device_invalidates_the_cookie() {
        // The whole point of binding: a session for one device must not be
        // reusable by editing the cookie.
        let keys = SessionKeys::generate();
        let cookie = keys.mint(&DeviceId::generate(), NOW);

        let forged = format!(
            "{}.{}",
            DeviceId::generate(),
            cookie.split_once('.').expect("has parts").1
        );
        assert_eq!(keys.verify(&forged, NOW), Err(SessionError::BadSignature));
    }

    #[test]
    fn extending_the_expiry_invalidates_the_cookie() {
        let keys = SessionKeys::generate();
        let device = DeviceId::generate();
        let cookie = keys.mint(&device, NOW);
        let signature = cookie.rsplit_once('.').expect("has a signature").1;

        let forged = format!("{device}.{}.{signature}", NOW + 999_999_999);
        assert_eq!(keys.verify(&forged, NOW), Err(SessionError::BadSignature));
    }

    #[test]
    fn an_expired_session_is_rejected() {
        let keys = SessionKeys::generate();
        let cookie = keys.mint(&DeviceId::generate(), NOW);

        assert_eq!(
            keys.verify(&cookie, NOW + SESSION_TTL_SECONDS),
            Err(SessionError::Expired)
        );
    }

    #[test]
    fn garbage_is_malformed_not_a_panic() {
        let keys = SessionKeys::generate();
        for bad in [
            "",
            ".",
            "no-dots",
            "a.b",
            "a.b.c",
            "....",
            "x.999999999999999999999.y",
        ] {
            let result = keys.verify(bad, NOW);
            assert!(result.is_err(), "{bad:?} should not verify");
        }
    }

    #[test]
    fn keys_round_trip_through_bytes() {
        // Needed for the Keychain: the same key must produce the same
        // signatures after a restart, or every session breaks on relaunch.
        let keys = SessionKeys::generate();
        let device = DeviceId::generate();
        let cookie = keys.mint(&device, NOW);

        let restored = SessionKeys::from_bytes(&keys.to_bytes());
        assert!(restored.verify(&cookie, NOW).is_ok());
        assert_eq!(restored.verifying_key(), keys.verifying_key());
    }

    #[test]
    fn tokens_are_unpredictable() {
        let a = random_token(16);
        let b = random_token(16);
        assert_ne!(a, b);
        assert!(a.len() >= 20, "16 bytes should not encode this short");
    }
}
