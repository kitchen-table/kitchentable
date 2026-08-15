//! The daemon's answer to "who is this, and may they?"
//!
//! kt-server asks the questions, kt-auth decides, kt-store remembers. This is
//! the wiring between them.

use std::sync::Arc;

use axum::http::{header, HeaderMap};
use kt_auth::{Device, DeviceId, DeviceStatus, InviteToken, SessionKeys};
use kt_server::{Redemption, TrustSource};
use kt_store::{AccessEvent, Store};
use kt_types::Event;

pub struct Trust {
    store: Arc<Store>,
    keys: Arc<SessionKeys>,
    /// Asks the network what a device calls itself. Used once per device, off
    /// the request path, purely to suggest a better name than the user agent.
    discoverer: Arc<dyn kt_mdns::Discoverer>,
    /// Where pairing requests and access go, so the window does not have to
    /// poll for either.
    events: crate::rpc::Events,
    /// Whether an address this machine holds counts as the owner's own
    /// machine, as well as loopback. Off only for the end-to-end suite; see
    /// `own_address`.
    trust_own_address: bool,
    /// Whether this machine is on a network the owner marked as home.
    ///
    /// Fixed for now. Remembering networks by gateway fingerprint is the next
    /// piece of the Household level, and until it exists the honest default is
    /// to treat the current network as home - the alternative would refuse
    /// every household app on every machine.
    household: bool,
}

impl Trust {
    pub fn new(store: Arc<Store>, keys: Arc<SessionKeys>, events: crate::rpc::Events) -> Self {
        Self::with_discoverer(
            store,
            keys,
            events,
            Arc::from(kt_mdns::discoverer_for_this_platform()),
        )
    }

    pub fn with_discoverer(
        store: Arc<Store>,
        keys: Arc<SessionKeys>,
        events: crate::rpc::Events,
        discoverer: Arc<dyn kt_mdns::Discoverer>,
    ) -> Self {
        Self {
            store,
            keys,
            events,
            discoverer,
            household: true,
            trust_own_address: std::env::var("KT_NO_OWN_ADDRESS").is_err(),
        }
    }

    /// Ask the network for this device's own name, and adopt it if it answers.
    ///
    /// On its own thread because the lookup takes up to a few seconds when
    /// nothing replies, and the caller is an HTTP handler with a phone waiting
    /// on the other end of it. The name lands before the owner reaches the
    /// prompt in practice, and if it does not, the prompt shows the guess -
    /// which is what it showed before any of this existed.
    fn discover_name(&self, id: DeviceId, peer: Option<std::net::IpAddr>) {
        let Some(std::net::IpAddr::V4(addr)) = peer else {
            return;
        };
        if addr.is_loopback() || !self.discoverer.is_live() {
            return;
        }

        let discoverer = Arc::clone(&self.discoverer);
        let store = Arc::clone(&self.store);
        std::thread::spawn(move || {
            let Some(name) = discoverer.name_for(addr, kt_mdns::NAME_LOOKUP_BUDGET) else {
                return;
            };
            // `set_discovered_name` only replaces a guess, so an owner who
            // typed a name while this was in flight is not overwritten.
            match store.set_discovered_name(&id, &name) {
                Ok(true) => tracing::info!(device = id.as_str(), name, "device named itself"),
                Ok(false) => {}
                Err(e) => tracing::warn!(error = %e, "could not store the discovered name"),
            }
        });
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default()
    }

