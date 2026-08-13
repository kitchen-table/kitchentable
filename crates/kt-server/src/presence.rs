//! Who currently has an app open.
//!
//! Serving static files leaves nothing to observe: the browser fetches the
//! page and hangs up, so "still reading it" and "closed it an hour ago" look
//! identical from here. The only way to tell them apart is for the page to keep
//! saying so, which is why [`crate::live`] puts a small script into served HTML
//! and why this module exists to hear from it.
//!
//! Everything here is in memory and per-run. Presence is a statement about
//! right now; a restart means nobody is known to be reading anything, which is
//! true, because nobody's page has checked in yet.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use kt_types::Viewer;

/// How often a page checks in.
///
/// Ten seconds is frequent enough that leaving a tab shows up while someone is
/// still looking at the window, and rare enough to be invisible: one request
/// per page per ten seconds, on a LAN, against a daemon already serving files.
pub const BEAT_EVERY: Duration = Duration::from_secs(10);

/// How long after its last beat a page is still counted.
///
/// Two and a half intervals, so one dropped request - a phone that slept for a
/// moment, a wifi hiccup - does not make somebody flicker out of the list and
/// back in. The cost of being generous is naming someone who has just closed a
/// tab for a few seconds longer; the cost of being strict is a list that
/// twitches.
pub const LINGER: Duration = Duration::from_secs(25);

struct Seen {
    at: Instant,
    path: String,
}

/// Live viewers, keyed by app and device.
#[derive(Default)]
pub struct Presence {
    seen: Mutex<HashMap<(String, String), Seen>>,
}

impl Presence {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a check-in. Returns true when this changes what anyone would see,
    /// so the caller knows whether it is worth telling the window about.
    ///
    /// Moving to another page counts as a change: the list says what each
    /// person is looking at, so that is part of what is on screen.
    pub fn beat(&self, slug: &str, device: &str, path: &str) -> bool {
        let mut seen = self.seen.lock().expect("not poisoned");
        let key = (slug.to_string(), device.to_string());
        let changed = match seen.get(&key) {
            Some(previous) => previous.path != path,
            None => true,
        };
        seen.insert(
            key,
            Seen {
                at: Instant::now(),
                path: path.to_string(),
            },
        );
        changed
    }

    /// A page said it was closing. Returns true if we were still counting it.
    ///
    /// Waiting for the entry to expire would be correct but slow: somebody
    /// closing a tab should leave the owner's list promptly, not in half a
    /// minute. Best-effort, because a tab that crashes says nothing - which is
    /// what [`LINGER`] is for.
    pub fn left(&self, slug: &str, device: &str) -> bool {
        let mut seen = self.seen.lock().expect("not poisoned");
        seen.remove(&(slug.to_string(), device.to_string()))
            .is_some()
    }

    /// Forget everyone who has stopped checking in. Returns true if anyone did.
    pub fn sweep(&self) -> bool {
        let mut seen = self.seen.lock().expect("not poisoned");
        let before = seen.len();
        seen.retain(|_, s| s.at.elapsed() < LINGER);
        before != seen.len()
    }

    /// Who is reading `slug` right now, longest-idle last.
    ///
    /// Sweeps as it reads rather than trusting a background task to have run:
    /// a stale name in this list is a claim about the present, and a wrong one
    /// is worse than an empty list.
    pub fn viewers(&self, slug: &str) -> Vec<Viewer> {
        let seen = self.seen.lock().expect("not poisoned");
        let mut viewers: Vec<Viewer> = seen
            .iter()
            .filter(|((app, _), s)| app == slug && s.at.elapsed() < LINGER)
            .map(|((_, device), s)| Viewer {
                device_id: device.clone(),
                path: s.path.clone(),
                seconds_ago: s.at.elapsed().as_secs().min(u32::MAX as u64) as u32,
            })
            .collect();
        viewers.sort_by_key(|v| v.seconds_ago);
        viewers
    }

    /// Every app someone is currently reading, for the library's counts.
    pub fn counts(&self) -> HashMap<String, u32> {
        let seen = self.seen.lock().expect("not poisoned");
        let mut counts: HashMap<String, u32> = HashMap::new();
        for ((app, _), s) in seen.iter() {
            if s.at.elapsed() < LINGER {
                *counts.entry(app.clone()).or_default() += 1;
            }
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_that_checks_in_is_being_read() {
        let presence = Presence::new();
        assert!(presence.beat("trip", "d1", "/"), "the first beat is news");

        let viewers = presence.viewers("trip");
        assert_eq!(viewers.len(), 1);
        assert_eq!(viewers[0].device_id, "d1");
        assert_eq!(viewers[0].path, "/");
    }

    #[test]
    fn checking_in_from_the_same_page_is_not_news() {
        // Otherwise every viewer would push an event every ten seconds and the
        // window would redraw the list constantly for no new information.
        let presence = Presence::new();
        presence.beat("trip", "d1", "/");
        assert!(!presence.beat("trip", "d1", "/"));
    }

    #[test]
    fn moving_to_another_page_is_news() {
        let presence = Presence::new();
        presence.beat("trip", "d1", "/");
        assert!(presence.beat("trip", "d1", "/budget"));
        assert_eq!(presence.viewers("trip")[0].path, "/budget");
    }

    #[test]
    fn apps_do_not_borrow_each_others_viewers() {
        let presence = Presence::new();
        presence.beat("trip", "d1", "/");
        presence.beat("chores", "d2", "/");

        assert_eq!(presence.viewers("trip").len(), 1);
        assert_eq!(presence.viewers("chores").len(), 1);
        assert_eq!(presence.counts().get("trip"), Some(&1));
    }

    #[test]
    fn two_anonymous_readers_are_two_viewers() {
        // A Household app pairs nobody, so most of its readers have no device.
        // Counting them under one id would show a houseful of phones as one
        // person - which is the failure this exists to avoid.
        let presence = Presence::new();
        presence.beat("trip", "at:192.168.0.51", "/");
        presence.beat("trip", "at:192.168.0.77", "/");
        assert_eq!(presence.viewers("trip").len(), 2);
        assert_eq!(presence.counts().get("trip"), Some(&2));
    }

    #[test]
    fn one_person_on_two_devices_is_two_viewers() {
        let presence = Presence::new();
        presence.beat("trip", "phone", "/");
        presence.beat("trip", "laptop", "/");
        assert_eq!(presence.viewers("trip").len(), 2);
    }

    #[test]
    fn somebody_who_stopped_checking_in_is_dropped() {
        // The whole point: a closed tab stops beating, and the list has to
        // notice rather than keeping the name up forever.
        let presence = Presence::new();
        presence.beat("trip", "d1", "/");

        // Reach in and age the entry rather than sleeping for LINGER, which
        // would make this test take half a minute.
        {
            let mut seen = presence.seen.lock().expect("not poisoned");
            let entry = seen.get_mut(&("trip".into(), "d1".into())).expect("there");
            entry.at = Instant::now() - LINGER - Duration::from_secs(1);
        }

        assert!(
            presence.viewers("trip").is_empty(),
            "stale viewers are gone"
        );
        assert!(presence.counts().is_empty());
        assert!(presence.sweep(), "the sweep reports it dropped somebody");
        assert!(
            !presence.sweep(),
            "and has nothing to report the second time"
        );
    }
}
