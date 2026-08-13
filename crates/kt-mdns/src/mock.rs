//! Announcers that do not touch the network.

use std::net::Ipv4Addr;
use std::sync::Mutex;

use crate::{Announcement, Announcer, Handle, MdnsError};

/// Records what was announced, and can pretend names are already taken.
///
/// Lets the daemon's wiring be tested without a network, which matters because
/// the real path needs a permission prompt and a second device.
#[derive(Default)]
pub struct MockAnnouncer {
    taken: Mutex<Vec<String>>,
    announced: Mutex<Vec<String>>,
    fail: Mutex<Option<String>>,
}

impl MockAnnouncer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pretend these names are already claimed on the network.
    pub fn with_taken<I, S>(self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        *self.taken.lock().expect("not poisoned") = names.into_iter().map(Into::into).collect();
        self
    }

    /// Make every announcement fail, for testing the degraded path.
    pub fn failing(self, reason: impl Into<String>) -> Self {
        *self.fail.lock().expect("not poisoned") = Some(reason.into());
        self
    }

    /// Names announced so far, in order.
    pub fn announced(&self) -> Vec<String> {
        self.announced.lock().expect("not poisoned").clone()
    }
}

impl Announcer for MockAnnouncer {
    fn announce(&self, name: &str, _addr: Ipv4Addr) -> Result<Announcement, MdnsError> {
        if let Some(reason) = self.fail.lock().expect("not poisoned").clone() {
            return Err(MdnsError::Responder(reason));
        }

        let mut taken = self.taken.lock().expect("not poisoned");
        let registered = if taken.iter().any(|t| t == name) {
            (2..)
                .map(|n| format!("{name}-{n}"))
                .find(|c| !taken.iter().any(|t| t == c))
                .unwrap_or_else(|| format!("{name}-x"))
        } else {
            name.to_string()
        };

        taken.push(registered.clone());
        self.announced
            .lock()
            .expect("not poisoned")
            .push(registered.clone());

        Ok(Announcement {
            requested: name.to_string(),
            registered,
            handle: Handle::None,
        })
    }

    fn is_live(&self) -> bool {
        false
    }
}

/// Accepts announcements and does nothing, on platforms with no implementation
/// yet. Apps stay reachable at the machine's own hostname and by IP.
pub struct UnsupportedAnnouncer;

impl Announcer for UnsupportedAnnouncer {
    fn announce(&self, _name: &str, _addr: Ipv4Addr) -> Result<Announcement, MdnsError> {
        Err(MdnsError::Unsupported)
    }

    fn is_live(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_what_it_announced() {
        let announcer = MockAnnouncer::new();
        announcer
            .announce("a", Ipv4Addr::LOCALHOST)
            .expect("announces");
        announcer
            .announce("b", Ipv4Addr::LOCALHOST)
            .expect("announces");
        assert_eq!(announcer.announced(), ["a", "b"]);
    }

    #[test]
    fn two_apps_wanting_one_name_both_get_served() {
        let announcer = MockAnnouncer::new();
        let first = announcer
            .announce("notes", Ipv4Addr::LOCALHOST)
            .expect("first");
        let second = announcer
            .announce("notes", Ipv4Addr::LOCALHOST)
            .expect("second");

        assert_eq!(first.registered, "notes");
        assert_eq!(second.registered, "notes-2");
    }

    #[test]
    fn a_failing_responder_is_an_error_not_a_silent_success() {
        let announcer = MockAnnouncer::new().failing("permission denied");
        assert!(matches!(
            announcer.announce("x", Ipv4Addr::LOCALHOST),
            Err(MdnsError::Responder(_))
        ));
    }

    #[test]
    fn unsupported_platforms_say_so_rather_than_pretending() {
        assert!(matches!(
            UnsupportedAnnouncer.announce("x", Ipv4Addr::LOCALHOST),
            Err(MdnsError::Unsupported)
        ));
        assert!(!UnsupportedAnnouncer.is_live());
    }
}

/// No network lookup at all: the platform has no implementation, or a test does
/// not want one. Callers fall back to whatever the user agent suggested.
pub struct NoDiscoverer;

impl crate::Discoverer for NoDiscoverer {
    fn name_for(&self, _addr: std::net::Ipv4Addr, _within: std::time::Duration) -> Option<String> {
        None
    }

    fn is_live(&self) -> bool {
        false
    }
}

/// A discoverer with a fixed answer sheet, for tests.
#[derive(Default)]
pub struct MockDiscoverer {
    names: std::collections::HashMap<std::net::Ipv4Addr, String>,
}

impl MockDiscoverer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, addr: std::net::Ipv4Addr, name: &str) -> Self {
        self.names.insert(addr, name.to_string());
        self
    }
}

impl crate::Discoverer for MockDiscoverer {
    fn name_for(&self, addr: std::net::Ipv4Addr, _within: std::time::Duration) -> Option<String> {
        self.names.get(&addr).cloned()
    }
}
