//! Events the daemon pushes to subscribed clients over the same socket.
//!
//! The shell forwards these to the UI as Tauri events, which is what makes the
//! library update itself without polling and what pops the pairing prompt
//! (docs/architecture.md section 4).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{app::App, status::ServingState};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "event", rename_all = "snake_case")]
#[ts(export)]
pub enum Event {
    /// An app appeared, changed, or its manifest was rewritten.
    AppChanged { app: App },
    /// An app folder left the workspace.
    AppRemoved { slug: String },
    /// Serving started, stopped, or degraded. Drives the tray and the banner.
    ServingChanged { serving: ServingState },
    /// A device is waiting to be let in. This is what pops the approval prompt
    /// (docs/architecture.md section 4).
    ///
    /// Carries the id rather than the device: `device.list` is already the one
    /// description of a device on the wire, and a second one here could
    /// disagree with it. The window fetches, and gets the network-discovered
    /// name if it has landed by then.
    PairingRequest { device_id: String },
    /// Somebody opened an app, was turned away, or had a decision made about
    /// them. Drives the Activity view without polling.
    ///
    /// Deliberately thin. The access log is the record; this only says that it
    /// changed, so nothing here has to be kept in step with the log's own shape.
    Access {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        app_slug: Option<String>,
        action: String,
    },
    /// Who has an app open changed: somebody arrived, left, or moved to
    /// another page.
    ///
    /// Carries the whole list rather than a delta. It is a handful of people at
    /// most, and a list that replaces the previous one cannot drift out of step
    /// with it the way a stream of arrivals and departures can.
    Presence {
        app_slug: String,
        viewers: Vec<Viewer>,
    },
    /// Something went wrong that the owner should see, outside any one request.
    Error { message: String },
}

/// Somebody with an app open right now.
///
/// Known only because the page said so - see the `live` module in kt-server.
/// The device is named by id; the window resolves it against `device.list`, so
/// there is one description of a device rather than two that can disagree.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Viewer {
    pub device_id: String,
    /// The path they are on, e.g. `/budget`. Never the page's contents.
    pub path: String,
    /// Seconds since they last checked in. Small by construction: anyone who
    /// stops checking in drops off the list entirely.
    pub seconds_ago: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_tagged_by_name() {
        let e = Event::AppRemoved {
            slug: "trip-planner".into(),
        };
        let json = serde_json::to_value(&e).expect("serialises");
        assert_eq!(json["event"], "app_removed");
        assert_eq!(json["slug"], "trip-planner");
    }
}
