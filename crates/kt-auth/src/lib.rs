//! Who may open an app, and how that is decided.
//!
//! Three pieces fit together. A **device** is a browser that has been approved
//! once and remembered. A **session** is a signed cookie proving which device
//! is asking. An **invite** is a link that lets a new device ask in the first
//! place.
//!
//! Sessions are Ed25519-signed rather than HMAC'd, which costs nothing here and
//! buys something later: the relay's edge can verify a session against a public
//! key the daemon registers, while only the daemon can mint one. Verification
//! without minting power is the property that lets snapshots be served safely
//! (server-architecture.md section 3).

use kt_types::Visibility;

pub mod device;
pub mod invite;
pub mod session;

pub use device::{Device, DeviceId, DeviceStatus, NamedBy};
pub use invite::{Invite, InviteError, InvitePolicy, InviteToken};
pub use session::{SessionError, SessionKeys, SessionToken};

/// Everything the gate knows about one request.
#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    /// A valid session cookie, if one was presented and verified.
    pub session: Option<SessionToken>,
    /// Whether the request arrived from a network the owner marked as home.
    pub on_household_network: bool,
    /// Whether this request came from the owner's own machine (loopback).
    pub is_loopback: bool,
}

/// What the gate decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Serve it.
    Allow,
    /// Send them through the pairing flow: they may be allowed, but this
    /// device is not known yet.
    NeedsPairing,
    /// No, and no path forward from here.
    Deny(DenyReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// Private, and this is not the owner.
    NotTheOwner,
    /// Household, but the machine is not on a network the owner trusts.
    WrongNetwork,
    /// The device was explicitly revoked; a new invite is required.
    DeviceRevoked,
    /// The owner took the app offline. Nothing to do with who is asking.
    Paused,
}

/// Decide whether a request may open an app.
///
/// The whole visibility model in one function, deliberately: scattering these
/// rules across middleware is how a level ends up meaning something subtly
/// different on one code path than another. Its test matrix is the security
/// test for the product.
pub fn decide(
    visibility: Visibility,
    paused: bool,
    device: Option<&Device>,
    ctx: &RequestContext,
) -> Decision {
    // A paused app answers nobody - not the owner, not an approved device, not
    // this machine. It is the one refusal that says nothing about the caller,
    // which is why it is checked before anything is known about them.
    if paused {
        return Decision::Deny(DenyReason::Paused);
    }

    // A revoked device is refused everywhere, whatever the level says.
    // Revocation that any visibility level could override would not be
    // revocation.
    if let Some(device) = device {
        if device.status == DeviceStatus::Revoked {
            return Decision::Deny(DenyReason::DeviceRevoked);
        }
    }

    match visibility {
        // The owner's own machine always counts: it is where the files are.
        Visibility::Private => {
            if ctx.is_loopback || is_owner(device) {
                Decision::Allow
            } else if device.is_some() {
                Decision::Deny(DenyReason::NotTheOwner)
            } else {
                // An unknown device asking for a private app is refused rather
                // than offered pairing: pairing is for apps that are shared.
                Decision::Deny(DenyReason::NotTheOwner)
            }
        }

        // Bound to place, not to whatever network the laptop is on right now
        // (product.md section 5.4).
        Visibility::Network => {
            if ctx.is_loopback || ctx.on_household_network {
                Decision::Allow
            } else {
                Decision::Deny(DenyReason::WrongNetwork)
            }
        }

        Visibility::Invited => match device {
            Some(d) if d.status == DeviceStatus::Approved => Decision::Allow,
            // Approval is pending, or this device is unknown. Either way the
            // answer is the wait page, not a refusal.
            Some(_) | None => {
                if ctx.is_loopback {
                    Decision::Allow
                } else {
                    Decision::NeedsPairing
                }
            }
        },

        // Only meaningful once the relay is on; locally it behaves like
        // Household without the network check.
        Visibility::Public => Decision::Allow,
    }
}

fn is_owner(device: Option<&Device>) -> bool {
    device.is_some_and(|d| d.status == DeviceStatus::Owner)
}

#[cfg(test)]
mod matrix {
    use super::*;

    fn ctx(loopback: bool, household: bool) -> RequestContext {
        RequestContext {
            session: None,
            on_household_network: household,
            is_loopback: loopback,
        }
    }

    fn device(status: DeviceStatus) -> Device {
        Device::new_for_test("d1", status)
    }

