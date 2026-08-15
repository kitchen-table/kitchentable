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

    /// Whether a request that arrived over the relay could ever satisfy this
    /// level.
    ///
    /// Private is satisfiable only on loopback and Network means being on the
    /// home network. A relayed request is neither by construction, so for those
    /// two the relay can produce nothing but refusals - which makes a public
    /// address for them a URL that is guaranteed to fail, and a relay switch on
    /// them a control with no reachable effect.
    ///
    /// The rule lives here rather than in the window because it is a fact about
    /// the visibility model, and because the window was deciding it separately
    /// and getting a different answer.
    pub const fn reachable_over_relay(self) -> bool {
        matches!(self, Self::Invited | Self::Public)
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
    /// Taken offline by the owner, without being forgotten.
    ///
    /// Lives in the manifest beside `visibility` because it is the same kind
    /// of fact - a decision about who may open this, made by the owner - and
    /// because it has to outlive a restart. A paused app stays in the library,
    /// keeps its links and its devices, and simply does not answer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub paused: bool,
    /// Whether this app is reachable from outside the local network, and how.
    ///
    /// Beside `visibility` for the same reasons `paused` is, and orthogonal to
    /// it for a reason worth stating: **visibility says who may open an app;
    /// the relay says whether it leaves the house.** An app can be Public on
    /// the local network and unreachable from the internet, which is what
    /// `Off` means and what every app means until somebody decides otherwise.
    #[serde(default, skip_serializing_if = "RelayMode::is_off")]
    pub relay: RelayMode,
    /// What this app is called in its public address, when the owner has
    /// chosen something other than the slug.
    ///
    /// A separate name from `slug` on purpose. The slug is how this machine
    /// routes - `chores-rota.local`, `/chores-rota/`, the store key - and it
    /// follows the folder. The public label is a promise made to whoever was
    /// sent the link, so renaming a folder must not break it and changing the
    /// public address must not move the app on the local network.
    ///
    /// `None` means "use the slug", which is every app until somebody edits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_label: Option<String>,
    /// Unknown keys are preserved verbatim so a manifest written by a newer
    /// daemon survives a round-trip through an older one.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn default_entry() -> String {
    "index.html".to_string()
}

impl AppManifest {
    /// What this app actually answers to in a public address.
    ///
    /// The slug until somebody edits it. One accessor rather than
    /// `public_label.unwrap_or(slug)` scattered about, because the two names
    /// are easy to confuse and getting it wrong means the daemon builds a URL
    /// it will not itself answer to.
    pub fn effective_public_label(&self) -> &str {
        self.public_label.as_deref().unwrap_or(&self.slug)
    }
}

/// Whether an app is reachable away from home, and on what terms.
///
/// The two modes are not a performance setting. They are the resolution of a
/// promise the product could not keep twice: serving a snapshot while the
/// owner's machine sleeps requires the edge to hold servable plaintext, and
/// end-to-end encryption requires that it cannot. Both cannot be true of the
/// same app, so the owner chooses per app, at the moment they switch the relay
/// on - never by a silent default for the sensitive case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum RelayMode {
    /// Not reachable from outside this network. The resting state of every app
    /// that has never been published, and the reason a folder appearing in the
    /// workspace puts nothing on the internet.
    #[default]
    Off,
    /// TLS terminates at the edge, which can therefore serve a snapshot while
    /// this machine is asleep - and can technically read the app. Honest
    /// marketing is "encrypted in transit and at rest", not "end to end".
    Standard,
    /// The edge routes ciphertext by hostname and terminates nothing, so it
    /// cannot read the app even in principle. The cost is that there is no
    /// snapshot: offline means offline.
    Strict,
}

impl RelayMode {
    pub const fn is_off(&self) -> bool {
        matches!(self, Self::Off)
    }

    /// Whether a request that arrived over the relay may be served at all.
    ///
    /// Deliberately not the same question as which mode it is: everything
    /// downstream of here cares only that the owner published this app.
    pub const fn is_published(&self) -> bool {
        !self.is_off()
    }

