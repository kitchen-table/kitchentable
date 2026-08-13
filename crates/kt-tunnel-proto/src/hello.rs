//! Proving which machine is on the other end of the tunnel.
//!
//! Two messages. The edge speaks first with a [`ServerHello`] carrying a fresh
//! random nonce and the versions it speaks; the daemon answers with a
//! [`ClientHello`] naming its install key and signing what it was given.
//!
//! The key is the daemon's **install key**: an Ed25519 keypair generated on
//! first run and kept in the machine's keystore, distinct from the
//! session-signing key in `kt-auth` and used for a different job: that one
//! signs viewer sessions, this one says which machine is dialling.
//! Authentication is a signature rather
//! than a bearer token on purpose. A token that authenticates a tunnel is a
//! token that can be copied off a machine and used from somewhere else, and
//! the account password that could otherwise be phished into the same power
//! never enters this exchange at all. Nothing here can be replayed from
//! another machine without the private key, which never leaves the keystore.
//!
//! # What is signed
//!
//! [`hello_transcript`] is the single definition of that, compiled by both
//! sides, which is the point of putting it in a shared crate rather than
//! writing it twice. It covers:
//!
//! - **A context string**, so a signature made here can never be mistaken for
//!   one made anywhere else. The install key signs other things elsewhere; a
//!   signature is only ever evidence about the message it was made over, and
//!   the context is what pins down which message that was.
//! - **The nonce**, which the edge chose and has never used before, so a
//!   recorded hello cannot be replayed against a later connection.
//! - **The negotiated version**, so a hello is valid for one version only and
//!   a signature captured during a version-1 connection cannot be presented as
//!   agreement to version 2.
//! - **The install key itself**, binding the signature to the identity being
//!   claimed rather than leaving the two merely adjacent in the same message.
//!
//! Every field is fixed width after the context separator, so there is exactly
//! one byte string for any set of inputs and no way to shift meaning between
//! fields by padding one of them.

use base64ct::{Base64UrlUnpadded, Encoding};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::version::{VersionError, VersionRange};

/// Nonces are 32 bytes: far past any birthday concern for a value that only
/// has to be unique among live connections, and the same width as the keys it
/// sits beside.
pub const NONCE_LEN: usize = 32;

/// Domain separation for the hello signature. The trailing version marker is
/// part of the string rather than a number appended to it, so a future
/// transcript layout gets a new context and old signatures cannot be carried
/// across into it.
const HELLO_CONTEXT: &[u8] = b"kitchentable-tunnel-hello-v1";

/// A challenge from the edge, used once.
///
/// The edge must generate it randomly per connection and must not accept a
/// nonce it has already seen; the daemon must not reuse one. That is the
/// entire anti-replay story, so it is worth being blunt about: everything else
/// in this exchange is public.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Nonce(#[serde(with = "crate::b64")] [u8; NONCE_LEN]);

impl Nonce {
    /// A fresh nonce from the OS random source.
    pub fn generate() -> Self {
        let mut bytes = [0u8; NONCE_LEN];
        getrandom::fill(&mut bytes).expect("the OS random source must work");
        Self(bytes)
    }

    pub const fn from_bytes(bytes: [u8; NONCE_LEN]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; NONCE_LEN] {
        &self.0
    }
}

/// The public half of a daemon's install keypair: which machine this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InstallKey(#[serde(with = "crate::b64")] [u8; 32]);

impl InstallKey {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The verifying key, if these bytes are a point on the curve at all.
    ///
    /// Fallible because 32 arbitrary bytes are not necessarily a key, and the
    /// caller is holding bytes that arrived over a network.
    pub fn verifying_key(&self) -> Result<VerifyingKey, HelloError> {
        VerifyingKey::from_bytes(&self.0).map_err(|_| HelloError::MalformedKey)
    }
}

impl From<&VerifyingKey> for InstallKey {
    fn from(key: &VerifyingKey) -> Self {
        Self(key.to_bytes())
    }
}

impl std::fmt::Display for InstallKey {
    /// The form it takes everywhere a human or a database sees it: unpadded
    /// base64url, the same as on the wire. Safe to log and to print - it is the
    /// public half, and the whole point of it is being told to other people.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&Base64UrlUnpadded::encode_string(&self.0))
    }
}

/// A daemon's install keypair: the private half included.
///
/// Lives here rather than in the daemon for the same reason `SessionKeys` lives
/// in `kt-auth` rather than beside the code that stores it - the cryptography
/// belongs with the thing it is cryptography *for*. Where the key is kept
/// between runs, and what to do when it cannot be kept, is a separate question
/// and a different crate's business.
///
/// The private half never leaves this type. Callers get [`InstallIdentity::public`]
/// to register or display, and [`InstallIdentity::hello`] when there is a
/// challenge to answer.
pub struct InstallIdentity {
    signing: SigningKey,
}

