//! The visibility gate: what happens to a request before a file is read.
//!
//! Every inbound request passes through here. The decision itself lives in
//! kt-auth so it has one definition and one test matrix; this module is the
//! part that knows about cookies, headers and HTML.

use axum::http::{header, HeaderMap};
use kt_auth::{Decision, DenyReason, Device, DeviceId, RequestContext, SessionKeys};
use kt_types::{RelayMode, Visibility};

/// The cookie carrying a session. `__Host-` would be stricter but requires
/// HTTPS, which is not available until the local CA is trusted, so the name is
/// plain until then.
pub const SESSION_COOKIE: &str = "kt_session";

/// What the gate wants done with a request.
#[derive(Debug, PartialEq)]
pub enum Gated {
    /// Serve the file.
    Serve,
    /// Show the wait page; the owner is being asked.
    Waiting,
    /// A dead end, with the page to show.
    Refused(Refusal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NotShared,
    WrongNetwork,
    Revoked,
    /// The owner took it offline. Nothing the viewer can do about it, so the
    /// page says so plainly rather than implying they are unwelcome.
    Paused,
    /// Asked for from outside, for an app whose owner never published it.
    ///
    /// Answered as a 404 rather than a 403, and that is a decision rather than
    /// an oversight: from the internet this app genuinely does not exist, and
    /// saying "forbidden" would confirm that it does to anybody guessing names.
    NotPublished,
}

/// Look up the session cookie, if any, and verify it.
pub fn session_device(headers: &HeaderMap, keys: &SessionKeys, now: i64) -> Option<DeviceId> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;

    // A browser sends every matching cookie in one header; find ours without
    // pulling in a cookie crate for one lookup.
    let raw = cookies.split(';').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name.trim() == SESSION_COOKIE).then(|| value.trim())
    })?;

    keys.verify(raw, now).ok().map(|token| token.device)
}

/// Decide what to do with a request for an app.
pub fn gate(
    visibility: Visibility,
    paused: bool,
    relay: RelayMode,
    device: Option<&Device>,
    ctx: &RequestContext,
) -> Gated {
    match kt_auth::decide(visibility, paused, relay, device, ctx) {
        Decision::Allow => Gated::Serve,
        Decision::NeedsPairing => Gated::Waiting,
        Decision::Deny(DenyReason::WrongNetwork) => Gated::Refused(Refusal::WrongNetwork),
        Decision::Deny(DenyReason::DeviceRevoked) => Gated::Refused(Refusal::Revoked),
        Decision::Deny(DenyReason::NotTheOwner) => Gated::Refused(Refusal::NotShared),
        Decision::Deny(DenyReason::Paused) => Gated::Refused(Refusal::Paused),
        Decision::Deny(DenyReason::NotPublished) => Gated::Refused(Refusal::NotPublished),
    }
}