    /// Find or create the device behind a request, minting a session if it is
    /// new. Returns the device and a cookie when one was minted.
    fn identify(&self, headers: &HeaderMap) -> (Device, Option<String>) {
        let now = Self::now();
        let user_agent = headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();

        if let Some(id) = kt_server::gate::session_device(headers, &self.keys, now) {
            if let Ok(Some(mut existing)) = self.store.get_device(&id) {
                existing.last_seen = now;
                let _ = self.store.upsert_device(&existing);
                return (existing, None);
            }
        }

        // No cookie, or a cookie for a device we no longer have. Either way
        // this is a new device asking.
        let device = Device::pending(DeviceId::generate(), user_agent, now);
        if let Err(e) = self.store.upsert_device(&device) {
            tracing::warn!(error = %e, "could not record a new device");
        }
        // Somebody is holding a phone waiting for this. The window polled for
        // it every two seconds before events existed, which is up to two
        // seconds of someone staring at a wait page for no reason.
        self.events.send(Event::PairingRequest {
            device_id: device.id.as_str().to_string(),
        });
        let cookie = kt_server::gate::session_cookie(&self.keys.mint(&device.id, now), false);
        (device, Some(cookie))
    }
}

impl TrustSource for Trust {
    fn device_for(&self, headers: &HeaderMap) -> Option<Device> {
        let id = kt_server::gate::session_device(headers, &self.keys, Self::now())?;
        self.store.get_device(&id).ok().flatten()
    }

    fn on_household_network(&self) -> bool {
        self.household
    }

    /// The same lookup that builds the fallback URL: ask the routing table
    /// which source address it would use, without sending anything.
    ///
    /// Per request on purpose. It is a UDP socket bind and a `connect` that
    /// puts no packet on the wire, and asking each time is what keeps this
    /// honest across a DHCP change - a remembered address could later belong to
    /// somebody else's laptop, and this is the one place an address grants the
    /// owner's exemption.
    ///
    /// `KT_NO_OWN_ADDRESS` turns the widened exemption off, leaving loopback
    /// only. It exists for the end-to-end suite, whose one way of playing a
    /// stranger is to connect to this machine's LAN address from this machine -
    /// which is precisely what this now recognises as the owner. **It can only
    /// ever take trust away**, which is why it is a switch rather than a way to
    /// name an address: a variable that could nominate one would be a way to
    /// hand a stranger the owner's exemption.
    fn own_address(&self) -> Option<std::net::IpAddr> {
        if self.trust_own_address {
            kt_mdns::primary_ipv4().map(std::net::IpAddr::V4)
        } else {
            None
        }
    }

    fn redeem(&self, headers: &HeaderMap, token: &str) -> Result<Redemption, String> {
        let now = Self::now();

        let token = InviteToken::parse(token).ok_or("That link is not valid.")?;
        let invite = self
            .store
            .get_invite(&token)
            .map_err(|_| "That link is not valid.")?
            .ok_or("That link is not valid.")?;

        // Reuse the session if this browser already has one. Opening the same
        // link twice is ordinary behaviour, and minting a second identity
        // would make a pinned link report itself as forwarded.
        let (device, fresh_cookie) = self.identify(headers);

        // Rate limiting per source lands with the connection info in the next
        // step; zero here means "no attempts recorded yet".
        let outcome = invite
            .check(&device.id, now, 0)
            .map_err(|e| e.to_string())?;

        let pinned = outcome.pins_now.then_some(&device.id);
        self.store
            .record_redemption(&token, pinned)
            .map_err(|_| "Something went wrong on this machine.")?;

        // A link may waive the approval prompt on a household network, so
        // handing someone a link across the kitchen table is one tap.
        let auto_approve = invite.policy.auto_approve_on_household && self.household;
        if auto_approve {
            let _ = self
                .store
                .set_device_status(&device.id, DeviceStatus::Approved);
        }

        self.log(
            &invite.app_slug,
            Some(&device.id),
            if auto_approve { "paired" } else { "requested" },
        );

        // Already-approved devices skip the wait page entirely.
        let approved = auto_approve || device.status == DeviceStatus::Approved;

        Ok(Redemption {
            cookie: fresh_cookie,
            app_slug: invite.app_slug,
            pending: !approved,
        })
    }