    /// The label shown to people.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Standard => "Standard",
            Self::Strict => "Strict",
        }
    }

    /// What switching the relay on should default to for a given visibility.
    ///
    /// Invited apps are somebody's private thing shared with named people, so
    /// they default to the mode where the operator cannot read them. Public
    /// apps are already public, so they default to the mode that keeps the
    /// link alive when the laptop shuts. The owner can always override; this
    /// only decides which radio button starts selected.
    pub const fn suggested_for(visibility: Visibility) -> Self {
        match visibility {
            Visibility::Public => Self::Standard,
            _ => Self::Strict,
        }
    }
}

/// The longest a single DNS label may be. A public hostname is
/// `<label>-<handle>.<domain>`, and `<label>-<handle>` is one label.
pub const MAX_DNS_LABEL: usize = 63;

/// Why a public address the owner typed cannot be used.
///
/// Separate variants rather than one string because the window has to say
/// which rule was broken next to the field, and "invalid" next to a text box is
/// the least useful thing a form can say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicLabelError {
    Empty,
    /// Contains something that is not a letter, digit or hyphen.
    BadCharacter(char),
    /// A DNS label may not begin or end with a hyphen.
    EdgeHyphen,
    /// `<label>-<handle>` would exceed [`MAX_DNS_LABEL`].
    TooLong {
        max: usize,
    },
    /// Another app on this machine already answers to it. Two apps sharing a
    /// public name means one of them is unreachable, silently.
    Taken {
        slug: String,
    },
}

impl PublicLabelError {
    /// A sentence that can be shown under the field as-is.
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "An address needs at least one character.".into(),
            Self::BadCharacter(c) => {
                format!("‘{c}’ can't be in a web address. Use letters, numbers and hyphens.")
            }
            Self::EdgeHyphen => "An address can't start or end with a hyphen.".into(),
            Self::TooLong { max } => {
                format!("That's too long. Keep it to {max} characters or fewer.")
            }
            Self::Taken { slug } => {
                format!("‘{slug}’ already uses that address. Pick another.")
            }
        }
    }
}

impl std::fmt::Display for PublicLabelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for PublicLabelError {}