impl InstallIdentity {
    /// A fresh identity. Generated once per install, on first run.
    pub fn generate() -> Self {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).expect("the OS random source must work");
        Self::from_bytes(&seed)
    }

    pub fn from_bytes(seed: &[u8; 32]) -> Self {
        Self {
            signing: SigningKey::from_bytes(seed),
        }
    }

    /// The seed, for putting in a keystore. The only way the private half
    /// leaves, and the caller is expected to have somewhere safe to put it.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.signing.to_bytes()
    }

    /// What the control plane and the edge know this install by.
    pub fn public(&self) -> InstallKey {
        InstallKey::from(&self.signing.verifying_key())
    }

    /// Answer a challenge from the edge.
    pub fn hello(
        &self,
        server: &ServerHello,
        daemon_version: &str,
        platform: &str,
    ) -> Result<ClientHello, HelloError> {
        ClientHello::sign(server, &self.signing, daemon_version, platform)
    }
}

/// The exact bytes a hello signature is made over.
///
/// Both sides call this. Neither side should ever assemble it by hand: a
/// mismatch between the two would not be a compile error, and would look like
/// every daemon in the world suddenly failing to authenticate.
pub fn hello_transcript(version: u16, nonce: &Nonce, install: &InstallKey) -> Vec<u8> {
    let mut transcript = Vec::with_capacity(HELLO_CONTEXT.len() + 1 + 2 + NONCE_LEN + 32);
    transcript.extend_from_slice(HELLO_CONTEXT);
    // A byte the context cannot contain, so the context can never run into the
    // fields after it however the two are read.
    transcript.push(0);
    transcript.extend_from_slice(&version.to_be_bytes());
    transcript.extend_from_slice(nonce.as_bytes());
    transcript.extend_from_slice(install.as_bytes());
    transcript
}

/// First message on a new tunnel connection, from the edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerHello {
    /// What the edge speaks. The daemon picks from this.
    pub versions: VersionRange,
    /// Fresh, and never to be seen again.
    pub nonce: Nonce,
}

impl ServerHello {
    pub fn new(nonce: Nonce) -> Self {
        Self {
            versions: VersionRange::supported(),
            nonce,
        }
    }

    /// Check a daemon's answer, and say who it is if it holds up.
    ///
    /// The version is settled before the signature is examined, because the
    /// signature is over a transcript that names the version: checking them in
    /// the other order would mean verifying against a version the peer had not
    /// actually agreed to.
    pub fn accept(&self, hello: &ClientHello) -> Result<Accepted, HelloError> {
        let expected = self.versions.negotiate(hello.versions)?;
        if hello.version != expected {
            return Err(HelloError::VersionNotNegotiated {
                claimed: hello.version,
                expected,
            });
        }

        let verifying = hello.install.verifying_key()?;
        let signature = Signature::from_bytes(&hello.signature);
        let transcript = hello_transcript(hello.version, &self.nonce, &hello.install);
        verifying
            .verify(&transcript, &signature)
            .map_err(|_| HelloError::BadSignature)?;

        Ok(Accepted {
            version: hello.version,
            install: hello.install,
        })
    }
}

/// The daemon's answer: who it is, what it picked, and proof of both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientHello {
    /// The version chosen out of what the edge offered.
    pub version: u16,
    /// What the daemon can speak, so a rejection can explain itself instead of
    /// saying only that this connection failed.
    pub versions: VersionRange,
    pub install: InstallKey,
    /// Free-form, for the fleet view and for support: neither is trusted, and
    /// neither may be used to decide anything.
    pub daemon_version: String,
    pub platform: String,
    #[serde(with = "crate::b64")]
    pub signature: [u8; 64],
}

impl ClientHello {
    /// Answer a [`ServerHello`], picking the best shared version and signing
    /// for it.
    pub fn sign(
        server: &ServerHello,
        signing: &SigningKey,
        daemon_version: impl Into<String>,
        platform: impl Into<String>,
    ) -> Result<Self, HelloError> {
        let ours = VersionRange::supported();
        let version = ours.negotiate(server.versions)?;
        let install = InstallKey::from(&signing.verifying_key());
        let transcript = hello_transcript(version, &server.nonce, &install);

        Ok(Self {
            version,
            versions: ours,
            install,
            daemon_version: daemon_version.into(),
            platform: platform.into(),
            signature: signing.sign(&transcript).to_bytes(),
        })
    }
}

