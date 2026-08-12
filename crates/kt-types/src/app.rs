//! Apps: the on-disk manifest, and the view of an app that clients get.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Who can open an app (docs/product.md section 5.4).
///
/// `Network` is the wire value; the UI labels it **Household**. The rename is
/// display-only on purpose: keeping the stored value stable means the level can
/// be retired later as a UI and validation change rather than a migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Visibility {
    /// Owner's own devices only. The default for a folder that has never been
    /// shared: nothing becomes reachable by appearing in the workspace.
    #[default]
    Private,
    /// Anyone on a network the owner has marked as home. Apps at this level
    /// auto-pause when the machine joins an unrecognised network.
    Network,
    /// Holders of a valid invite link, on approved devices.
    Invited,
    /// Anyone with the URL. Only meaningful once the relay is on.
    Public,
}

impl Visibility {
    /// The label shown to people. Only `Network` differs from its wire value.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Private => "Private",
            Self::Network => "Household",
            Self::Invited => "Invited",
            Self::Public => "Public",
        }
    }

    /// Whether opening this app requires the daemon to check a session.
    pub const fn requires_session(self) -> bool {
        matches!(self, Self::Private | Self::Invited)
    }
}

/// The on-disk `app.json` (docs/architecture.md section 8).
///
/// Everything is optional: a folder with no manifest gets one generated, with
/// the name taken from the folder and visibility Private. The daemon owns
/// `version`; humans and agents own the rest.
///
/// Not exported to TypeScript. This is the daemon's view of a file on disk;
/// clients read [`App`], which carries the derived fields they actually need.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppManifest {
    pub name: String,
    pub slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default = "default_entry")]
    pub entry: String,
    #[serde(default)]
    pub visibility: Visibility,
    /// Owned by the daemon; incremented on deploy.
    #[serde(default)]
    pub version: u32,
    /// Unknown keys are preserved verbatim so a manifest written by a newer
    /// daemon survives a round-trip through an older one.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn default_entry() -> String {
    "index.html".to_string()
}

/// An app as clients see it: the manifest plus what the daemon knows about
/// serving it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct App {
    pub slug: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub icon: Option<String>,
    pub entry: String,
    pub visibility: Visibility,
    pub version: u32,
    /// Friendly URL, e.g. `https://trip-planner.local`.
    pub url: String,
    /// Always-available IP-and-port URL, for networks where mDNS does not
    /// resolve. Android is the usual reason (docs/architecture.md section 10).
    pub fallback_url: String,
    /// Absolute path to the app folder in the workspace.
    pub path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_manifest_gets_sensible_defaults() {
        let m: AppManifest =
            serde_json::from_str(r#"{"name":"Trip planner","slug":"trip-planner"}"#)
                .expect("parses");
        assert_eq!(m.entry, "index.html");
        assert_eq!(m.visibility, Visibility::Private);
        assert_eq!(m.version, 0);
    }

    #[test]
    fn unknown_keys_survive_a_round_trip() {
        let src = r#"{"name":"N","slug":"s","futureField":{"a":1}}"#;
        let m: AppManifest = serde_json::from_str(src).expect("parses");
        let out = serde_json::to_value(&m).expect("serialises");
        assert_eq!(out["futureField"]["a"], 1);
    }

    #[test]
    fn network_is_the_wire_value_and_household_is_the_label() {
        assert_eq!(
            serde_json::to_string(&Visibility::Network).expect("serialises"),
            r#""network""#
        );
        assert_eq!(Visibility::Network.label(), "Household");
    }

    #[test]
    fn only_private_and_invited_need_a_session() {
        assert!(Visibility::Private.requires_session());
        assert!(Visibility::Invited.requires_session());
        assert!(!Visibility::Network.requires_session());
        assert!(!Visibility::Public.requires_session());
    }
}