/// A `Set-Cookie` value for a freshly minted session.
///
/// `SameSite=Lax` rather than `Strict` because the whole product is people
/// following links from WhatsApp, and `Strict` would drop the cookie on exactly
/// that navigation. Lax plus the required write header is the CSRF story
/// (docs/architecture.md section 14).
pub fn session_cookie(value: &str, secure: bool) -> String {
    let mut cookie = format!(
        "{SESSION_COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        kt_auth::session::SESSION_TTL_SECONDS
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Whether a request came from this machine.
///
/// The owner's own browser must always be able to open a private app without
/// pairing with itself - and it reaches this daemon by more than one address.
/// `localhost` is loopback, but `chores-rota.local` resolves through Bonjour to
/// this machine's *LAN* address, and so did not count: an owner opening their
/// own Private app from the window was told "Not available" on their own Mac.
///
/// `own` is the address this machine currently holds, asked for per request
/// rather than remembered. That is the safe direction: an address read once at
/// startup goes stale when DHCP moves, and the stale value could later belong
/// to somebody else's laptop, which would hand them the owner's exemption.
///
/// **Why an address is trusted here when it is trusted nowhere else.** A device
/// asserts its identity; it does not assert its address. This one comes from the
/// accepted socket, and a request whose source is one of *our own* addresses
/// started on this machine - the kernel routes traffic to a local address
/// internally, and a different host cannot both claim that address and complete
/// a TCP handshake from it. It is still only ever a route to the owner's own
/// exemption, never to anybody else's.
pub fn is_loopback(addr: Option<std::net::IpAddr>, own: Option<std::net::IpAddr>) -> bool {
    addr.is_some_and(|ip| ip.is_loopback() || own.is_some_and(|mine| mine == ip))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kt_auth::DeviceStatus;

    fn headers_with(cookie: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, cookie.parse().expect("valid header"));
        headers
    }

    #[test]
    fn a_valid_cookie_yields_its_device() {
        let keys = SessionKeys::generate();
        let device = DeviceId::generate();
        let cookie = keys.mint(&device, 1000);

        let found = session_device(
            &headers_with(&format!("{SESSION_COOKIE}={cookie}")),
            &keys,
            1000,
        );
        assert_eq!(found, Some(device));
    }

    #[test]
    fn our_cookie_is_found_among_others() {
        let keys = SessionKeys::generate();
        let device = DeviceId::generate();
        let cookie = keys.mint(&device, 1000);

        let header = format!("other=1; {SESSION_COOKIE}={cookie}; another=2");
        assert_eq!(
            session_device(&headers_with(&header), &keys, 1000),
            Some(device)
        );
    }

    #[test]
    fn a_forged_cookie_yields_nothing() {
        let keys = SessionKeys::generate();
        let elsewhere = SessionKeys::generate();
        let cookie = elsewhere.mint(&DeviceId::generate(), 1000);

        assert_eq!(
            session_device(
                &headers_with(&format!("{SESSION_COOKIE}={cookie}")),
                &keys,
                1000
            ),
            None
        );
    }

    #[test]
    fn no_cookie_at_all_is_fine() {
        let keys = SessionKeys::generate();
        assert_eq!(session_device(&HeaderMap::new(), &keys, 1000), None);
        assert_eq!(session_device(&headers_with("junk"), &keys, 1000), None);
    }

    #[test]
    fn the_cookie_survives_a_link_from_a_chat_app() {
        // SameSite=Strict would drop the cookie on the cross-site navigation
        // that is this product's entire distribution mechanism.
        let cookie = session_cookie("abc", false);
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("HttpOnly"));
        assert!(!cookie.contains("Secure"), "not over plain http");

        assert!(session_cookie("abc", true).contains("Secure"));
    }

    #[test]
    fn an_unknown_device_on_an_invited_app_waits_rather_than_being_refused() {
        let ctx = RequestContext::default();
        assert_eq!(
            gate(Visibility::Invited, false, RelayMode::Off, None, &ctx),
            Gated::Waiting
        );
    }

    #[test]
    fn each_refusal_maps_to_its_own_page() {
        let away = RequestContext {
            on_household_network: false,
            ..Default::default()
        };
        assert_eq!(
            gate(Visibility::Network, false, RelayMode::Off, None, &away),
            Gated::Refused(Refusal::WrongNetwork)
        );
        assert_eq!(
            gate(Visibility::Private, false, RelayMode::Off, None, &away),
            Gated::Refused(Refusal::NotShared)
        );

        let revoked = Device::pending(DeviceId::generate(), "test", 0);
        let revoked = Device {
            status: DeviceStatus::Revoked,
            ..revoked
        };
        assert_eq!(
            gate(
                Visibility::Public,
                false,
                RelayMode::Off,
                Some(&revoked),
                &away
            ),
            Gated::Refused(Refusal::Revoked)
        );
    }

    #[test]
    fn the_owners_own_browser_never_has_to_pair_with_itself() {
        let local = RequestContext {
            is_loopback: true,
            ..Default::default()
        };
        for visibility in [
            Visibility::Private,
            Visibility::Network,
            Visibility::Invited,
            Visibility::Public,
        ] {
            assert_eq!(
                gate(visibility, false, RelayMode::Off, None, &local),
                Gated::Serve,
                "{visibility:?}"
            );
        }
    }

    #[test]
    fn this_machine_is_loopback_or_the_address_this_machine_holds() {
        use std::net::IpAddr;
        let mine: Option<IpAddr> = Some(IpAddr::from([192, 168, 0, 10]));

        assert!(is_loopback(Some(IpAddr::from([127, 0, 0, 1])), None));
        assert!(is_loopback(Some("::1".parse().expect("valid")), None));

        // The case that was refusing the owner on their own Mac: `.local`
        // resolves through Bonjour to this machine's LAN address.
        assert!(is_loopback(Some(IpAddr::from([192, 168, 0, 10])), mine));

        // And the rest of the house is still the rest of the house. One
        // address off is a different machine.
        assert!(!is_loopback(Some(IpAddr::from([192, 168, 0, 11])), mine));
        assert!(!is_loopback(Some(IpAddr::from([192, 168, 0, 5])), None));
        assert!(!is_loopback(None, mine));
    }
}
