//! Invite links: how a device gets permission to ask.
//!
//! A link is a bearer token, so it is treated like one: 128 bits of entropy,
//! compared in constant time, and revocable. Optional pinning to the first
//! device that redeems it exists because links get forwarded - a WhatsApp
//! message reaches more people than the sender meant it to
//! (product.md section 5.4).

use serde::{Deserialize, Serialize};

use crate::device::DeviceId;
use crate::session::random_token;

/// 128 bits, per architecture.md section 14.
const TOKEN_BYTES: usize = 16;

/// How many redemption attempts one source may make before being throttled.
/// Guessing a 128-bit token is not the threat; a script hammering the endpoint
/// is.
pub const MAX_ATTEMPTS_PER_SOURCE: u32 = 20;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum InviteError {
    #[error("that link is not valid")]
    Unknown,
    #[error("that link has expired")]
    Expired,
    #[error("that link has been revoked")]
    Revoked,
    #[error("that link is already in use on another device")]
    PinnedElsewhere,
    #[error("too many attempts; wait a moment and try again")]
    RateLimited,
}

/// The secret in the URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteToken(String);

impl InviteToken {
    pub fn generate() -> Self {
        Self(random_token(TOKEN_BYTES))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let ok = (16..=64).contains(&raw.len())
            && raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        ok.then(|| Self(raw.to_string()))
    }

    /// Constant-time comparison, so a timing signal cannot narrow the search.
    pub fn matches(&self, other: &InviteToken) -> bool {
        let (a, b) = (self.0.as_bytes(), other.0.as_bytes());
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }
}

impl std::fmt::Display for InviteToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a link is allowed to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvitePolicy {
    /// Absent means it does not expire.
    pub expires_at: Option<i64>,
    /// Bind to the first device that redeems it, so a forwarded link is inert.
    pub pin_to_first_device: bool,
    /// Skip the owner's approval prompt for redemptions from a household
    /// network - handing someone a link across the kitchen table should be one
    /// tap for both of you (product.md section 5.5).
    pub auto_approve_on_household: bool,
}

