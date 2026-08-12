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
    /// Seconds since the daemon started.
    pub uptime_secs: u64,
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
