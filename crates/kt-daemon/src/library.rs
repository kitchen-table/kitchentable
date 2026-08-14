//! The set of apps currently being served.
//!
//! Held behind an `RwLock` and replaced wholesale on rescan, so a request
//! either sees the whole old library or the whole new one, never a half-applied
//! update.

use std::collections::HashMap;
use std::sync::RwLock;

use kt_mdns::{Announcement, Announcer};
use kt_server::{AppSource, ServedApp};
use kt_types::AppRecord;

#[derive(Default)]
pub struct Library {
    apps: RwLock<HashMap<String, Entry>>,
    /// This install's handle, when it has one, and the domain its apps hang
    /// off: `("adarsh", "kitchentable.cloud")`.
    ///
    /// `None` until an account claims a handle - which is every install today.
    /// A daemon with no handle answers no relay hostname at all, which is the
    /// correct behaviour rather than a limitation: nothing has been published.
    relay_identity: RwLock<Option<(String, String)>>,
}

struct Entry {
    record: AppRecord,
    served: ServedApp,
    /// The `<name>.local` this app answers to, when mDNS is live. Held so the
    /// announcement is withdrawn if the app leaves the workspace.
    announcement: Option<Announcement>,
}

impl Entry {
    fn hostname(&self) -> Option<String> {
        self.announcement.as_ref().map(|a| a.hostname())
    }
}

impl Library {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tell the library which public names belong to this machine.
    ///
    /// Environment-configured for now, in the same spirit as `KT_RELAY_URL`:
    /// a placeholder for the handle claim that arrives with account linking.
    pub fn set_relay_identity(&self, handle: Option<(String, String)>) {
        *self
            .relay_identity
            .write()
            .unwrap_or_else(|e| e.into_inner()) =
            handle.map(|(h, d)| (h.trim().to_ascii_lowercase(), d.trim().to_ascii_lowercase()));
    }

    /// The slug inside a public relay hostname, if it is one of ours.
    ///
    /// `notes-adarsh.kitchentable.cloud` is `notes`, given the handle `adarsh`.
    /// Everything about this is a refusal to guess:
    ///
    /// - the domain has to match, or it is somebody else's name;
    /// - the handle has to be **this machine's**, so a request misrouted to us
    ///   is refused rather than served from whatever app happens to share a
    ///   slug with somebody else's;
    /// - the split is at the last hyphen, which is unambiguous only because a
    ///   handle may not contain one.
    ///
    /// `kt-tunnel-proto` says a daemon receiving a hostname it does not serve
    /// should refuse rather than guess. This is that.
    fn slug_in_relay_host(&self, hostname: &str) -> Option<String> {
        let identity = self
            .relay_identity
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let (handle, domain) = identity.as_ref()?;

        let host = hostname.trim().trim_end_matches('.').to_ascii_lowercase();
        let label = host.strip_suffix(&format!(".{domain}"))?;
        if label.contains('.') {
            return None;
        }

        let (slug, claimed) = label.rsplit_once('-')?;
        (claimed == handle && !slug.is_empty()).then(|| slug.to_string())
    }

    /// Replace the whole library.
    ///
    /// A folder whose path will not canonicalise is dropped with a warning
    /// rather than failing the swap: one bad app must not take the library
    /// down.
    /// Replace the library without touching the network. Used by tests and by
    /// any caller that has no announcer.
    #[cfg(test)]
    pub fn replace(&self, records: Vec<AppRecord>) {
        self.replace_announcing(records, None, None);
    }

    /// Replace the library, announcing each app on the network.
    ///
    /// Apps that were already announced keep their announcement rather than
    /// being withdrawn and re-published, so a rescan does not make every phone
    /// on the network re-resolve.
    pub fn replace_announcing(
        &self,
        records: Vec<AppRecord>,
        announcer: Option<&dyn Announcer>,
        addr: Option<std::net::Ipv4Addr>,
    ) {
        let mut previous = std::mem::take(&mut *self.write());
        let mut next = HashMap::with_capacity(records.len());

        for record in records {
            let root = match std::path::Path::new(&record.path).canonicalize() {
                Ok(root) => root,
                Err(e) => {
                    tracing::warn!(path = %record.path, error = %e, "cannot serve app");
                    continue;
                }
            };

            let served = ServedApp {
                paused: record.manifest.paused,
                slug: record.manifest.slug.clone(),
                name: record.manifest.name.clone(),
                root,
                entry: record.manifest.entry.clone(),
                visibility: record.manifest.visibility,
                relay: record.manifest.relay,
            };

            let slug = record.manifest.slug.clone();
            let announcement = match previous.remove(&slug) {
                // Already on the air; leave it alone.
                Some(existing) if existing.announcement.is_some() => existing.announcement,
                _ => match (announcer, addr) {
                    (Some(announcer), Some(addr)) => match announcer.announce(&slug, addr) {
                        Ok(a) => {
                            if a.was_renamed() {
                                tracing::warn!(
                                    requested = %a.requested,
                                    registered = %a.registered,
                                    "name taken on this network, using a suffix"
                                );
                            }
                            Some(a)
                        }
                        // Announcing is best-effort: the app is still reachable
                        // by prefix and by IP, so a failure degrades rather
                        // than removing the app.
                        Err(e) => {
                            tracing::warn!(slug = %slug, error = %e, "could not announce");
                            None
                        }
                    },
                    _ => None,
                },
            };

            next.insert(
                slug,
                Entry {
                    record,
                    served,
                    announcement,
                },
            );
        }

        // Anything left in `previous` has gone; dropping it withdraws its name.
        *self.write() = next;
    }