    /// Pausing beats everything the matrix says.
    ///
    /// Rather than doubling every row above, this runs the same table again
    /// with the app paused and asserts the answer is always the same refusal.
    /// That is the claim worth making: taking an app offline is not a level,
    /// it is a switch, and no combination of visibility, device or network
    /// gets round it - including the owner on this very machine.
    #[test]
    fn pausing_overrides_every_row_of_the_matrix() {
        use Visibility::*;

        for visibility in [Private, Network, Invited, Public] {
            for status in [
                None,
                Some(DeviceStatus::Owner),
                Some(DeviceStatus::Approved),
                Some(DeviceStatus::Pending),
                Some(DeviceStatus::Revoked),
            ] {
                for loopback in [true, false] {
                    for household in [true, false] {
                        let device = status.map(device);
                        let got =
                            decide(visibility, true, device.as_ref(), &ctx(loopback, household));
                        assert_eq!(
                            got,
                            Decision::Deny(DenyReason::Paused),
                            "{visibility:?}/{status:?}/loopback={loopback}/household={household} \
                             should be refused while paused"
                        );
                    }
                }
            }
        }
    }

    /// Every visibility level against every kind of caller.
    ///
    /// CLAUDE.md makes this matrix the definition of done for anything touching
    /// kt-auth: extending auth means extending this table.
    #[test]
    fn the_visibility_matrix() {
        use DenyReason::*;
        use Visibility::*;

        // (visibility, device, loopback, household) -> decision
        let cases: &[(Visibility, Option<DeviceStatus>, bool, bool, Decision)] = &[
            // -- Private: the owner, and nobody else -----------------------
            (Private, None, true, false, Decision::Allow),
            (
                Private,
                Some(DeviceStatus::Owner),
                false,
                false,
                Decision::Allow,
            ),
            (
                Private,
                Some(DeviceStatus::Approved),
                false,
                true,
                Decision::Deny(NotTheOwner),
            ),
            (Private, None, false, true, Decision::Deny(NotTheOwner)),
            // -- Household: bound to place --------------------------------
            (Network, None, false, true, Decision::Allow),
            (
                Network,
                Some(DeviceStatus::Approved),
                false,
                true,
                Decision::Allow,
            ),
            // The travelling-laptop case: same device, café wifi.
            (
                Network,
                Some(DeviceStatus::Approved),
                false,
                false,
                Decision::Deny(WrongNetwork),
            ),
            (Network, None, false, false, Decision::Deny(WrongNetwork)),
            (Network, None, true, false, Decision::Allow),
            // -- Invited: approved devices, others get the pairing flow ----
            (
                Invited,
                Some(DeviceStatus::Approved),
                false,
                false,
                Decision::Allow,
            ),
            (
                Invited,
                Some(DeviceStatus::Pending),
                false,
                false,
                Decision::NeedsPairing,
            ),
            (Invited, None, false, false, Decision::NeedsPairing),
            // Being on the home network is not itself an invitation.
            (Invited, None, false, true, Decision::NeedsPairing),
            (Invited, None, true, false, Decision::Allow),
            // -- Public: anyone --------------------------------------------
            (Public, None, false, false, Decision::Allow),
            (
                Public,
                Some(DeviceStatus::Pending),
                false,
                false,
                Decision::Allow,
            ),
        ];

        for (visibility, status, loopback, household, expected) in cases {
            let device = status.map(device);
            let got = decide(
                *visibility,
                false,
                device.as_ref(),
                &ctx(*loopback, *household),
            );
            assert_eq!(
                got, *expected,
                "{visibility:?} + {status:?} (loopback={loopback}, household={household})"
            );
        }
    }

    #[test]
    fn a_revoked_device_is_refused_at_every_level() {
        // Revocation that a visibility level could override would not be
        // revocation. Public included: the point of revoking is that this
        // browser stops being able to open your things.
        for visibility in [
            Visibility::Private,
            Visibility::Network,
            Visibility::Invited,
            Visibility::Public,
        ] {
            let revoked = device(DeviceStatus::Revoked);
            assert_eq!(
                decide(visibility, false, Some(&revoked), &ctx(false, true)),
                Decision::Deny(DenyReason::DeviceRevoked),
                "{visibility:?} let a revoked device through"
            );
        }
    }

    #[test]
    fn nothing_is_reachable_by_default() {
        // A folder that has never been shared is Private, and Private refuses
        // everyone but the owner. This is the property that makes dropping a
        // folder into the workspace safe.
        assert_eq!(
            decide(Visibility::default(), false, None, &ctx(false, true)),
            Decision::Deny(DenyReason::NotTheOwner)
        );
    }
}
