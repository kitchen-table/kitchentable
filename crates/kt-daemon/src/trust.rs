//! The daemon's answer to "who is this, and may they?"
//!
//! kt-server asks the questions, kt-auth decides, kt-store remembers. This is
//! the wiring between them.

use std::sync::Arc;

use axum::http::{header, HeaderMap};
use kt_auth::{Device, DeviceId, DeviceStatus, InviteToken, SessionKeys};
use kt_server::{Redemption, TrustSource};
use kt_store::{AccessEvent, Store};

pub struct Trust {
    store: Arc<Store>,
    keys: Arc<SessionKeys>,
    /// Whether this machine is on a network the owner marked as home.
    ///
    /// Fixed for now. Remembering networks by gateway fingerprint is the next
    /// piece of the Household level, and until it exists the honest default is
    /// to treat the current network as home - the alternative would refuse
    /// every household app on every machine.
    household: bool,
}

impl Trust {
    pub fn new(store: Arc<Store>, keys: Arc<SessionKeys>) -> Self {
        Self {
            store,
            keys,
            household: true,
        }
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

    fn redeem(&self, token: &str, user_agent: &str) -> Result<Redemption, String> {
        let now = Self::now();

        let token = InviteToken::parse(token).ok_or("That link is not valid.")?;
        let invite = self
            .store
            .get_invite(&token)
            .map_err(|_| "That link is not valid.")?
            .ok_or("That link is not valid.")?;

        // A device asking for the first time gets an identity before the
        // invite is checked, because pinning needs something to pin to.
        let device = Device::pending(DeviceId::generate(), user_agent, now);
        self.store
            .upsert_device(&device)
            .map_err(|_| "Something went wrong on this machine.")?;

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

        Ok(Redemption {
            cookie: Some(kt_server::gate::session_cookie(
                &self.keys.mint(&device.id, now),
                false,
            )),
            app_slug: invite.app_slug,
            pending: !auto_approve,
        })
    }

    fn request_access(&self, headers: &HeaderMap, slug: &str) -> Redemption {
        let (device, cookie) = self.identify(headers);

        // Only log the first ask. The wait page refreshes every few seconds,
        // and an access log full of one viewer waiting is unreadable.
        if cookie.is_some() {
            self.log(slug, Some(&device.id), "requested");
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
        let invite = Invite::new("trip", "For Sam", InvitePolicy::default(), Trust::now());
        trust.store.create_invite(&invite).expect("creates");

        let redemption = trust
            .redeem(invite.token.as_str(), "Mozilla/5.0 (iPhone)")
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
            .redeem(invite.token.as_str(), "iPhone")
            .expect("redeems");
        assert!(
            !redemption.pending,
            "a household link lets them straight in"
        );
    }

    #[test]
    fn a_forwarded_pinned_link_is_refused() {
        let trust = trust();
        let invite = Invite::new("trip", "For Sam", InvitePolicy::default(), Trust::now());
        trust.store.create_invite(&invite).expect("creates");

        trust
            .redeem(invite.token.as_str(), "iPhone")
            .expect("first redemption");
        let second = trust.redeem(invite.token.as_str(), "Android");

        assert!(second.is_err(), "a pinned link must not work twice");
    }

    #[test]
    fn a_nonsense_token_is_refused_without_touching_the_database() {
        let trust = trust();
        assert!(trust.redeem("../../etc/passwd", "curl").is_err());
        assert!(trust.redeem("", "curl").is_err());
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
        let first = trust.request_access(&HeaderMap::new(), "trip");

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
            trust.request_access(&headers, "trip");
        }

        let events = trust.store.recent_access(Some("trip"), 50).expect("reads");
        assert_eq!(events.len(), 1, "only the first ask should be logged");
    }
}