    fn request_access(
        &self,
        headers: &HeaderMap,
        slug: &str,
        peer: Option<std::net::IpAddr>,
    ) -> Redemption {
        let (device, cookie) = self.identify(headers);

        // Only log the first ask, and only look the name up once. The wait page
        // refreshes every few seconds: an access log full of one viewer waiting
        // is unreadable, and a lookup per refresh would hammer the network.
        if cookie.is_some() {
            self.log(slug, Some(&device.id), "requested");
            self.discover_name(device.id.clone(), peer);
        }

        Redemption {
            cookie,
            app_slug: slug.to_string(),
            pending: true,
        }
    }

    fn log(&self, slug: &str, device: Option<&DeviceId>, action: &str) {
        let event = AccessEvent {
            at: Self::now(),
            app_slug: Some(slug.to_string()),
            device_id: device.map(|d| d.as_str().to_string()),
            actor: "viewer".to_string(),
            action: action.to_string(),
            detail: None,
        };
        if let Err(e) = self.store.log_access(&event) {
            tracing::warn!(error = %e, "could not write to the access log");
        }
        self.events.send(Event::Access {
            app_slug: Some(slug.to_string()),
            action: action.to_string(),
        });
    }

    fn presence_changed(&self, slug: &str, viewers: Vec<kt_types::Viewer>) {
        self.events.send(Event::Presence {
            app_slug: slug.to_string(),
            viewers,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kt_auth::{Invite, InvitePolicy};

    fn trust() -> Trust {
        Trust::new(
            Arc::new(Store::in_memory().expect("opens")),
            Arc::new(SessionKeys::generate()),
            crate::rpc::Events::new(),
        )
    }

    #[test]
    fn an_unknown_visitor_gets_an_identity_and_a_cookie() {
        let trust = trust();
        let (device, cookie) = trust.identify(&HeaderMap::new());

        assert_eq!(device.status, DeviceStatus::Pending);
        assert!(
            cookie.is_some(),
            "a new device needs a session to be recognised"
        );
        assert!(trust.store.get_device(&device.id).expect("reads").is_some());
    }

    #[test]
    fn a_returning_visitor_keeps_their_identity() {
        let trust = trust();
        let (first, cookie) = trust.identify(&HeaderMap::new());

        let mut headers = HeaderMap::new();
        let value = cookie
            .expect("minted")
            .split(';')
            .next()
            .expect("has a pair")
            .to_string();
        headers.insert(header::COOKIE, value.parse().expect("valid"));

        let (second, cookie) = trust.identify(&headers);
        assert_eq!(first.id, second.id, "the same browser is the same device");
        assert!(cookie.is_none(), "no need to re-mint an existing session");
    }

    #[test]
    fn redeeming_a_link_records_the_device_and_leaves_it_pending() {
        let trust = trust();
        let invite = Invite::new(
            "trip",
            "For book club",
            InvitePolicy::default(),
            Trust::now(),
        );
        trust.store.create_invite(&invite).expect("creates");

        let redemption = trust
            .redeem(&HeaderMap::new(), invite.token.as_str())
            .expect("redeems");

        assert_eq!(redemption.app_slug, "trip");
        assert!(redemption.pending, "the owner still has to approve");
        assert!(redemption.cookie.is_some());
    }

    #[test]
    fn a_household_link_can_waive_the_prompt() {
        let trust = trust();
        let invite = Invite::new(
            "trip",
            "At the table",
            InvitePolicy {
                auto_approve_on_household: true,
                ..Default::default()
            },
            Trust::now(),
        );
        trust.store.create_invite(&invite).expect("creates");

        let redemption = trust
            .redeem(&HeaderMap::new(), invite.token.as_str())
            .expect("redeems");
        assert!(
            !redemption.pending,
            "a household link lets them straight in"
        );
    }

    #[test]
    fn a_forwarded_pinned_link_is_refused() {
        let trust = trust();
        let invite = Invite::new(
            "trip",
            "For book club",
            InvitePolicy::default(),
            Trust::now(),
        );
        trust.store.create_invite(&invite).expect("creates");

        // Two genuinely different browsers: neither presents a cookie, so
        // each gets its own identity.
        trust
            .redeem(&HeaderMap::new(), invite.token.as_str())
            .expect("first redemption");
        let second = trust.redeem(&HeaderMap::new(), invite.token.as_str());

        assert!(
            second.is_err(),
            "a pinned link must not work from a second device"
        );
    }

    #[test]
    fn a_nonsense_token_is_refused_without_touching_the_database() {
        let trust = trust();
        assert!(trust.redeem(&HeaderMap::new(), "../../etc/passwd").is_err());
        assert!(trust.redeem(&HeaderMap::new(), "").is_err());
    }

    #[test]
    fn reopening_the_link_you_were_sent_still_works() {
        // The bug this guards: minting a new identity on every redemption made
        // a pinned link refuse the very person it was pinned to, the second
        // time they opened it.
        let trust = trust();
        let invite = Invite::new(
            "trip",
            "For book club",
            InvitePolicy::default(),
            Trust::now(),
        );
        trust.store.create_invite(&invite).expect("creates");

        let first = trust
            .redeem(&HeaderMap::new(), invite.token.as_str())
            .expect("first redemption");

        let mut headers = HeaderMap::new();
        let value = first
            .cookie
            .expect("minted")
            .split(';')
            .next()
            .expect("pair")
            .to_string();
        headers.insert(header::COOKIE, value.parse().expect("valid"));

        let again = trust.redeem(&headers, invite.token.as_str());
        assert!(again.is_ok(), "the same browser must be let back in");
    }

    #[test]
    fn an_approved_device_skips_the_wait_page() {
        let trust = trust();
        let invite = Invite::new(
            "trip",
            "For book club",
            InvitePolicy::default(),
            Trust::now(),
        );
        trust.store.create_invite(&invite).expect("creates");

        let first = trust
            .redeem(&HeaderMap::new(), invite.token.as_str())
            .expect("redeems");
        assert!(first.pending);

        // The owner approves, and the same browser comes back.
        let devices = trust.store.list_devices().expect("lists");
        let pending = devices.first().expect("a device");
        trust
            .store
            .set_device_status(&pending.id, DeviceStatus::Approved)
            .expect("approves");

        let mut headers = HeaderMap::new();
        let value = first
            .cookie
            .expect("minted")
            .split(';')
            .next()
            .expect("pair")
            .to_string();
        headers.insert(header::COOKIE, value.parse().expect("valid"));

        let again = trust
            .redeem(&headers, invite.token.as_str())
            .expect("redeems");
        assert!(!again.pending, "an approved device should go straight in");
    }

    #[test]
    fn access_is_logged() {
        let trust = trust();
        let (device, _) = trust.identify(&HeaderMap::new());
        trust.log("trip", Some(&device.id), "opened");

        let events = trust.store.recent_access(Some("trip"), 10).expect("reads");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "opened");
        assert_eq!(events[0].device_id.as_deref(), Some(device.id.as_str()));
    }

    #[test]
    fn a_waiting_viewer_is_logged_once_not_on_every_refresh() {
        // The wait page reloads every few seconds; logging each one would bury
        // everything else in the activity feed.
        let trust = trust();
        let first = trust.request_access(&HeaderMap::new(), "trip", None);

        let mut headers = HeaderMap::new();
        let value = first
            .cookie
            .expect("minted")
            .split(';')
            .next()
            .expect("pair")
            .to_string();
        headers.insert(header::COOKIE, value.parse().expect("valid"));

        for _ in 0..5 {
            trust.request_access(&headers, "trip", None);
        }

        let events = trust.store.recent_access(Some("trip"), 50).expect("reads");
        assert_eq!(events.len(), 1, "only the first ask should be logged");
    }
}