/// The edge's answer to a hello. The third and last message of the handshake.
///
/// An explicit answer rather than silence-means-yes, which is what this was
/// first written as. Silence costs one round trip less and is wrong for a
/// reason worth recording: the daemon would carry on believing it was connected
/// while the edge had already refused it, and - because a connection that got
/// that far counts as a success - it would reset its reconnect backoff and dial
/// again immediately, every time, for as long as the refusal lasted. The
/// tightest possible loop, produced by the one mechanism meant to prevent it.
///
/// A connection lasts hours. One round trip to know whether it exists is not a
/// cost worth arguing about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "hello", rename_all = "snake_case")]
pub enum HelloResponse {
    /// In. This is the last thing said before real traffic.
    Welcome { version: u16 },
    /// Not in, and why. Sent as a frame rather than in a websocket close
    /// frame, whose reason field caps at 123 bytes and would silently truncate
    /// anything with a detail string on it.
    Refused { reason: crate::CloseReason },
}

/// What the edge learned from a hello it accepted.
///
/// Deliberately small. Whether this install is linked to an account, still
/// subscribed, or suspended is the control plane's to answer, and a signature
/// says nothing about any of it - only that the machine holding this key is on
/// the other end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accepted {
    pub version: u16,
    pub install: InstallKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HelloError {
    #[error(transparent)]
    Version(#[from] VersionError),
    /// The peer signed for a version that is not the one the two of them would
    /// have agreed on. A tampered or confused peer, not an old one.
    #[error("the hello claims version {claimed} where {expected} was negotiated")]
    VersionNotNegotiated { claimed: u16, expected: u16 },
    #[error("the install key is not a valid public key")]
    MalformedKey,
    #[error("the hello signature does not verify")]
    BadSignature,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::PROTOCOL_VERSION;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn exchange() -> (ServerHello, SigningKey) {
        (ServerHello::new(Nonce::from_bytes([7; NONCE_LEN])), key(1))
    }

    #[test]
    fn a_daemon_that_holds_the_key_gets_in() {
        let (server, signing) = exchange();
        let hello = ClientHello::sign(&server, &signing, "0.1.0", "macos").unwrap();

        let accepted = server.accept(&hello).unwrap();
        assert_eq!(accepted.version, PROTOCOL_VERSION);
        assert_eq!(accepted.install, InstallKey::from(&signing.verifying_key()));
    }

    #[test]
    fn a_hello_recorded_from_one_connection_is_refused_on_the_next() {
        // The whole purpose of the nonce, stated as a test: capturing a valid
        // hello off the wire must not let it be replayed at a fresh challenge.
        let (first, signing) = exchange();
        let hello = ClientHello::sign(&first, &signing, "0.1.0", "macos").unwrap();

        let second = ServerHello::new(Nonce::from_bytes([9; NONCE_LEN]));
        assert_eq!(second.accept(&hello), Err(HelloError::BadSignature));
    }

    #[test]
    fn a_signature_cannot_be_moved_onto_another_identity() {
        let (server, signing) = exchange();
        let mut hello = ClientHello::sign(&server, &signing, "0.1.0", "macos").unwrap();

        // Claim to be a different machine while presenting the signature made
        // by this one. The transcript names the key, so it does not verify.
        hello.install = InstallKey::from(&key(2).verifying_key());
        assert_eq!(server.accept(&hello), Err(HelloError::BadSignature));
    }

    #[test]
    fn the_version_cannot_be_altered_after_signing() {
        // A daemon from the future, so that there is a choice to tamper with:
        // both ends here can speak 2, and both would pick it.
        let server = ServerHello {
            versions: VersionRange::new(1, 2),
            nonce: Nonce::from_bytes([7; NONCE_LEN]),
        };
        let signing = key(1);
        let install = InstallKey::from(&signing.verifying_key());
        let honest = ClientHello {
            version: 2,
            versions: VersionRange::new(1, 2),
            install,
            daemon_version: "0.2.0".into(),
            platform: "macos".into(),
            signature: signing
                .sign(&hello_transcript(2, &server.nonce, &install))
                .to_bytes(),
        };
        assert_eq!(server.accept(&honest).unwrap().version, 2);

        // Downgraded in flight. The two would have settled on 2, so a hello
        // claiming 1 is refused before the signature is even examined.
        let mut downgraded = honest.clone();
        downgraded.version = 1;
        assert_eq!(
            server.accept(&downgraded),
            Err(HelloError::VersionNotNegotiated {
                claimed: 1,
                expected: 2
            })
        );

        // Downgraded more carefully: the declared range is dragged down too, so
        // negotiation now agrees with the claim and the previous check passes.
        // This is the case the transcript exists for. The signature was made
        // over version 2 and verifies for version 2 alone.
        let mut forced = honest.clone();
        forced.version = 1;
        forced.versions = VersionRange::new(1, 1);
        assert_eq!(server.accept(&forced), Err(HelloError::BadSignature));
    }

    #[test]
    fn a_forged_signature_is_refused() {
        let (server, signing) = exchange();
        let mut hello = ClientHello::sign(&server, &signing, "0.1.0", "macos").unwrap();
        hello.signature[0] ^= 0xff;
        assert_eq!(server.accept(&hello), Err(HelloError::BadSignature));
    }

    #[test]
    fn garbage_in_the_key_field_is_refused_either_way() {
        let (server, signing) = exchange();
        let hello = ClientHello::sign(&server, &signing, "0.1.0", "macos").unwrap();

        // Roughly half of all 32-byte strings decompress to a point and half do
        // not, so there are two refusals here and both matter. This one is not
        // a point: caught while decoding, which keeps the failure legible
        // instead of surfacing from inside dalek later.
        let mut not_a_point = hello.clone();
        not_a_point.install = InstallKey::from_bytes([0x02; 32]);
        assert_eq!(server.accept(&not_a_point), Err(HelloError::MalformedKey));

        // And this one is a perfectly good point that nobody holds the private
        // half of. Nothing about it is malformed; it simply cannot have signed
        // this. The signature is what refuses it.
        let mut wrong_point = hello;
        wrong_point.install = InstallKey::from_bytes([0xff; 32]);
        assert!(wrong_point.install.verifying_key().is_ok());
        assert_eq!(server.accept(&wrong_point), Err(HelloError::BadSignature));
    }

    #[test]
    fn peers_with_no_shared_version_never_reach_the_signature() {
        let server = ServerHello {
            versions: VersionRange::new(500, 900),
            nonce: Nonce::from_bytes([7; NONCE_LEN]),
        };
        let signing = key(1);
        assert!(matches!(
            ClientHello::sign(&server, &signing, "0.1.0", "macos"),
            Err(HelloError::Version(VersionError::PeerTooNew { .. }))
        ));
    }

    #[test]
    fn the_transcript_is_domain_separated() {
        // The same nonce and key under a different context must not produce a
        // signature that verifies here. Demonstrated by signing the bare
        // inputs, which is what a careless second implementation would do.
        let (server, signing) = exchange();
        let install = InstallKey::from(&signing.verifying_key());
        let naive = signing.sign(server.nonce.as_bytes()).to_bytes();

        let hello = ClientHello {
            version: PROTOCOL_VERSION,
            versions: VersionRange::supported(),
            install,
            daemon_version: "0.1.0".into(),
            platform: "macos".into(),
            signature: naive,
        };
        assert_eq!(server.accept(&hello), Err(HelloError::BadSignature));
    }

    #[test]
    fn the_transcript_separator_cannot_be_smuggled_across_fields() {
        // Every field after the context is fixed width, so no pair of distinct
        // inputs can produce the same bytes.
        let a = hello_transcript(
            1,
            &Nonce::from_bytes([0; 32]),
            &InstallKey::from_bytes([1; 32]),
        );
        let b = hello_transcript(
            1,
            &Nonce::from_bytes([1; 32]),
            &InstallKey::from_bytes([0; 32]),
        );
        assert_ne!(a, b);
        assert_eq!(a.len(), b.len());
    }

    #[test]
    fn hellos_survive_a_round_trip_through_json() {
        let (server, signing) = exchange();
        let hello = ClientHello::sign(&server, &signing, "0.1.0", "macos").unwrap();

        let server_json = serde_json::to_string(&server).unwrap();
        let hello_json = serde_json::to_string(&hello).unwrap();
        let server_back: ServerHello = serde_json::from_str(&server_json).unwrap();
        let hello_back: ClientHello = serde_json::from_str(&hello_json).unwrap();

        assert_eq!(server_back, server);
        assert_eq!(hello_back, hello);
        server_back.accept(&hello_back).unwrap();
    }

    #[test]
    fn keys_and_nonces_are_base64url_on_the_wire() {
        let hello = ServerHello::new(Nonce::from_bytes([0; NONCE_LEN]));
        let json = serde_json::to_string(&hello).unwrap();
        // 32 zero bytes, unpadded base64url: no '=' and no '+' or '/'.
        assert!(json.contains("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"));
        assert!(!json.contains('='));
    }

    #[test]
    fn a_newer_peer_may_add_fields_without_breaking_an_older_one() {
        // Version skew is the normal case, so unknown fields are ignored
        // rather than rejected.
        let json = r#"{"versions":{"min":1,"max":1},
                       "nonce":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                       "region":"eu-west-1"}"#;
        let hello: ServerHello = serde_json::from_str(json).unwrap();
        assert_eq!(hello.versions, VersionRange::new(1, 1));
    }

    #[test]
    fn a_key_of_the_wrong_length_is_rejected_on_the_way_in() {
        let json = r#"{"versions":{"min":1,"max":1},"nonce":"AAAA"}"#;
        assert!(serde_json::from_str::<ServerHello>(json).is_err());
    }
}
