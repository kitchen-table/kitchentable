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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// An app as the daemon holds it: the manifest, plus where it lives.
///
/// The persistent half. URLs are deliberately absent because they depend on the
/// port and hostname the daemon happens to have this run, so they are derived
/// into [`App`] at the edge of the socket rather than stored.
#[derive(Debug, Clone, PartialEq)]
pub struct AppRecord {
    pub manifest: AppManifest,
    /// Absolute path to the app folder.
    pub path: String,
    /// Total size of the folder's contents, in bytes.
    pub size_bytes: u64,
    /// When the content last changed, in unix seconds. `None` when the
    /// filesystem will not say - some network mounts, and Linux before 4.11.
    pub deployed_at: Option<u64>,
}

impl AppRecord {
    /// A record whose folder has not been walked.
    ///
    /// `size_bytes` and `deployed_at` are measured from disk, which only the
    /// registry does. Everything else that builds a record - the store reading
    /// a row, tests - goes through here, so a zero size is visibly a
    /// "not measured" rather than a claim that the folder is empty.
    pub fn unmeasured(manifest: AppManifest, path: String) -> Self {
        Self {
            manifest,
            path,
            size_bytes: 0,
            deployed_at: None,
        }
    }

    /// Derive the client-facing view.
    ///
    /// `hostname` is the name actually announced on the network, which may be
    /// suffixed if another device already had it. When there is none - mDNS
    /// unavailable, permission denied, or an unsupported platform - the app is
    /// still reachable by prefix, so the URL falls back rather than vanishing.
    pub fn to_app(&self, urls: &Urls) -> App {
        let m = &self.manifest;
        let prefix_url = format!("{}/{}", urls.prefix_origin, m.slug);

        App {
            slug: m.slug.clone(),
            name: m.name.clone(),
            icon: m.icon.clone(),
            entry: m.entry.clone(),
            visibility: m.visibility,
            version: m.version,
            url: match &urls.hostname {
                Some(host) => format!("{}://{host}{}", urls.scheme, urls.port_suffix),
                None => prefix_url.clone(),
            },
            hostname: urls.hostname.clone(),
            fallback_url: format!("{}/{}", urls.fallback_origin, m.slug),
            path: self.path.clone(),
            // Saturating rather than wrapping: ts-rs maps u64 to bigint, which
            // JSON cannot carry, so these cross the wire as u32. An app folder
            // over 4 GB reports 4 GB, which is the right kind of wrong for a
            // number whose only job is to render as "12.4 MB".
            size_bytes: self.size_bytes.min(u32::MAX as u64) as u32,
            deployed_at: self.deployed_at.map(|t| t.min(u32::MAX as u64) as u32),
        }
    }
}

/// How to address one app this run.
#[derive(Debug, Clone, Default)]
pub struct Urls {
    /// `http` now; `https` once the local CA is trusted (D7).
    pub scheme: String,
    /// The announced `<name>.local`, if mDNS is live.
    pub hostname: Option<String>,
    /// `:8420`, or empty on the default port.
    pub port_suffix: String,
    /// Origin for prefix routing, e.g. `http://localhost`.
    pub prefix_origin: String,
    /// Origin that always resolves: the machine's IP and port.
    pub fallback_origin: String,
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
    /// Friendly URL, e.g. `http://trip-planner.local`. Falls back to the
    /// prefix URL when nothing was announced.
    pub url: String,
    /// The name announced on the network, absent when mDNS is not live. The UI
    /// shows this next to the slug when they differ, so a renamed app does not
    /// look like a bug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hostname: Option<String>,
    /// Always-available IP-and-port URL, for networks where mDNS does not
    /// resolve. Android is the usual reason (docs/architecture.md section 10).
    pub fallback_url: String,
    /// Absolute path to the app folder in the workspace.
    pub path: String,
    /// Bundle size in bytes, for the Overview tab. Saturates at `u32::MAX`.
    pub size_bytes: u32,
    /// Unix seconds when the content last changed. Absent when the filesystem
    /// does not report it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub deployed_at: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(hostname: Option<&str>) -> Urls {
        Urls {
            scheme: "http".into(),
            hostname: hostname.map(str::to_string),
            port_suffix: String::new(),
            prefix_origin: "http://localhost".into(),
            fallback_origin: "http://192.168.1.24:8420".into(),
        }
    }

    fn record() -> AppRecord {
        AppRecord {
            manifest: AppManifest {
                name: "Trip Planner".into(),
                slug: "trip-planner".into(),
                icon: None,
                entry: "index.html".into(),
                visibility: Visibility::Private,
                version: 1,
                extra: serde_json::Map::new(),
            },
            path: "/ws/trip".into(),
            size_bytes: 42_000,
            deployed_at: Some(1_760_000_000),
        }
    }

    #[test]
    fn an_announced_app_gets_its_own_hostname_url() {
        let app = record().to_app(&urls(Some("trip-planner.local")));
        assert_eq!(app.url, "http://trip-planner.local");
        assert_eq!(app.hostname.as_deref(), Some("trip-planner.local"));
    }

    #[test]
    fn without_mdns_the_url_falls_back_to_a_prefix_rather_than_vanishing() {
        let app = record().to_app(&urls(None));
        assert_eq!(app.url, "http://localhost/trip-planner");
        assert_eq!(app.hostname, None);
    }

    #[test]
    fn the_fallback_url_is_always_an_address_that_resolves() {
        // Whatever else happens, there is a URL a phone can open.
        for hostname in [Some("trip-planner.local"), None] {
            let app = record().to_app(&urls(hostname));
            assert_eq!(app.fallback_url, "http://192.168.1.24:8420/trip-planner");
        }
    }

    #[test]
    fn a_non_default_port_is_carried_into_the_hostname_url() {
        let mut u = urls(Some("trip-planner.local"));
        u.port_suffix = ":8420".into();
        assert_eq!(record().to_app(&u).url, "http://trip-planner.local:8420");
    }

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
