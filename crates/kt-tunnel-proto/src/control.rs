//! Keeping a connection alive, and ending it in a way the other end can act on.
//!
//! These travel on the single control stream ([`crate::StreamOpen::Control`]).
//!
//! # Why heartbeats are in the protocol and not left to the transport
//!
//! A home connection does not usually announce that it has gone. A router
//! reboots, a laptop lid closes, a NAT table drops an idle entry, and the
//! socket at the far end stays open for hours with nothing arriving on it. The
//! edge cannot wait that long to notice, because a product promise hangs on
//! the answer: when a daemon is offline, a Standard-mode app is served from its
//! snapshot with an honest badge saying the owner's machine is asleep, which
//! the README describes. "Is this daemon there?" is therefore a
//! question with a user-visible answer, and the timing of it belongs in the
//! agreement between the two ends rather than in whichever keepalive a
//! transport library happens to default to.

use serde::{Deserialize, Serialize};

use crate::version::VersionRange;

/// How often each side sends a [`ControlFrame::Ping`].
pub const HEARTBEAT_INTERVAL_SECS: u32 = 20;

/// How long silence lasts before the other end is presumed gone.
///
/// Three intervals. Two would make one lost packet on a phone-tethered
/// connection look like a sleeping laptop, and the visible cost of being wrong
/// is asymmetric: declaring a live daemon dead swaps a live app for a stale
/// snapshot badged as asleep, which is a worse lie than being a minute late to
/// notice a real absence.
pub const HEARTBEAT_TIMEOUT_SECS: u32 = HEARTBEAT_INTERVAL_SECS * 3;

/// First reconnect delay, before jitter.
pub const RECONNECT_MIN_SECS: u32 = 1;

/// The ceiling on reconnect backoff.
///
/// Both ends agree the schedule because a reconnect storm is a shared problem,
/// and an expensive one in both directions: every attempt costs a full TLS
/// handshake, and a daemon retrying in a tight loop is not distinguishable
/// from one trying to do harm.
/// Daemons must back off exponentially from [`RECONNECT_MIN_SECS`] to here and
/// must jitter, so that an edge deploy does not bring every daemon in the world
/// back in the same second.
pub const RECONNECT_MAX_SECS: u32 = 60;

// Checked at compile time rather than in a test, because getting either of
// these wrong is a typo with a user-visible consequence - a live app shown as
// asleep, or every daemon in the world reconnecting in the same second - and a
// typo should stop the build rather than wait for someone to run the suite.
const _: () = assert!(HEARTBEAT_TIMEOUT_SECS >= HEARTBEAT_INTERVAL_SECS * 3);
const _: () = assert!(RECONNECT_MIN_SECS < RECONNECT_MAX_SECS);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "control", rename_all = "snake_case")]
pub enum ControlFrame {
    /// Are you there. The sequence number is echoed, so a reply can be matched
    /// to its ping and round-trip time measured rather than guessed.
    Ping {
        seq: u32,
    },
    Pong {
        seq: u32,
    },
    /// I am going away, and here is why. Sent when there is time to send it;
    /// its absence is not an error, because the common way a connection ends is
    /// that a lid closes.
    Goodbye {
        reason: CloseReason,
    },
    /// A frame from a newer peer that this build does not know.
    ///
    /// Ignoring it is right: control frames are advisory, and a daemon that
    /// dropped its connection because the edge learned a new one would turn
    /// every forward-compatible change into an outage.
    #[serde(other)]
    Unknown,
}

/// Why a tunnel connection ended.
///
/// The point of naming these is [`CloseReason::retry`]. A daemon that reconnects
/// after every failure hammers the edge forever when the real answer is that
/// this install was revoked an hour ago; a daemon that gives up on every failure
/// stays down after a routine deploy. The difference is not something the daemon
/// can work out for itself, so the edge says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum CloseReason {
    /// The edge is draining for a deploy. Routine, and frequent: come back.
    Draining,
    /// No version in common.
    VersionUnsupported { supported: VersionRange },
    /// The hello did not verify.
    AuthenticationFailed,
    /// A valid key, but not one linked to any account.
    UnknownInstall,
    /// Linked once and deliberately unlinked since.
    InstallRevoked,
    /// The subscription lapsed. Distinct from revocation because it comes back
    /// on its own when someone updates a card, and the desktop app has
    /// something specific and useful to say about it.
    SubscriptionInactive,
    /// Another connection presented the same install key and won.
    Superseded,
    /// The edge is shedding load.
    Overloaded,
    /// The peer sent something that does not parse or does not belong here.
    ProtocolError { detail: String },
    /// A reason from a newer edge that this build has never heard of.
    ///
    /// Deliberately last, and deliberately retryable-with-backoff: an unknown
    /// reason from a newer edge is far more likely to be a transient condition
    /// than a permanent judgement, and backing off is the answer that is merely
    /// slow when it is wrong rather than an outage.
    #[serde(other)]
    Unknown,
}

