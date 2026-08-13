//! The words the daemon and the relay edge have to agree on, exactly.
//!
//! Kitchen Table serves from your own machine, which is behind a home router
//! that accepts no incoming connections. The relay exists to bridge that: the
//! daemon dials *out* to the edge and holds the connection open, and the edge
//! hands viewer requests back down it. Outbound-only is the whole trick, and it
//! is why no port forwarding, no firewall hole and no static IP are ever asked
//! of anyone.
//!
//! ```text
//!   TLS from the viewer's browser
//!     -> edge terminates (Standard mode) and opens one yamux stream per request
//!        -> WSS to the daemon, dialled outbound, one connection, many streams
//!           -> daemon's own gate, then the app's files
//! ```
//!
//! This crate is the layer in the middle of that picture and nothing else: the
//! handshake, the version rules, the shape of a request as it goes down a
//! stream, and the reasons a connection can end.
//!
//! # What is deliberately not here
//!
//! No I/O, no async, no yamux, no websocket. Every type is plain data and every
//! function is pure, which has three payoffs. The whole crate is testable
//! without a socket. The private repo's edge pins this as a git dependency
//! ([repo-architecture.md] section 4) and does not inherit a runtime with it.
//! And the parts most worth getting right - what gets signed, what counts as a
//! compatible version, what a peer is allowed to make us allocate - are decided
//! in one place that both sides compile, instead of twice in two languages'
//! worth of good intentions.
//!
//! The transport lives in the daemon's relay client and in the edge. This crate
//! only says what travels over it.
//!
//! # The rule that matters most
//!
//! **A request that arrives over the tunnel came from the internet.** It is not
//! on the household network and it is not from this machine, whatever address
//! the edge reports for the viewer. [`HttpRequestHead`] therefore has no field
//! that could say otherwise: the type cannot express the lie, because the gate
//! in `kt-auth` grants Network-visibility apps to anyone it believes is local,
//! and a relayed request that arrived claiming to be local would hand a
//! household app to the entire internet.
//!
//! [`ViewerOrigin::address`] is reported for the access log and for telling two
//! readers apart, on exactly the terms the daemon already uses an address for
//! (HANDOFF section 3.1): it decides which row of a list somebody appears on,
//! and it authorises nothing.
//!
//! # Version skew is the normal case
//!
//! The daemon ships inside an application on other people's machines and
//! updates when they let it; the edge redeploys whenever we like. The two are
//! never the same age. [`VersionRange::negotiate`] settles it per connection,
//! and unknown JSON fields are ignored rather than rejected, so a newer peer
//! adding a field does not break an older one. See [`version`].
//!
//! [repo-architecture.md]: https://github.com/kitchen-table/kitchentable

pub mod codec;
pub mod control;
pub mod hello;
pub mod stream;
pub mod version;

pub use codec::{decode, encode, try_decode, CodecError, MAX_FRAME_LEN};
pub use control::{
    CloseReason, ControlFrame, Retry, HEARTBEAT_INTERVAL_SECS, HEARTBEAT_TIMEOUT_SECS,
    RECONNECT_MAX_SECS, RECONNECT_MIN_SECS,
};
pub use hello::{
    hello_transcript, Accepted, ClientHello, HelloError, InstallIdentity, InstallKey, Nonce,
    ServerHello, NONCE_LEN,
};
pub use stream::{HttpRequestHead, HttpResponseHead, StreamOpen, ViewerOrigin};
pub use version::{VersionError, VersionRange, MIN_PROTOCOL_VERSION, PROTOCOL_VERSION};

/// Fixed-size byte strings travel as unpadded base64url in JSON.
///
/// Unpadded because these values end up in URLs, headers and logs often enough
/// that `=` is a nuisance, and it is what `kt-auth` already uses for session
/// cookies. The length is checked on the way in: a key or a signature of the
/// wrong size is a malformed message, not something to discover later inside a
/// cryptography library.
pub(crate) mod b64 {
    use base64ct::{Base64UrlUnpadded, Encoding};
    use serde::{de::Error as _, Deserialize, Deserializer, Serializer};

    pub fn serialize<S, const N: usize>(bytes: &[u8; N], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&Base64UrlUnpadded::encode_string(bytes))
    }

    pub fn deserialize<'de, D, const N: usize>(deserializer: D) -> Result<[u8; N], D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        let decoded = Base64UrlUnpadded::decode_vec(&encoded)
            .map_err(|_| D::Error::custom("expected unpadded base64url"))?;
        let len = decoded.len();
        decoded
            .try_into()
            .map_err(|_: Vec<u8>| D::Error::custom(format!("expected {N} bytes, found {len}")))
    }
}
