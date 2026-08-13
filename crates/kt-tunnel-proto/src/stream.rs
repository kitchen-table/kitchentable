//! What travels down a stream once the connection is up.
//!
//! One tunnel connection carries many streams (yamux, opened by the edge), one
//! per viewer request, plus a single long-lived control stream. Multiplexing is
//! why a family opening six apps at once is one connection and not six: a home
//! router's NAT table and a laptop's battery both care about that, and the
//! reconnect logic only has one thing to get right.
//!
//! Each stream opens with a length-prefixed [`StreamOpen`] and then becomes
//! raw bytes: request body up, [`HttpResponseHead`] and response body down.
//! Bodies are never parsed, buffered whole, or interpreted here. A stream ends
//! when the response ends.
//!
//! # A relayed request is an internet request
//!
//! Read [`HttpRequestHead`] before adding a field to it.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

/// The first message on a newly opened stream, saying what it is for.
///
/// Internally tagged, and with no catch-all: a kind this build does not know
/// is a protocol error rather than something to skip past, because there is no
/// safe default behaviour for a stream whose purpose is unknown. That is the
/// deliberate opposite of the rule for [`crate::CloseReason`], where carrying
/// on is exactly right.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stream", rename_all = "snake_case")]
pub enum StreamOpen {
    /// The one control stream, opened once per connection and held open for as
    /// long as it lasts. Carries [`crate::ControlFrame`].
    Control,
    /// A viewer's request, on its own stream.
    Http(HttpRequestHead),
}

/// A request the edge received from a viewer, on its way to the daemon.
///
/// # There is no field here saying this is local, and there never will be
///
/// The gate in `kt-auth` allows a Network-visibility app to anyone it believes
/// is on the household network, and allows anything at all to the owner's own
/// machine on loopback. A request that arrived down this tunnel is neither. It
/// came from the internet, by way of a server in a datacentre, and the daemon
/// must build its `RequestContext` with `on_household_network` and
/// `is_loopback` both false.
///
/// So this type carries nothing that could be mistaken for evidence to the
/// contrary. Not because the edge would lie, but because the failure mode is
/// silent and total: a Household app is one that somebody deliberately chose
/// not to publish, and the whole visibility model exists to keep that choice
/// meaningful. A field here that could carry "local" would put the entire
/// internet one bug away from the family's chore rota.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRequestHead {
    /// The public hostname the viewer asked for, which is how the daemon knows
    /// which app this is.
    ///
    /// Per repo-architecture.md section 8, app identity comes from the host and
    /// never from a parameter. The edge has already used this to choose which
    /// tunnel to send the request down, so a daemon receiving a hostname it
    /// does not serve should refuse rather than guess.
    pub hostname: String,
    pub method: String,
    /// Path and query exactly as received, undecoded. The daemon canonicalises
    /// and refuses escapes, as it does for a request off the LAN; nothing here
    /// has been made safe on its behalf.
    pub path: String,
    /// In order, and repeats preserved: `Set-Cookie` and friends are legally
    /// repeated and collapsing them changes meaning.
    pub headers: Vec<(String, String)>,
    pub viewer: ViewerOrigin,
}

/// Where a request came from, for the access log and for telling readers apart.
///
/// Not an authorisation input, on the same terms the daemon already applies to
/// addresses on its own network (HANDOFF section 3.1): the gate has decided by
/// the time any of this is used, and an address only decides which row of a
/// list somebody appears on. Here it is weaker still, having been asserted by a
/// viewer, observed by the edge, and forwarded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerOrigin {
    /// The peer address the edge saw, where it saw one worth reporting.
    ///
    /// Optional because it can genuinely be absent - and because a daemon that
    /// has to handle its absence anyway will not grow a dependence on its
    /// presence.
    pub address: Option<IpAddr>,
    /// Whether the viewer's own connection to the edge was TLS.
    ///
    /// Always true in practice, and sent anyway so the daemon reports what
    /// happened rather than what should have happened. A relayed app that ever
    /// sees `false` here has a problem worth surfacing.
    pub tls: bool,
}

