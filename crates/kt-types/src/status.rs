//! Daemon status: what `sys.status` returns.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Whether the daemon is currently serving, and if not, why.
///
/// `Degraded` is a first-class state rather than an error because most of its
/// causes are recoverable and the UI is expected to explain each one: a denied
/// Local Network permission, port 80 already taken by another dev server, an
/// untrusted local CA (docs/onboarding.md section 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
#[ts(export)]
pub enum ServingState {
    Serving,
    /// Paused by the owner from the tray or `sys.pause_serving`.
    Paused,
    Degraded {
        reason: DegradedReason,
        /// Human-readable detail for the banner. Never a raw error string.
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DegradedReason {
    /// macOS Local Network permission denied: apps work on this machine only.
    LocalNetworkDenied,
    /// Port 80 or 443 was already bound, so we fell back to a high port.
    PortFallback,
    /// The local CA is not trusted, so apps are served over plain HTTP.
    HttpsUnavailable,
    /// mDNS registration failed; only the fallback URL works.
    MdnsUnavailable,
}

/// Whether this machine is reachable from the internet right now.
///
/// Distinct from whether any app is *published*, which is per-app and lives in
/// the manifest. An app can be published and unreachable, which is what happens
/// every time the tunnel drops - and the window has to be able to tell the
/// difference, because for a long time it could not and showed a confident
/// green badge over an app nobody outside could open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
#[ts(export)]
pub enum RelayState {
    /// No relay configured. Every install today, and not a fault.
    Off,
    /// Dialling, or waiting to dial again after a drop.
    Connecting,
    /// Up. Published apps are reachable at their public addresses.
    Connected,
    /// Stopped trying, because waiting cannot fix it. Somebody has to act, so
    /// the message is written to be read by them.
    NeedsAttention { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SysStatus {
    /// See [`crate::PROTOCOL_VERSION`]. The shell compares this on handshake.
    pub protocol_version: u32,
    pub daemon_version: String,
    /// Absolute path of the watched workspace folder.
    pub workspace: String,
    pub serving: ServingState,
    pub app_count: u32,
    /// This install's public identity to the relay, unpadded base64url.
    ///
    /// `None` means the daemon has no identity this run, which is a real state
    /// and not an error: the keystore would not give one back and inventing a
    /// replacement would have silently unlinked whatever account this install
    /// belongs to. Local serving is unaffected; only the relay needs this.
    pub install_key: Option<String>,
    /// This machine's handle and the domain its apps hang off, e.g.
    /// `("adarsh", "kitchentable.cloud")` rendered as a suffix.
    ///
    /// Install-level rather than per-app because **one handle covers every app
    /// on this machine** - that is what makes one entry in the edge enough. The
    /// window needs it to show what an address will look like before the app
    /// has one, so it can render `<label>-adarsh.kitchentable.cloud` while the
    /// owner is still typing the label.
    ///
    /// `None` until an account claims a handle, which is every install today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub relay_handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub relay_domain: Option<String>,
    /// Whether the tunnel is up right now, which decides whether a published
    /// app is actually reachable.
    pub relay: RelayState,
    /// Seconds since the daemon started. `u32` rather than `u64` because
    /// ts-rs maps 64-bit integers to `bigint`, which JSON cannot carry - and
    /// 136 years of uptime is enough.
    pub uptime_secs: u32,
    /// Whether `KT_WORKSPACE` fixed the workspace for this run.
    ///
    /// True means Settings must draw the folder as unchangeable and say why:
    /// the environment outranks the stored setting deliberately, so a picker
    /// offered here would write a value the daemon will never read and appear
    /// to do nothing after a restart.
    #[serde(default)]
    pub workspace_locked: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serving_state_is_tagged_so_typescript_can_narrow_it() {
        let json = serde_json::to_value(ServingState::Serving).expect("serialises");
        assert_eq!(json["state"], "serving");

        let degraded = ServingState::Degraded {
            reason: DegradedReason::LocalNetworkDenied,
            message: "Devices on your network can't see your apps yet.".into(),
        };
        let json = serde_json::to_value(&degraded).expect("serialises");
        assert_eq!(json["state"], "degraded");
        assert_eq!(json["reason"], "local_network_denied");
    }
}