    /// The announced hostname for an app, if it has one.
    pub fn hostname(&self, slug: &str) -> Option<String> {
        self.read().get(slug).and_then(Entry::hostname)
    }

    pub fn records(&self) -> Vec<AppRecord> {
        let apps = self.read();
        let mut out: Vec<_> = apps.values().map(|e| e.record.clone()).collect();
        out.sort_by_key(|r| r.manifest.name.to_lowercase());
        out
    }

    pub fn record(&self, slug: &str) -> Option<AppRecord> {
        self.read().get(slug).map(|e| e.record.clone())
    }

    pub fn len(&self) -> usize {
        self.read().len()
    }

    /// Paired with `len` so the type reads normally; the UI asks for this
    /// to show the empty-workspace state in D3.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A poisoned lock means a reader panicked; the data is still consistent
    /// because every write is a whole-map swap.
    fn read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, Entry>> {
        self.apps.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, Entry>> {
        self.apps.write().unwrap_or_else(|e| e.into_inner())
    }
}

impl AppSource for Library {
    fn get(&self, slug: &str) -> Option<ServedApp> {
        self.read().get(slug).map(|e| e.served.clone())
    }

    fn get_by_hostname(&self, hostname: &str) -> Option<ServedApp> {
        // An announced `<name>.local` on the local network.
        if let Some(app) = self
            .read()
            .values()
            .find(|e| e.hostname().as_deref() == Some(hostname))
            .map(|e| e.served.clone())
        {
            return Some(app);
        }

        // Or a public relay name, if this machine has one. The gate still
        // decides everything else, including whether this app was ever
        // published - see `RelayMode` and `decide`.
        let slug = self.slug_in_relay_host(hostname)?;
        self.get(&slug)
    }