/// The daemon's answer, ahead of the body bytes.
///
/// Status and headers only. Whether this is a 200 with a file, a 302 into the
/// pairing flow, or the 503 of a paused app is entirely the daemon's business:
/// the edge forwards what it is given and adds nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpResponseHead {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn head() -> HttpRequestHead {
        HttpRequestHead {
            hostname: "trip-adarsh.kitchentable.cloud".into(),
            method: "GET".into(),
            path: "/packing?tab=1".into(),
            headers: vec![("accept".into(), "text/html".into())],
            viewer: ViewerOrigin {
                address: Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9))),
                tls: true,
            },
        }
    }

    #[test]
    fn a_request_survives_a_round_trip() {
        let open = StreamOpen::Http(head());
        let json = serde_json::to_string(&open).unwrap();
        assert_eq!(serde_json::from_str::<StreamOpen>(&json).unwrap(), open);
    }

    #[test]
    fn the_control_stream_carries_no_payload() {
        let json = serde_json::to_string(&StreamOpen::Control).unwrap();
        assert_eq!(json, r#"{"stream":"control"}"#);
    }

    #[test]
    fn a_request_head_says_nothing_about_being_local() {
        // The guard for the invariant in this module's docs. A serialised head
        // must not contain a field the daemon could read as household or
        // loopback trust; adding one has to fail here first.
        let json = serde_json::to_string(&StreamOpen::Http(head())).unwrap();
        for forbidden in [
            "loopback",
            "local",
            "household",
            "trusted",
            "on_household_network",
            "is_loopback",
        ] {
            assert!(
                !json.contains(forbidden),
                "a request arriving over the relay must not be able to claim {forbidden}"
            );
        }
    }

    #[test]
    fn repeated_headers_keep_their_order_and_their_repeats() {
        let mut request = head();
        request.headers = vec![
            ("cookie".into(), "a=1".into()),
            ("cookie".into(), "b=2".into()),
        ];
        let json = serde_json::to_string(&request).unwrap();
        let back: HttpRequestHead = serde_json::from_str(&json).unwrap();
        assert_eq!(back.headers, request.headers);
    }

    #[test]
    fn a_stream_kind_this_build_does_not_know_is_an_error() {
        // Unlike a close reason, there is no carrying on from here.
        let json = r#"{"stream":"quic_datagram","hostname":"x"}"#;
        assert!(serde_json::from_str::<StreamOpen>(json).is_err());
    }

    #[test]
    fn a_viewer_with_no_address_is_representable() {
        let mut request = head();
        request.viewer.address = None;
        let json = serde_json::to_string(&request).unwrap();
        let back: HttpRequestHead = serde_json::from_str(&json).unwrap();
        assert_eq!(back.viewer.address, None);
    }

    #[test]
    fn ipv6_viewers_round_trip() {
        let mut request = head();
        request.viewer.address = Some("2001:db8::1".parse().unwrap());
        let json = serde_json::to_string(&request).unwrap();
        let back: HttpRequestHead = serde_json::from_str(&json).unwrap();
        assert_eq!(back.viewer.address, request.viewer.address);
    }

    #[test]
    fn the_path_is_forwarded_undecoded() {
        // Canonicalisation is the daemon's job and it has a traversal suite for
        // it. The edge must not helpfully normalise on the way past.
        let mut request = head();
        request.path = "/%2e%2e/%2e%2e/etc/passwd".into();
        let json = serde_json::to_string(&request).unwrap();
        let back: HttpRequestHead = serde_json::from_str(&json).unwrap();
        assert_eq!(back.path, "/%2e%2e/%2e%2e/etc/passwd");
    }

    #[test]
    fn a_response_head_round_trips() {
        let response = HttpResponseHead {
            status: 503,
            headers: vec![("retry-after".into(), "120".into())],
        };
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(
            serde_json::from_str::<HttpResponseHead>(&json).unwrap(),
            response
        );
    }
}
