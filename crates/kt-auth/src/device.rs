//! Devices: browsers that have been approved once and remembered.
//!
//! Pairing is modelled on Bluetooth deliberately (product.md section 5.5). A
//! new device asks, the owner sees who is asking, and approving it gives the
//! device a name a person would recognise rather than an opaque identifier.

use serde::{Deserialize, Serialize};

/// Stable identifier for one browser on one device.
///
/// Random rather than derived from anything about the device: fingerprinting a
/// browser would be both unreliable and the wrong instinct for a product whose
/// pitch is that your things stay yours.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(String);

impl DeviceId {
    pub fn generate() -> Self {
        Self(crate::session::random_token(16))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parse an id that came in on a cookie. Rejects anything that is not the
    /// shape we mint, so a hand-crafted cookie cannot smuggle in a path or SQL.
    pub fn parse(raw: &str) -> Option<Self> {
        let ok = raw.len() >= 16
            && raw.len() <= 64
            && raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        ok.then(|| Self(raw.to_string()))
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceStatus {
    /// Has asked, and is waiting on the owner.
    Pending,
    Approved,
    /// Explicitly refused or later revoked. Never silently upgraded back.
    Revoked,
    /// The owner's own machine.
    Owner,
}

/// Where a device's name came from.
///
/// The order matters: a stronger source may overwrite a weaker one, never the
/// other way round. Nothing overwrites what the owner typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamedBy {
    /// Guessed from the user agent: a model class at best, and wrong on an
    /// iPad in desktop mode.
    #[default]
    Guess,
    /// The name the device publishes for itself on the local network.
    Network,
    /// Typed by the owner. Final.
    Owner,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: DeviceId,
    /// Something a person recognises: "Kitchen iPad". Suggested from the
    /// user agent, improved by the network where it answers, and editable.
    pub name: String,
    /// Where `name` came from, so a later lookup never overwrites the owner.
    #[serde(default)]
    pub named_by: NamedBy,
    pub status: DeviceStatus,
    /// What the browser said it was, kept for the approval prompt.
    pub user_agent: String,
    /// A short, stable string shown next to the device so two similar ones can
    /// be told apart at a glance.
    pub fingerprint: String,
    pub first_seen: i64,
    pub last_seen: i64,
}

impl Device {
    pub fn pending(id: DeviceId, user_agent: &str, now: i64) -> Self {
        Self {
            fingerprint: fingerprint(&id),
            name: suggest_name(user_agent),
            named_by: NamedBy::Guess,
            id,
            status: DeviceStatus::Pending,
            user_agent: user_agent.to_string(),
            first_seen: now,
            last_seen: now,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(id: &str, status: DeviceStatus) -> Self {
        Self {
            id: DeviceId(id.to_string()),
            name: "Test device".into(),
            status,
            user_agent: "test".into(),
            fingerprint: "aa:bb:cc".into(),
            first_seen: 0,
            last_seen: 0,
            named_by: NamedBy::Guess,
        }
    }
}

/// A short visual fingerprint, in the style of an SSH key fingerprint.
///
/// Not a security control - it is derived from a public id - but it lets the
/// approval prompt and the device list refer to the same thing unambiguously.
fn fingerprint(id: &DeviceId) -> String {
    let bytes = id.0.as_bytes();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    let b = hash.to_be_bytes();
    format!("{:02x}:{:02x}:{:02x}", b[0], b[1], b[2])
}

/// Guess a name from the user agent, for the owner to accept or edit.
///
/// Deliberately coarse. Getting "iPhone" right matters; getting the iOS version
/// right does not, and a wrong-but-confident name is worse than a vague one.
pub fn suggest_name(user_agent: &str) -> String {
    let ua = user_agent.to_ascii_lowercase();

    let device = if ua.contains("ipad") {
        "iPad"
    } else if ua.contains("iphone") {
        "iPhone"
    } else if ua.contains("android") {
        "Android phone"
    } else if ua.contains("macintosh") || ua.contains("mac os") {
        "Mac"
    } else if ua.contains("windows") {
        "Windows PC"
    } else if ua.contains("linux") {
        "Linux machine"
    } else {
        return "New device".to_string();
    };

    device.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_distinct_and_parseable() {
        let a = DeviceId::generate();
        let b = DeviceId::generate();
        assert_ne!(a, b);
        assert_eq!(DeviceId::parse(a.as_str()), Some(a));
    }

    #[test]
    fn a_hand_crafted_cookie_cannot_smuggle_anything_in() {
        for bad in [
            "",
            "short",
            "../../etc/passwd",
            "abcdefghijklmnop'; DROP TABLE devices;--",
            "abcdefghijklmnop/../..",
            &"x".repeat(65),
        ] {
            assert_eq!(DeviceId::parse(bad), None, "{bad:?} should be rejected");
        }
    }

    #[test]
    fn fingerprints_are_stable_and_differ_between_devices() {
        let a = DeviceId::generate();
        let b = DeviceId::generate();
        assert_eq!(fingerprint(&a), fingerprint(&a));
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn device_names_are_recognisable() {
        let cases = [
            ("Mozilla/5.0 (iPhone; CPU iPhone OS 19_0) Safari", "iPhone"),
            ("Mozilla/5.0 (iPad; CPU OS 19_0) Safari", "iPad"),
            (
                "Mozilla/5.0 (Linux; Android 15; Pixel 9) Chrome",
                "Android phone",
            ),
            ("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) Safari", "Mac"),
            ("Mozilla/5.0 (Windows NT 10.0; Win64) Chrome", "Windows PC"),
            ("curl/8.0", "New device"),
            ("", "New device"),
        ];

        for (ua, expected) in cases {
            assert_eq!(suggest_name(ua), expected, "for {ua:?}");
        }
    }

    #[test]
    fn an_ipad_is_not_mistaken_for_a_mac() {
        // iPadOS reports "Macintosh" in desktop mode, so order matters here.
        let ipad_desktop_mode = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit iPad";
        assert_eq!(suggest_name(ipad_desktop_mode), "iPad");
    }

    #[test]
    fn a_new_device_starts_pending() {
        let d = Device::pending(DeviceId::generate(), "iPhone", 100);
        assert_eq!(d.status, DeviceStatus::Pending);
        assert_eq!(d.first_seen, 100);
    }
}
