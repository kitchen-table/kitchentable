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
    /// Something went wrong that the owner should see, outside any one request.
    Error { message: String },
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