/// Check and normalise a public address the owner typed.
///
/// Hostnames are case-insensitive and whitespace around a pasted value is an
/// accident, so both are absorbed rather than refused. Everything else is a
/// real refusal, because the result becomes a DNS name.
///
/// Hyphens *inside* the label are fine, and deliberately so: the hostname
/// splits at its **last** hyphen and a handle may never contain one, so
/// `trip-briefing-html-template-adarsh` is unambiguous. Only the edges are
/// refused, which is a DNS rule rather than one of ours.
pub fn normalise_public_label(raw: &str, handle: &str) -> Result<String, PublicLabelError> {
    let label = raw.trim().to_ascii_lowercase();

    if label.is_empty() {
        return Err(PublicLabelError::Empty);
    }
    if let Some(c) = label
        .chars()
        .find(|c| !c.is_ascii_alphanumeric() && *c != '-')
    {
        return Err(PublicLabelError::BadCharacter(c));
    }
    if label.starts_with('-') || label.ends_with('-') {
        return Err(PublicLabelError::EdgeHyphen);
    }

    // The handle and its joining hyphen are spent before the owner types a
    // character, so the limit shown is the one that applies to *this* install.
    let max = MAX_DNS_LABEL.saturating_sub(handle.len() + 1);
    if label.len() > max {
        return Err(PublicLabelError::TooLong { max });
    }

    Ok(label)
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
    /// Whether the manifest's `entry` actually exists in the folder.
    ///
    /// False means every request to the app's root is a 404. The app is
    /// otherwise entirely healthy - registered, announced, gated - which is
    /// exactly why it has to be said out loud rather than left to be discovered
    /// by tapping the link on a phone.
    pub entry_exists: bool,
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
            // Assumed present. Claiming a missing entry without having looked
            // would put a "will not open" warning on a working app.
            entry_exists: true,
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
            paused: m.paused,
            relay: m.relay,
            url: match &urls.hostname {
                Some(host) => format!("{}://{host}{}", urls.scheme, urls.port_suffix),
                None => prefix_url.clone(),
            },
            hostname: urls.hostname.clone(),
            fallback_url: format!("{}/{}", urls.fallback_origin, m.slug),
            // Only for an app whose owner has published it. An unpublished app
            // has no public URL because it is not on the internet, and showing
            // one that 404s would be a worse lie than showing nothing.
            // Present only when a request could actually arrive and be served
            // this way: the owner published it, this install has a name to
            // publish under, and the visibility is one a relayed request can
            // satisfy. Setting an app to Private leaves the relay switch alone
            // - the two are orthogonal on purpose - but it does retire the
            // address, because from that moment the URL answers 403 to
            // everybody including the owner, and printing it would be handing
            // out a link that cannot work.
            public_url: match (
                m.relay.is_published() && m.visibility.reachable_over_relay(),
                &urls.public_host,
            ) {
                (true, Some(host)) => Some(format!("https://{host}")),
                _ => None,
            },
            // Always loopback, always the owner's own machine. `url` is the
            // announced `.local` name, which resolves to this machine's *LAN*
            // address - so it is not loopback, does not earn the own-machine
            // exemption in the gate, and answers 403 for a Private app.
            loopback_url: prefix_url.clone(),
            public_label: m.effective_public_label().to_string(),
            path: self.path.clone(),
            // Saturating rather than wrapping: ts-rs maps u64 to bigint, which
            // JSON cannot carry, so these cross the wire as u32. An app folder
            // over 4 GB reports 4 GB, which is the right kind of wrong for a
            // number whose only job is to render as "12.4 MB".
            size_bytes: self.size_bytes.min(u32::MAX as u64) as u32,
            deployed_at: self.deployed_at.map(|t| t.min(u32::MAX as u64) as u32),
            entry_exists: self.entry_exists,
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
    /// The public name this app would answer to away from home, e.g.
    /// `trip-adarsh.kitchentable.cloud`. `None` until an account claims a
    /// handle, which is every install today.
    ///
    /// Carries no opinion about whether the app is published. Whether it is
    /// *shown* is [`RelayMode`]'s question, resolved in [`AppRecord::to_app`].
    pub public_host: Option<String>,
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
    /// Taken offline by the owner. Still listed, still shared with the same
    /// people, but answering nobody until it is resumed.
    #[serde(default)]
    pub paused: bool,
    /// Whether this app is reachable away from home, and on what terms.
    ///
    /// Carried here and not only in the manifest because the window has to be
    /// able to say which of the two promises is in force. A relay switch that
    /// can be set but not read would show every published app as unpublished.
    #[serde(default)]
    pub relay: RelayMode,
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
    /// The loopback URL, for the owner's own browser on this machine.
    ///
    /// The one address that is always openable by the owner whatever the
    /// visibility, because loopback is what the gate exempts. Neither `url` nor
    /// `fallback_url` is loopback: the announced `.local` name and the IP one
    /// both resolve to this machine's *LAN* address, which is why opening
    /// either was answered 403 on a Private app and asked for pairing on an
    /// Invited one - on the owner's own machine.
    pub loopback_url: String,
    /// Where this app answers from away from home, once its owner has
    /// published it and an account has claimed a handle.
    ///
    /// Absent means there is no public URL to show - either the relay is off
    /// for this app, or this install has no handle yet. Both are true of every
    /// app until somebody decides otherwise, and neither is an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub public_url: Option<String>,
    /// The editable part of the public address: everything before `-<handle>`.
    ///
    /// Always present, and equal to the slug until somebody edits it, so the
    /// window can prefill the field whether or not this app has ever been
    /// published. What it is *joined to* is install-level and comes from
    /// `sys.status`, because one handle covers every app on this machine.
    pub public_label: String,
    /// Absolute path to the app folder in the workspace.
    pub path: String,
    /// Bundle size in bytes, for the Overview tab. Saturates at `u32::MAX`.
    pub size_bytes: u32,
    /// Unix seconds when the content last changed. Absent when the filesystem
    /// does not report it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub deployed_at: Option<u32>,
    /// Whether `entry` is actually in the folder. False means the app's root
    /// URL is a 404 even though everything else about it is healthy.
    pub entry_exists: bool,
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
            public_host: None,
        }
    }

    fn record() -> AppRecord {
        AppRecord {
            manifest: AppManifest {
                relay: RelayMode::Off,
                public_label: None,
                name: "Trip Planner".into(),
                slug: "trip-planner".into(),
                icon: None,
                entry: "index.html".into(),
                visibility: Visibility::Private,
                version: 1,
                paused: false,
                extra: serde_json::Map::new(),
            },
            path: "/ws/trip".into(),
            size_bytes: 42_000,
            deployed_at: Some(1_760_000_000),
            entry_exists: true,
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
    fn an_address_is_normalised_rather_than_refused_where_it_safely_can_be() {
        // Hostnames are case-insensitive and whitespace round a pasted value is
        // an accident. Refusing either would be pedantry at somebody typing.
        assert_eq!(
            normalise_public_label("  Trip-Briefing  ", "adarsh"),
            Ok("trip-briefing".to_string())
        );
    }

    #[test]
    fn hyphens_inside_an_address_are_fine_and_only_the_edges_are_not() {
        // The hostname splits at its LAST hyphen and a handle may never contain
        // one, so interior hyphens are unambiguous. Edge hyphens are a DNS rule.
        assert!(normalise_public_label("trip-briefing-html-template", "adarsh").is_ok());
        assert_eq!(
            normalise_public_label("-trip", "adarsh"),
            Err(PublicLabelError::EdgeHyphen)
        );
        assert_eq!(
            normalise_public_label("trip-", "adarsh"),
            Err(PublicLabelError::EdgeHyphen)
        );
    }

    #[test]
    fn an_address_that_would_not_survive_dns_is_refused_with_the_reason() {
        assert_eq!(
            normalise_public_label("", "adarsh"),
            Err(PublicLabelError::Empty)
        );
        assert_eq!(
            normalise_public_label("my trip", "adarsh"),
            Err(PublicLabelError::BadCharacter(' '))
        );
        assert_eq!(
            normalise_public_label("trip.briefing", "adarsh"),
            Err(PublicLabelError::BadCharacter('.')),
            "a dot would make it two labels and fall outside the wildcard cert"
        );
    }

    #[test]
    fn the_length_limit_counts_the_handle_the_owner_did_not_type() {
        // `<label>-<handle>` is one DNS label. The handle and its joining
        // hyphen are spent before a character is typed, so the number shown has
        // to be the one that actually applies to this install.
        let max = MAX_DNS_LABEL - "adarsh".len() - 1;
        assert!(normalise_public_label(&"a".repeat(max), "adarsh").is_ok());
        assert_eq!(
            normalise_public_label(&"a".repeat(max + 1), "adarsh"),
            Err(PublicLabelError::TooLong { max })
        );

        // And a longer handle leaves less room, rather than the limit being a
        // constant that happens to be right for one person's name.
        let roomier = MAX_DNS_LABEL - "jo".len() - 1;
        assert!(normalise_public_label(&"a".repeat(roomier), "jo").is_ok());
    }

    #[test]
    fn an_app_answers_to_its_slug_until_somebody_renames_it() {
        let mut m = record().manifest;
        assert_eq!(m.effective_public_label(), "trip-planner");

        m.public_label = Some("holiday".into());
        assert_eq!(m.effective_public_label(), "holiday");
    }

    #[test]
    fn only_a_published_app_has_a_public_url() {
        // The name exists for every app on a machine with a handle - that is
        // the whole point of one entry in the edge covering all of them. What
        // decides whether anyone is shown it is whether the owner published it.
        let mut u = urls(Some("trip-planner.local"));
        u.public_host = Some("trip-planner-adarsh.kitchentable.cloud".into());

        let mut r = record();
        // Invited, because Private can never be served over the relay and so
        // never has a public address whatever the switch says.
        r.manifest.visibility = Visibility::Invited;
        assert_eq!(r.to_app(&u).public_url, None, "off means no public URL");

        r.manifest.relay = RelayMode::Standard;
        assert_eq!(
            r.to_app(&u).public_url.as_deref(),
            Some("https://trip-planner-adarsh.kitchentable.cloud")
        );
    }

    #[test]
    fn setting_a_published_app_to_private_retires_its_public_address() {
        // Found by publishing an app as Invited and then setting it Private:
        // the window went on showing a `.cloud` URL that answered 403 to
        // everybody, the owner included. The relay switch is deliberately left
        // alone - the two are orthogonal - but the *address* has to go, because
        // from that moment nothing can arrive on it and be served.
        let mut u = urls(Some("trip-planner.local"));
        u.public_host = Some("trip-adarsh.kitchentable.cloud".into());

        let mut r = record();
        r.manifest.relay = RelayMode::Standard;

        for visibility in [Visibility::Invited, Visibility::Public] {
            r.manifest.visibility = visibility;
            assert!(
                r.to_app(&u).public_url.is_some(),
                "{visibility:?} is reachable over the relay"
            );
        }
        for visibility in [Visibility::Private, Visibility::Network] {
            r.manifest.visibility = visibility;
            assert_eq!(
                r.to_app(&u).public_url,
                None,
                "{visibility:?} can only ever refuse a relayed request, so the URL is a lie"
            );
            // And the switch itself is untouched, so the owner still sees it is
            // on and can turn it off.
            assert!(r.to_app(&u).relay.is_published());
        }
    }

    #[test]
    fn the_owner_always_has_one_address_that_opens() {
        // `.local` and the IP URL both resolve to this machine's LAN address,
        // so neither is loopback and neither earns the own-machine exemption.
        // Opening a Private app from the window was answered 403 for exactly
        // that reason.
        let app = record().to_app(&urls(Some("trip-planner.local")));
        assert_eq!(app.loopback_url, "http://localhost/trip-planner");
        assert!(
            !app.url.contains("localhost"),
            "the .local name is not loopback"
        );
        assert!(
            !app.fallback_url.contains("localhost"),
            "the address URL is not loopback either"
        );
    }

    #[test]
    fn an_install_with_no_handle_shows_no_public_url_however_published() {
        // Every install today. Publishing without a handle is a real state and
        // not an error, so it shows nothing rather than a half-formed name.
        let mut r = record();
        r.manifest.relay = RelayMode::Standard;
        r.manifest.visibility = Visibility::Invited;
        assert_eq!(r.to_app(&urls(None)).public_url, None);
    }

    #[test]
    fn the_public_url_is_https_because_the_edge_terminates_tls() {
        // The local scheme is http and says so. The relay is not the local
        // listener, so it must not inherit that.
        let mut u = urls(None);
        u.public_host = Some("trip-planner-adarsh.kitchentable.cloud".into());
        let mut r = record();
        r.manifest.relay = RelayMode::Standard;
        r.manifest.visibility = Visibility::Invited;

        let app = r.to_app(&u);
        assert!(app.url.starts_with("http://"));
        assert!(app.public_url.expect("published").starts_with("https://"));
    }

    #[test]
    fn the_relay_mode_reaches_the_client_view() {
        // The window has to render published and unpublished differently, so a
        // mode that can be set but not read would show every published app as
        // off - and offer to publish something already on the internet.
        let mut r = record();
        assert_eq!(r.to_app(&urls(None)).relay, RelayMode::Off);

        r.manifest.relay = RelayMode::Standard;
        assert_eq!(r.to_app(&urls(None)).relay, RelayMode::Standard);
    }

    #[test]
    fn switching_the_relay_on_suggests_the_safer_mode_for_everything_but_public() {
        // Public is already public, so it defaults to the mode that keeps the
        // link alive. Everything else defaults to the one the edge cannot read.
        assert_eq!(
            RelayMode::suggested_for(Visibility::Public),
            RelayMode::Standard
        );
        for v in [
            Visibility::Private,
            Visibility::Network,
            Visibility::Invited,
        ] {
            assert_eq!(RelayMode::suggested_for(v), RelayMode::Strict);
        }
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
