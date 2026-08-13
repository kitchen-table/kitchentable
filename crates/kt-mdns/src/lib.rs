//! Publishing `appname.local` on the local network.
//!
//! Deliberately built on the platform's own mDNS responder rather than a
//! pure-Rust implementation. On macOS that means the Local Network permission
//! prompt is attributed to Kitchen Table and the system handles name conflicts
//! for us; a Rust mDNS stack would fight mDNSResponder for port 5353 and get
//! the attribution wrong (docs/architecture.md section 10).

use std::net::Ipv4Addr;
use std::time::Duration;

#[cfg(target_os = "macos")]
mod bonjour;
#[cfg(target_os = "macos")]
mod discover;
mod mock;

pub use mock::{MockAnnouncer, MockDiscoverer, NoDiscoverer, UnsupportedAnnouncer};

#[cfg(target_os = "macos")]
pub use bonjour::BonjourAnnouncer;
#[cfg(target_os = "macos")]
pub use discover::BonjourDiscoverer;

#[derive(Debug, thiserror::Error)]
pub enum MdnsError {
    #[error("could not reach the system mDNS responder: {0}")]
    Responder(String),
    #[error("{name}.local is already claimed on this network")]
    NameConflict { name: String },
    #[error("this platform has no mDNS support wired up yet")]
    Unsupported,
}

/// A published name. Dropping it withdraws the announcement.
#[derive(Debug)]
pub struct Announcement {
    /// What we asked for.
    pub requested: String,
    /// What we got. Differs when the network already had that name, which the
    /// UI surfaces so nobody wonders why the URL they were told does not match
    /// the one on screen.
    pub registered: String,
    /// Held, not read: dropping it is what withdraws the announcement.
    #[allow(dead_code)]
    handle: Handle,
}

impl Announcement {
    /// The hostname to put in a URL, e.g. `trip-planner.local`.
    pub fn hostname(&self) -> String {
        format!("{}.local", self.registered)
    }

    /// Whether the network made us take a different name.
    pub fn was_renamed(&self) -> bool {
        self.requested != self.registered
    }
}

#[derive(Debug)]
enum Handle {
    /// Nothing to release: mock, or a platform without an implementation.
    None,
    #[cfg(target_os = "macos")]
    Bonjour(#[allow(dead_code)] bonjour::Registration),
}

/// Publishes hostnames on the local network.
///
/// One implementation per platform, plus [`MockAnnouncer`] for tests. The
/// daemon holds a `dyn Announcer` so nothing above this crate has a
/// `cfg(target_os)` in it.
pub trait Announcer: Send + Sync {
    /// Publish `<name>.local` pointing at this machine.
    ///
    /// Implementations may return a different name than requested when the
    /// network already has one; callers must use the returned name.
    fn announce(&self, name: &str, addr: Ipv4Addr) -> Result<Announcement, MdnsError>;

    /// Whether announcements from this implementation are really visible on the
    /// network. False for the mock and on unsupported platforms, which lets the
    /// daemon report a degraded state rather than promising URLs that will not
    /// resolve.
    fn is_live(&self) -> bool {
        true
    }
}

/// Asks the network what the device at an address calls itself.
///
/// Separate from [`Announcer`] because they are opposite directions and have
/// different costs: announcing is cheap and permanent, asking is slow and
/// happens once, when someone is waiting to be let in.
///
/// The answer is a *suggestion for a name*, never an identity. Matching is by
/// IP, which the device asserts; nothing is authorised on the strength of it,
/// and the owner reads and edits the name before approving anything.
pub trait Discoverer: Send + Sync {
    /// The name the device at `addr` publishes for itself, if any.
    ///
    /// Must return within roughly `within`. Callers run this off any path a
    /// person is waiting on - a phone sitting on the pairing page must not be
    /// held up by a lookup about itself.
    fn name_for(&self, addr: Ipv4Addr, within: Duration) -> Option<String>;

    /// Whether this implementation can really see the network. False for the
    /// no-op one, so callers can skip the wait entirely.
    fn is_live(&self) -> bool {
        true
    }
}

/// How long a name lookup may take before the guess is used instead.
///
/// Generous because it is off the request path and only runs when someone is
/// about to be asked a question; short enough that the pairing prompt is not
/// visibly waiting on it.
pub const NAME_LOOKUP_BUDGET: Duration = Duration::from_secs(4);

/// The discoverer for this platform, or a no-op one where there is none.
pub fn discoverer_for_this_platform() -> Box<dyn Discoverer> {
    #[cfg(target_os = "macos")]
    {
        Box::new(BonjourDiscoverer)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(NoDiscoverer)
    }
}

/// The address other devices on the network should use to reach this machine.
///
/// Found by asking the routing table which source address it would use to
/// reach the internet. No packet is sent: a UDP socket has no handshake, so
/// this works offline and costs nothing.
pub fn primary_ipv4() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:80").ok()?; // TEST-NET-1, guaranteed unrouted
    match socket.local_addr().ok()? {
        std::net::SocketAddr::V4(addr) => Some(*addr.ip()),
        std::net::SocketAddr::V6(_) => None,
    }
}

/// The announcer for this platform, or a no-op one where there is none.
pub fn for_this_platform() -> Box<dyn Announcer> {
    #[cfg(target_os = "macos")]
    {
        Box::new(BonjourAnnouncer::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Windows and Linux land in D-later; until then apps are still
        // reachable at the machine's own hostname and by IP.
        Box::new(mock::UnsupportedAnnouncer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hostname_is_the_registered_name_not_the_requested_one() {
        let announcer = MockAnnouncer::new().with_taken(["notes"]);
        let a = announcer
            .announce("notes", Ipv4Addr::LOCALHOST)
            .expect("announces");

        assert_eq!(a.requested, "notes");
        assert_eq!(a.registered, "notes-2");
        assert_eq!(a.hostname(), "notes-2.local");
        assert!(a.was_renamed());
    }

    #[test]
    fn an_uncontested_name_is_not_renamed() {
        let announcer = MockAnnouncer::new();
        let a = announcer
            .announce("trip-planner", Ipv4Addr::LOCALHOST)
            .expect("announces");

        assert_eq!(a.hostname(), "trip-planner.local");
        assert!(!a.was_renamed());
    }

    #[test]
    fn the_primary_address_is_not_loopback() {
        // Skipped rather than failed when there is no network at all, so this
        // does not turn into a flaky CI failure on an isolated runner.
        if let Some(addr) = primary_ipv4() {
            assert!(!addr.is_loopback(), "got {addr}, which no phone can reach");
        }
    }
}