    fn list(&self) -> Vec<ServedApp> {
        let apps = self.read();
        let mut out: Vec<_> = apps.values().map(|e| e.served.clone()).collect();
        out.sort_by_key(|a| a.name.to_lowercase());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kt_types::{AppManifest, Visibility};

    fn record(slug: &str, name: &str, path: &str) -> AppRecord {
        AppRecord::unmeasured(
            AppManifest {
                relay: Default::default(),
                name: name.into(),
                slug: slug.into(),
                icon: None,
                entry: "index.html".into(),
                visibility: Visibility::Private,
                version: 1,
                paused: false,
                extra: serde_json::Map::new(),
            },
            path.into(),
        )
    }

    #[test]
    fn an_unservable_path_is_skipped_without_losing_the_rest() {
        let dir = std::env::temp_dir();
        let library = Library::new();
        library.replace(vec![
            record("real", "Real", &dir.display().to_string()),
            record("ghost", "Ghost", "/nonexistent/path/for/sure"),
        ]);

        assert_eq!(library.len(), 1);
        assert!(library.get("real").is_some());
        assert!(library.get("ghost").is_none());
    }

    #[test]
    fn replacing_removes_what_is_gone() {
        let dir = std::env::temp_dir().display().to_string();
        let library = Library::new();
        library.replace(vec![record("a", "A", &dir), record("b", "B", &dir)]);
        assert_eq!(library.len(), 2);

        library.replace(vec![record("a", "A", &dir)]);
        assert_eq!(library.len(), 1);
        assert!(library.get("b").is_none());
    }

    #[test]
    fn an_announced_app_is_reachable_by_its_hostname() {
        let dir = std::env::temp_dir().display().to_string();
        let announcer = kt_mdns::MockAnnouncer::new();
        let library = Library::new();

        library.replace_announcing(
            vec![record("trip", "Trip", &dir)],
            Some(&announcer),
            Some(std::net::Ipv4Addr::LOCALHOST),
        );

        assert_eq!(library.hostname("trip").as_deref(), Some("trip.local"));
        assert!(library.get_by_hostname("trip.local").is_some());
        assert!(library.get_by_hostname("other.local").is_none());
    }

    #[test]
    fn a_relay_hostname_resolves_to_the_app_it_names() {
        // The whole point of the handle scheme: one claim covers every app on
        // the machine, and a new folder is reachable the moment it exists
        // without anything being configured anywhere.
        let library = Library::new();
        library.replace(vec![
            record(
                "chores-rota",
                "Chores rota",
                &std::env::temp_dir().display().to_string(),
            ),
            record(
                "trip-briefing-html-template",
                "Trip briefing",
                &std::env::temp_dir().display().to_string(),
            ),
        ]);
        library.set_relay_identity(Some(("adarsh".into(), "kitchentable.cloud".into())));

        assert_eq!(
            library
                .get_by_hostname("chores-rota-adarsh.kitchentable.cloud")
                .map(|a| a.slug),
            Some("chores-rota".to_string())
        );
        // A slug with hyphens of its own splits at the *last* one, which is why
        // a handle may never contain one.
        assert_eq!(
            library
                .get_by_hostname("trip-briefing-html-template-adarsh.kitchentable.cloud")
                .map(|a| a.slug),
            Some("trip-briefing-html-template".to_string())
        );
        // Case and a trailing dot are both legal in a Host header.
        assert!(library
            .get_by_hostname("Chores-Rota-Adarsh.KitchenTable.Cloud.")
            .is_some());
    }

    #[test]
    fn a_hostname_this_machine_does_not_own_resolves_to_nothing() {
        // kt-tunnel-proto says a daemon receiving a hostname it does not serve
        // should refuse rather than guess. Every one of these is a way of
        // guessing wrong, and the most important is the second: a request
        // misrouted to us must not be served from whatever local app happens
        // to share a slug with somebody else's.
        let library = Library::new();
        library.replace(vec![record(
            "chores-rota",
            "Chores rota",
            &std::env::temp_dir().display().to_string(),
        )]);
        library.set_relay_identity(Some(("adarsh".into(), "kitchentable.cloud".into())));

        for hostname in [
            "chores-rota-someoneelse.kitchentable.cloud", // another install's handle
            "chores-rota-adarsh.example.com",             // another domain
            "a.chores-rota-adarsh.kitchentable.cloud",    // deeper than the wildcard
            "chores-rota.kitchentable.cloud",             // no handle at all
            "-adarsh.kitchentable.cloud",                 // no app named
            "nosuchapp-adarsh.kitchentable.cloud",        // a handle we own, an app we do not
        ] {
            assert!(
                library.get_by_hostname(hostname).is_none(),
                "{hostname} should not have resolved"
            );
        }
    }

    #[test]
    fn an_install_with_no_handle_answers_no_relay_hostname() {
        // Every install today. Nothing has been claimed, so nothing public
        // resolves - which is correct rather than a limitation.
        let library = Library::new();
        library.replace(vec![record(
            "chores-rota",
            "Chores rota",
            &std::env::temp_dir().display().to_string(),
        )]);

        assert!(library
            .get_by_hostname("chores-rota-adarsh.kitchentable.cloud")
            .is_none());
    }

    #[test]
    fn a_rescan_does_not_re_announce_what_is_already_on_the_air() {
        let dir = std::env::temp_dir().display().to_string();
        let announcer = kt_mdns::MockAnnouncer::new();
        let library = Library::new();
        let records = vec![record("trip", "Trip", &dir)];

        library.replace_announcing(
            records.clone(),
            Some(&announcer),
            Some(std::net::Ipv4Addr::LOCALHOST),
        );
        library.replace_announcing(
            records,
            Some(&announcer),
            Some(std::net::Ipv4Addr::LOCALHOST),
        );

        assert_eq!(
            announcer.announced(),
            ["trip"],
            "re-announcing would make every phone on the network re-resolve"
        );
    }

    #[test]
    fn an_app_that_cannot_be_announced_is_still_served() {
        let dir = std::env::temp_dir().display().to_string();
        let announcer = kt_mdns::MockAnnouncer::new().failing("no permission");
        let library = Library::new();

        library.replace_announcing(
            vec![record("trip", "Trip", &dir)],
            Some(&announcer),
            Some(std::net::Ipv4Addr::LOCALHOST),
        );

        assert_eq!(library.len(), 1, "mDNS failing must not remove the app");
        assert!(library.get("trip").is_some());
        assert_eq!(library.hostname("trip"), None);
    }

    #[test]
    fn listings_are_name_ordered() {
        let dir = std::env::temp_dir().display().to_string();
        let library = Library::new();
        library.replace(vec![
            record("z", "Alpha", &dir),
            record("a", "zulu", &dir),
            record("m", "Middle", &dir),
        ]);

        let names: Vec<_> = library.list().into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["Alpha", "Middle", "zulu"]);
    }
}
