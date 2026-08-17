//! Which desktop notifications the owner wants, as `notify.prefs` returns them.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The four switches on the Notifications surface.
///
/// Daemon-side rather than a browser preference, because the thing being
/// configured is not a property of a window: the daemon keeps serving with
/// every window closed, and an owner who turned access requests off expects
/// that to still hold after an update replaces the shell. It also means the CLI
/// and an agent read the same answer the window shows.
///
/// Defaults are the mockup's: everything on except opens, which the mockup
/// itself labels "can get chatty" - one banner per page load is the only
/// setting here that can plausibly drown the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NotifyPrefs {
    /// A device is at the door. The one that needs an answer, so it is also the
    /// only one that carries buttons.
    pub requests: bool,
    /// Somebody opened an app.
    pub opens: bool,
    /// A deploy finished. Nothing writes a deploy yet (product.md 5.10), so
    /// this is stored and honoured and simply never fires until something does.
    pub deploys: bool,
    /// An app stopped being reachable. Not detected yet either; see above.
    pub offline: bool,
}

impl Default for NotifyPrefs {
    fn default() -> Self {
        Self {
            requests: true,
            opens: false,
            deploys: true,
            offline: true,
        }
    }
}

impl NotifyPrefs {
    /// Apply whichever switches a caller sent, leaving the rest alone.
    ///
    /// A partial update rather than a whole-object write: two windows open on
    /// the same daemon would otherwise race, and the one that saved last would
    /// silently undo a switch the other had just moved.
    pub fn merge(mut self, patch: &serde_json::Value) -> Self {
        let take = |name: &str, field: &mut bool| {
            if let Some(value) = patch.get(name).and_then(serde_json::Value::as_bool) {
                *field = value;
            }
        };
        take("requests", &mut self.requests);
        take("opens", &mut self.opens);
        take("deploys", &mut self.deploys);
        take("offline", &mut self.offline);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_only_moves_the_switches_it_names() {
        let prefs = NotifyPrefs::default().merge(&serde_json::json!({ "opens": true }));

        assert!(prefs.opens, "the named switch moved");
        assert!(prefs.requests, "an unnamed switch kept its value");
        assert!(prefs.deploys);
        assert!(prefs.offline);
    }

    #[test]
    fn nonsense_in_a_patch_is_ignored_rather_than_an_error() {
        // A caller sending the wrong shape should not be able to reset
        // somebody's notification settings by accident.
        let prefs =
            NotifyPrefs::default().merge(&serde_json::json!({ "requests": "no", "nope": true }));

        assert!(prefs.requests);
    }
}
