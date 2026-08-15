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

use kt_types::{RelayMode, Visibility};

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
    /// Whether this request originated on the owner's own machine.
    ///
    /// Loopback is the obvious case and was for a while the only one, which
    /// made the owner a stranger to their own app the moment they reached it by
    /// its `.local` name: that resolves to this machine's *LAN* address, so a
    /// browser two inches away was refused a Private app. The test is now "did
    /// this start here", which `Peer::context` answers with loopback **or** an
    /// address this machine currently holds. The name is kept because renaming
    /// a field the whole matrix turns on is a change worth making on its own.
    pub is_loopback: bool,
    /// Whether this request arrived over the relay - that is, from the
    /// internet, by way of a server in a datacentre.
    ///
    /// It is never inferred here. `Peer::context` in kt-server sets it, and it
    /// only ever takes trust away: a relayed request already arrives with
    /// `on_household_network` and `is_loopback` false by construction, and this
    /// field is what additionally requires the owner to have published the app.
    pub arrived_by_relay: bool,
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
    /// The app exists and is perfectly healthy, but its owner has not published
    /// it to the relay - so from outside this network it does not exist.
    ///
    /// Distinct from every other reason here because it is not about the
    /// caller at all, and because the honest answer to the internet is that
    /// there is nothing at this address rather than that there is something
    /// they may not have.
    NotPublished,
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
    relay: RelayMode,
    device: Option<&Device>,
    ctx: &RequestContext,
) -> Decision {
    // A paused app answers nobody - not the owner, not an approved device, not
    // this machine. It is the one refusal that says nothing about the caller,
    // which is why it is checked before anything is known about them.
    if paused {
        return Decision::Deny(DenyReason::Paused);
    }

    // Nothing leaves the house unless its owner said so. Checked here, before
    // anything about the caller is examined, because - like pausing - it is a
    // fact about the app rather than about who is asking.
    //
    // This is what stops a folder appearing in the workspace from appearing on
    // the internet. Visibility decides who may open an app; this decides
    // whether the question can even be asked from outside, and no combination
    // of visibility, device or invite gets round it.
    if ctx.arrived_by_relay && !relay.is_published() {
        return Decision::Deny(DenyReason::NotPublished);
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

        // Anyone with the URL. Which URLs exist is the relay's question, asked
        // above: off the relay this is the local network, and on it, anywhere.
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
            arrived_by_relay: false,
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
            for relay in [RelayMode::Off, RelayMode::Standard, RelayMode::Strict] {
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
                            let got = decide(
                                visibility,
                                true,
                                relay,
                                device.as_ref(),
                                &ctx(loopback, household),
                            );
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
    }

    /// An unpublished app is not reachable from the internet, whatever else is
    /// true of it.
    ///
    /// The same shape as the pausing test above, and the same claim: the relay
    /// is not a visibility level, it is a switch, and no combination of
    /// visibility, device, invite or network gets round it. This is what stops
    /// a folder appearing in somebody's workspace from appearing on the
    /// internet.
    ///
    /// Note the refusal is `NotPublished` rather than any of the others - it
    /// renders as a 404, because from outside the app genuinely does not exist
    /// and saying "forbidden" would confirm to a stranger that it does.
    #[test]
    fn an_unpublished_app_is_refused_every_relayed_request() {
        use Visibility::*;

        for visibility in [Private, Network, Invited, Public] {
            for status in [
                None,
                Some(DeviceStatus::Owner),
                Some(DeviceStatus::Approved),
                Some(DeviceStatus::Pending),
                Some(DeviceStatus::Revoked),
            ] {
                let device = status.map(device);
                let mut relayed = ctx(false, false);
                relayed.arrived_by_relay = true;

                assert_eq!(
                    decide(visibility, false, RelayMode::Off, device.as_ref(), &relayed),
                    Decision::Deny(DenyReason::NotPublished),
                    "{visibility:?}/{status:?} arriving over the relay should be refused \
                     while the app is unpublished"
                );
            }
        }
    }

    /// Publishing an app does not publish the ones beside it.
    ///
    /// The switch is per app, so the only thing that changes for a relayed
    /// request is the app that was switched on. Both modes behave identically
    /// here: Standard and Strict differ in how bytes travel, never in who may
    /// have them.
    #[test]
    fn publishing_changes_nothing_about_who_may_open_it() {
        use Visibility::*;

        for relay in [RelayMode::Standard, RelayMode::Strict] {
            let mut relayed = ctx(false, false);
            relayed.arrived_by_relay = true;

            // Public is the level that exists to be reachable by anyone.
            assert_eq!(
                decide(Public, false, relay, None, &relayed),
                Decision::Allow,
                "a published Public app should be served over the relay"
            );

            // Invited still means invited. Publishing an app does not approve
            // anybody's device, and an unknown viewer gets the pairing flow
            // rather than the app.
            assert_eq!(
                decide(Invited, false, relay, None, &relayed),
                Decision::NeedsPairing
            );
            assert_eq!(
                decide(
                    Invited,
                    false,
                    relay,
                    Some(&device(DeviceStatus::Approved)),
                    &relayed
                ),
                Decision::Allow
            );

            // And the two levels that are about *place* remain unreachable
            // from anywhere else, which is why the UI does not offer them the
            // relay at all. Publishing one cannot make it work.
            assert_eq!(
                decide(Network, false, relay, None, &relayed),
                Decision::Deny(DenyReason::WrongNetwork)
            );
            assert_eq!(
                decide(Private, false, relay, None, &relayed),
                Decision::Deny(DenyReason::NotTheOwner)
            );
        }
    }

    /// A request that did not arrive over the relay is unaffected by the switch.
    ///
    /// The free tier is the whole product for most people, and it must not
    /// change because a paid feature exists.
    #[test]
    fn local_requests_do_not_care_whether_the_relay_is_on() {
        use Visibility::*;

        for visibility in [Private, Network, Invited, Public] {
            for loopback in [true, false] {
                for household in [true, false] {
                    let local = ctx(loopback, household);
                    let baseline = decide(visibility, false, RelayMode::Off, None, &local);

                    for relay in [RelayMode::Standard, RelayMode::Strict] {
                        assert_eq!(
                            decide(visibility, false, relay, None, &local),
                            baseline,
                            "{visibility:?}/loopback={loopback}/household={household} \
                             changed answer because the relay was {relay:?}"
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
                RelayMode::Off,
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
                decide(
                    visibility,
                    false,
                    RelayMode::Off,
                    Some(&revoked),
                    &ctx(false, true)
                ),
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
            decide(
                Visibility::default(),
                false,
                RelayMode::Off,
                None,
                &ctx(false, true)
            ),
            Decision::Deny(DenyReason::NotTheOwner)
        );
    }
}