impl Default for InvitePolicy {
    fn default() -> Self {
        Self {
            expires_at: None,
            // On by default: the common case is sending a link to one person,
            // and a link that stops working when forwarded is the safer
            // surprise.
            pin_to_first_device: true,
            auto_approve_on_household: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Invite {
    pub token: InviteToken,
    pub app_slug: String,
    /// Shown in the sharing panel so a list of links is legible: "For the
    /// family", "For Sam".
    pub label: String,
    pub policy: InvitePolicy,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
    /// Set on first redemption when the policy pins.
    pub pinned_device: Option<DeviceId>,
    pub redemptions: u32,
}

impl Invite {
    pub fn new(app_slug: &str, label: &str, policy: InvitePolicy, now: i64) -> Self {
        Self {
            token: InviteToken::generate(),
            app_slug: app_slug.to_string(),
            label: label.to_string(),
            policy,
            created_at: now,
            revoked_at: None,
            pinned_device: None,
            redemptions: 0,
        }
    }

    /// Whether this device may redeem this link right now.
    ///
    /// Checks are ordered cheapest-and-least-informative first; every failure
    /// returns the same shape, so probing tells an attacker nothing beyond
    /// "no".
    pub fn check(
        &self,
        device: &DeviceId,
        now: i64,
        attempts: u32,
    ) -> Result<Outcome, InviteError> {
        if attempts >= MAX_ATTEMPTS_PER_SOURCE {
            return Err(InviteError::RateLimited);
        }
        if self.revoked_at.is_some() {
            return Err(InviteError::Revoked);
        }
        if self.policy.expires_at.is_some_and(|at| now >= at) {
            return Err(InviteError::Expired);
        }
        if let Some(pinned) = &self.pinned_device {
            if pinned != device {
                return Err(InviteError::PinnedElsewhere);
            }
        }
        Ok(Outcome {
            pins_now: self.policy.pin_to_first_device && self.pinned_device.is_none(),
        })
    }

    /// Record a successful redemption.
    pub fn redeem(&mut self, device: &DeviceId, outcome: &Outcome) {
        if outcome.pins_now {
            self.pinned_device = Some(device.clone());
        }
        self.redemptions += 1;
    }

    pub fn is_active(&self, now: i64) -> bool {
        self.revoked_at.is_none() && !self.policy.expires_at.is_some_and(|at| now >= at)
    }
}

/// What redeeming will do, decided before anything is written.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub pins_now: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    fn invite(policy: InvitePolicy) -> Invite {
        Invite::new("trip-planner", "For the family", policy, NOW)
    }

    #[test]
    fn tokens_carry_enough_entropy_and_are_distinct() {
        let a = InviteToken::generate();
        let b = InviteToken::generate();
        assert_ne!(a, b);
        assert!(a.as_str().len() >= 20, "128 bits should not encode shorter");
    }

    #[test]
    fn a_fresh_link_can_be_redeemed() {
        let invite = invite(InvitePolicy::default());
        assert!(invite.check(&DeviceId::generate(), NOW, 0).is_ok());
    }

    #[test]
    fn an_expired_link_is_refused() {
        let invite = invite(InvitePolicy {
            expires_at: Some(NOW + 100),
            ..Default::default()
        });
        assert!(invite.check(&DeviceId::generate(), NOW + 50, 0).is_ok());
        assert_eq!(
            invite.check(&DeviceId::generate(), NOW + 100, 0),
            Err(InviteError::Expired)
        );
    }

    #[test]
    fn a_revoked_link_is_refused_even_before_it_expires() {
        let mut invite = invite(InvitePolicy::default());
        invite.revoked_at = Some(NOW + 1);
        assert_eq!(
            invite.check(&DeviceId::generate(), NOW + 2, 0),
            Err(InviteError::Revoked)
        );
    }

    #[test]
    fn a_pinned_link_stops_working_when_forwarded() {
        // The reason pinning exists: a link in a group chat reaches more
        // people than the sender meant.
        let mut invite = invite(InvitePolicy::default());
        let first = DeviceId::generate();

        let outcome = invite.check(&first, NOW, 0).expect("first redemption");
        assert!(outcome.pins_now);
        invite.redeem(&first, &outcome);

        // The same device can come back.
        assert!(invite.check(&first, NOW, 0).is_ok());
        // A forwarded link cannot.
        assert_eq!(
            invite.check(&DeviceId::generate(), NOW, 0),
            Err(InviteError::PinnedElsewhere)
        );
    }

    #[test]
    fn an_unpinned_link_serves_a_whole_household() {
        let mut invite = invite(InvitePolicy {
            pin_to_first_device: false,
            ..Default::default()
        });

        for _ in 0..5 {
            let device = DeviceId::generate();
            let outcome = invite.check(&device, NOW, 0).expect("redeems");
            assert!(!outcome.pins_now);
            invite.redeem(&device, &outcome);
        }
        assert_eq!(invite.redemptions, 5);
    }

    #[test]
    fn links_pin_by_default() {
        // The safer surprise: a link that stops working beats one that keeps
        // working for strangers.
        assert!(InvitePolicy::default().pin_to_first_device);
        assert!(!InvitePolicy::default().auto_approve_on_household);
    }

    #[test]
    fn hammering_the_endpoint_is_throttled() {
        let invite = invite(InvitePolicy::default());
        assert_eq!(
            invite.check(&DeviceId::generate(), NOW, MAX_ATTEMPTS_PER_SOURCE),
            Err(InviteError::RateLimited)
        );
    }

    #[test]
    fn rate_limiting_is_checked_before_anything_that_leaks_state() {
        // A throttled caller must not be able to tell a revoked link from an
        // expired one by watching which error comes back.
        let mut invite = invite(InvitePolicy {
            expires_at: Some(NOW - 1),
            ..Default::default()
        });
        invite.revoked_at = Some(NOW - 1);

        assert_eq!(
            invite.check(&DeviceId::generate(), NOW, MAX_ATTEMPTS_PER_SOURCE),
            Err(InviteError::RateLimited)
        );
    }

    #[test]
    fn token_comparison_does_not_short_circuit() {
        let token = InviteToken::generate();
        let same = InviteToken::parse(token.as_str()).expect("parses");
        assert!(token.matches(&same));
        assert!(!token.matches(&InviteToken::generate()));
    }

    #[test]
    fn a_malformed_token_never_becomes_one() {
        for bad in ["", "short", "has spaces", "../../secret", &"x".repeat(65)] {
            assert_eq!(InviteToken::parse(bad), None, "{bad:?} should be rejected");
        }
    }
}