impl CloseReason {
    /// What the daemon should do next.
    pub const fn retry(&self) -> Retry {
        match self {
            // Expected, and over in seconds. Still jittered: a deploy drains
            // every daemon at once, and they must not all come back together.
            Self::Draining | Self::Superseded => Retry::Soon,

            // Might clear on its own, or with an edge deploy, or when a card is
            // updated. Keep a connection attempt in the diary, rarely.
            Self::Overloaded | Self::SubscriptionInactive | Self::Unknown => Retry::Backoff,

            // Nothing about waiting fixes any of these. Retrying is a bill and
            // a log full of noise, and it delays the daemon telling the person
            // in front of it the one thing that would help.
            Self::VersionUnsupported { .. }
            | Self::AuthenticationFailed
            | Self::UnknownInstall
            | Self::InstallRevoked => Retry::Never,

            // A bug on one side or the other. Reconnecting into the same bug is
            // how a reconnect storm starts, so it backs off like everything
            // else that might clear.
            Self::ProtocolError { .. } => Retry::Backoff,
        }
    }

    /// Whether the person using the desktop app has to do something.
    ///
    /// These are the states where the relay is off and staying off until
    /// somebody acts, so the window should say so plainly rather than showing a
    /// spinner that will never stop.
    pub const fn needs_the_owner(&self) -> bool {
        matches!(
            self,
            Self::UnknownInstall
                | Self::InstallRevoked
                | Self::SubscriptionInactive
                | Self::VersionUnsupported { .. }
        )
    }
}

/// What to do about a closed connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retry {
    /// Reconnect from the bottom of the backoff schedule, with jitter.
    Soon,
    /// Reconnect on the full exponential schedule, with jitter.
    Backoff,
    /// Do not reconnect. Tell the owner instead.
    Never,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_that_needs_a_human_is_retried_in_a_loop() {
        // The rule this enum exists for. A daemon that keeps dialling an edge
        // which has already refused it for good is a bill, a noisy log, and a
        // window that never gets round to saying what is wrong.
        for reason in [
            CloseReason::AuthenticationFailed,
            CloseReason::UnknownInstall,
            CloseReason::InstallRevoked,
            CloseReason::VersionUnsupported {
                supported: VersionRange::new(2, 3),
            },
        ] {
            assert_eq!(reason.retry(), Retry::Never, "{reason:?}");
        }
    }

    #[test]
    fn a_deploy_is_a_blip_and_not_an_outage() {
        assert_eq!(CloseReason::Draining.retry(), Retry::Soon);
    }

    #[test]
    fn a_superseded_connection_does_not_fight_the_one_that_replaced_it() {
        // Soon rather than immediately, and jittered by the daemon, or two
        // connections take turns evicting each other.
        assert_eq!(CloseReason::Superseded.retry(), Retry::Soon);
    }

    #[test]
    fn an_unknown_reason_backs_off_rather_than_giving_up() {
        // A newer edge will invent reasons this build has never seen. Guessing
        // "permanent" would strand daemons on a transient condition; guessing
        // "immediately" would hammer. Backoff is wrong in the least bad way.
        let json = r#"{"reason":"quota_exceeded_this_month"}"#;
        let reason: CloseReason = serde_json::from_str(json).unwrap();
        assert_eq!(reason, CloseReason::Unknown);
        assert_eq!(reason.retry(), Retry::Backoff);
    }

    #[test]
    fn an_unknown_control_frame_is_ignored_rather_than_fatal() {
        let json = r#"{"control":"bandwidth_hint","bytes":4096}"#;
        let frame: ControlFrame = serde_json::from_str(json).unwrap();
        assert_eq!(frame, ControlFrame::Unknown);
    }

    #[test]
    fn the_states_worth_interrupting_someone_for_are_the_ones_they_can_fix() {
        assert!(CloseReason::SubscriptionInactive.needs_the_owner());
        assert!(CloseReason::InstallRevoked.needs_the_owner());
        assert!(!CloseReason::Draining.needs_the_owner());
        assert!(!CloseReason::Overloaded.needs_the_owner());
        // Nothing the owner can do about a bug on either side.
        assert!(!CloseReason::ProtocolError {
            detail: "bad frame".into()
        }
        .needs_the_owner());
    }

    #[test]
    fn a_version_rejection_carries_what_the_edge_speaks() {
        // So the daemon can say "update Kitchen Table" rather than "the relay
        // said no", which is the difference between a fix and a support email.
        let reason = CloseReason::VersionUnsupported {
            supported: VersionRange::new(3, 4),
        };
        let json = serde_json::to_string(&reason).unwrap();
        let back: CloseReason = serde_json::from_str(&json).unwrap();
        assert_eq!(back, reason);
    }

    #[test]
    fn pings_carry_a_sequence_so_a_reply_can_be_matched_to_them() {
        let frame = ControlFrame::Ping { seq: 42 };
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(json, r#"{"control":"ping","seq":42}"#);
        assert_eq!(serde_json::from_str::<ControlFrame>(&json).unwrap(), frame);
    }
}
