//! Linking this install to an account, and what that switches on.
//!
//! An account is the paid tier's one object: the subscription, the handle, and
//! the way published links survive a dead laptop. The daemon's half of it is
//! deliberately small, because the ceremony happens in a browser - Stripe's
//! checkout page collects the email and the card, the cloud creates the
//! account from the receipt and binds this install's key to it from the
//! session metadata, and nothing here ever sees a password, a token, or a card
//! number. What comes back down is only the public outcome: a handle, a
//! domain, and where to dial.
//!
//! # The shape of the flow
//!
//! 1. `account.begin_upgrade` answers with a URL - the cloud's upgrade page
//!    with this install's public key in the query string. No network call:
//!    the address is knowable offline, and a button that cannot even open a
//!    browser without a round trip is a button that hangs.
//! 2. The browser does the whole checkout. Meanwhile the daemon polls the
//!    cloud, asking one question signed by nothing more than knowing its own
//!    public key: "is this install linked yet, and as whom?"
//! 3. The moment the answer is yes, the handle is persisted, every app's
//!    public hostname starts resolving, and the relay is dialled. The window
//!    hears `account_changed` rather than discovering it on a poll.
//!
//! # Environment overrides
//!
//! `KT_RELAY_HANDLE`, `KT_RELAY_DOMAIN` and `KT_RELAY_URL` outrank the stored
//! link, in the same spirit as `KT_WORKSPACE`: they are how a test and a dev
//! stack get a linked-looking daemon without a cloud to talk to, and a stored
//! setting that could override them would make those runs depend on whatever
//! was linked last. `account.unlink` refuses under an override rather than
//! appearing to work, for the same reason Settings draws a locked workspace
//! as locked.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use kt_store::Store;
use kt_tunnel_proto::InstallIdentity;
use kt_types::{AccountLink, AccountStatus, Event};

use crate::library::Library;
use crate::relay::{self, RelayStatus};
use crate::rpc::Events;

/// Where the linked handle lives between runs. Settings rather than the
/// keystore: a handle is public - it is printed on every URL the owner shares -
/// and the keystore's job is things that must not leak, not things that must
/// merely persist.
const HANDLE_SETTING: &str = "relay.handle";
const DOMAIN_SETTING: &str = "relay.domain";
const URL_SETTING: &str = "relay.url";

const DEFAULT_DOMAIN: &str = "kitchentable.cloud";
/// Where `begin_upgrade` sends the browser. The page owns the checkout: it
/// creates the Stripe session server-side with the install key in the
/// metadata, so the daemon needs no Stripe vocabulary at all. The api host
/// rather than the apex: the apex is the static marketing site, and the
/// upgrade page is served by the control plane.
const DEFAULT_SITE: &str = "https://api.kitchentable.cloud";
/// The accounts API the daemon polls.
const DEFAULT_API: &str = "https://api.kitchentable.cloud";
/// Where a linked install dials when the cloud did not say otherwise. The
/// answer from the API wins and is persisted; this exists so an install linked
/// by an older cloud that omitted the field still has somewhere to go.
/// The deployed edge's listener, port included (see the cloud repo's
/// beta-deployment.md); the daemon must not have to guess it.
const DEFAULT_RELAY_URL: &str = "wss://tunnel.kitchentable.cloud:8443";

/// How often to ask whether the checkout has landed. Human-paced: the person
/// is typing a card number, and the difference between learning in one second
/// and three is invisible next to that.
const POLL_EVERY: Duration = Duration::from_secs(3);
/// How long to keep asking before deciding nobody is coming back. Long,
/// because an abandoned checkout costs one cheap request every few seconds,
/// and a poll that gives up while somebody is hunting for their wallet turns
/// a paid-for link into a "nothing happened".
const GIVE_UP_AFTER: Duration = Duration::from_secs(30 * 60);

/// The cloud's two addresses, resolved once at startup.
#[derive(Debug, Clone)]
pub struct Cloud {
    /// The website carrying the upgrade page.
    pub site: String,
    /// The accounts API.
    pub api: String,
}

impl Cloud {
    /// `KT_CLOUD_SITE` and `KT_CLOUD_API` override, so a dev stack and the
    /// account tests can stand in for the real thing on localhost.
    pub fn from_env() -> Self {
        let read = |name: &str, default: &str| {
            std::env::var(name)
                .ok()
                .map(|v| v.trim().trim_end_matches('/').to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| default.to_string())
        };
        Self {
            site: read("KT_CLOUD_SITE", DEFAULT_SITE),
            api: read("KT_CLOUD_API", DEFAULT_API),
        }
    }
}

/// What the accounts API says about one install.
///
/// The daemon asks with nothing but the install's public key, and everything
/// in the answer is as public as the key is: the handle appears in every URL
/// the owner hands out. Nothing sensitive rides this poll, which is what lets
/// it be a bare GET.
#[derive(Debug, serde::Deserialize)]
struct LinkAnswer {
    linked: bool,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    relay_url: Option<String>,
}

/// A handle fit to build hostnames from, or nothing.
///
/// Hostnames split at the last hyphen, so a handle containing one makes every
/// name ambiguous; a dot would change how many DNS labels the name has.
/// Refused wherever a handle arrives - the environment, the store, the cloud -
/// because better to answer nothing than to answer the wrong app.
fn valid_handle(raw: &str) -> Option<String> {
    let handle = raw.trim().to_ascii_lowercase();
    if handle.is_empty() || handle.contains('-') || handle.contains('.') {
        if !handle.is_empty() {
            tracing::warn!(handle = %raw, "a relay handle may not contain a hyphen or a dot; ignoring it");
        }
        return None;
    }
    Some(handle)
}

/// A stored setting, treating an empty string as absent.
///
/// The store has no delete, so `unlink` writes `""` - and reading that back as
/// a real value would resurrect the link it just cleared.
fn stored(store: &Store, key: &str) -> Option<String> {
    store
        .setting(key)
        .unwrap_or_else(|e| {
            tracing::warn!(key, error = %e, "could not read a relay setting");
            None
        })
        .filter(|v| !v.is_empty())
}

/// The identity to publish under this run, and whether the environment chose
/// it. Environment first, then the stored link, then nothing - see the module
/// docs for why that order is fixed.
fn resolve_identity(store: &Store) -> Option<(String, String, bool)> {
    if let Ok(raw) = std::env::var("KT_RELAY_HANDLE") {
        if let Some(handle) = valid_handle(&raw) {
            let domain = std::env::var("KT_RELAY_DOMAIN")
                .ok()
                .map(|d| d.trim().to_ascii_lowercase())
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| DEFAULT_DOMAIN.to_string());
            return Some((handle, domain, true));
        }
        return None;
    }

    let handle = valid_handle(&stored(store, HANDLE_SETTING)?)?;
    let domain = stored(store, DOMAIN_SETTING).unwrap_or_else(|| DEFAULT_DOMAIN.to_string());
    Some((handle, domain, false))
}

/// Where to dial, under the same precedence as the identity.
fn resolve_relay_url(store: &Store, linked: bool) -> Option<String> {
    if let Some(config) = relay::Config::from_env() {
        return Some(config.url);
    }
    if let Some(url) = stored(store, URL_SETTING) {
        return Some(url);
    }
    linked.then(|| DEFAULT_RELAY_URL.to_string())
}

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error(
        "this machine has no install identity, so there is nothing to link an \
         account to; see the keystore warnings in the daemon log"
    )]
    NoIdentity,
    #[error("this install is already linked as {handle:?}")]
    AlreadyLinked { handle: String },
    #[error(
        "the relay identity is set by the environment (KT_RELAY_HANDLE), which \
         outranks anything stored; unset it to unlink"
    )]
    EnvironmentOutranks,
}

/// What `account.begin_upgrade` hands back: where to send the browser.
#[derive(Debug, serde::Serialize)]
pub struct UpgradeStarted {
    pub url: String,
}

/// The account state machine, and the relay task it owns.
///
/// Owns the relay's lifecycle because linking is what makes a relay dialable:
/// before this existed the dial happened once at startup from the environment,
/// and a link that landed mid-run would have sat inert until a restart.
pub struct Manager {
    cloud: Cloud,
    identity: Option<Arc<InstallIdentity>>,
    store: Arc<Store>,
    library: Arc<Library>,
    relay_status: Arc<RelayStatus>,
    events: Events,
    /// The same router the LAN listener serves, so a relayed request meets the
    /// same gate rather than a second implementation of one.
    router: Arc<axum::Router>,
    link: Mutex<AccountLink>,
    relay_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    poll_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// The poll cadence, a field so the tests do not take three seconds a step.
    /// Nothing outside tests should change it: the numbers are chosen above.
    pub(crate) poll_every: Duration,
    pub(crate) give_up_after: Duration,
}

impl Manager {
    pub fn new(
        cloud: Cloud,
        identity: Option<Arc<InstallIdentity>>,
        store: Arc<Store>,
        library: Arc<Library>,
        relay_status: Arc<RelayStatus>,
        events: Events,
        router: Arc<axum::Router>,
    ) -> Self {
        Self {
            cloud,
            identity,
            store,
            library,
            relay_status,
            events,
            router,
            link: Mutex::new(AccountLink::Unlinked),
            relay_task: Mutex::new(None),
            poll_task: Mutex::new(None),
            poll_every: POLL_EVERY,
            give_up_after: GIVE_UP_AFTER,
        }
    }

    /// Bring the stored (or environment-set) link back to life.
    ///
    /// Called once at startup. Restores the library's relay identity so public
    /// hostnames resolve, and dials the relay if there is anywhere to dial.
    pub fn start(self: &Arc<Self>) {
        let resolved = resolve_identity(&self.store);
        if let Some((handle, domain, from_env)) = &resolved {
            tracing::info!(%handle, %domain, from_env, "relay hostnames configured");
            self.library
                .set_relay_identity(Some((handle.clone(), domain.clone())));
            *self.lock_link() = AccountLink::Linked {
                handle: handle.clone(),
                domain: domain.clone(),
            };
        }

        match (
            resolve_relay_url(&self.store, resolved.is_some()),
            self.identity.clone(),
        ) {
            (Some(url), Some(identity)) => self.spawn_relay(url, identity),
            (Some(_), None) => tracing::warn!(
                "a relay is configured but this install has no identity, so there \
                 is nothing to dial with; see the keystore warnings above"
            ),
            (None, _) => {}
        }
    }

    pub fn status(&self) -> AccountStatus {
        AccountStatus {
            install_key: self.install_key(),
            link: self.lock_link().clone(),
        }
    }

    fn install_key(&self) -> Option<String> {
        self.identity.as_ref().map(|i| i.public().to_string())
    }

    /// Start the upgrade: answer with the page to open, and begin asking the
    /// cloud whether the checkout has landed.
    ///
    /// Idempotent while waiting - a second press reuses the same URL and does
    /// not stack a second poll - because the first thing anyone does to a
    /// button that seems not to have worked is press it again.
    pub fn begin_upgrade(self: &Arc<Self>) -> Result<UpgradeStarted, AccountError> {
        let Some(key) = self.install_key() else {
            return Err(AccountError::NoIdentity);
        };
        if let AccountLink::Linked { handle, .. } = &*self.lock_link() {
            return Err(AccountError::AlreadyLinked {
                handle: handle.clone(),
            });
        }

        let url = format!("{}/upgrade?install={key}", self.cloud.site);

        let began = {
            let mut link = self.lock_link();
            let began = *link == AccountLink::Unlinked;
            *link = AccountLink::Waiting;
            began
        };
        if began {
            self.announce();
        }
        self.ensure_polling(key);

        Ok(UpgradeStarted { url })
    }

    /// Forget the link on this machine.
    ///
    /// Local by design, for now: the cloud's side of unlinking - releasing the
    /// install from the account so a dashboard stops listing this machine -
    /// arrives with the dashboard. What this must do today is stop publishing
    /// and stop claiming to.
    pub fn unlink(self: &Arc<Self>) -> Result<AccountStatus, AccountError> {
        if std::env::var_os("KT_RELAY_HANDLE").is_some() {
            return Err(AccountError::EnvironmentOutranks);
        }

        if let Some(task) = self
            .poll_task
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            task.abort();
        }
        if let Some(task) = self
            .relay_task
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            task.abort();
        }
        // The tunnel is gone the moment the task is; saying so is not optional.
        if self.relay_status.set(relay::RelayState::Off) {
            self.events.send(Event::RelayChanged {
                relay: self.relay_status.get().into(),
            });
        }

        for key in [HANDLE_SETTING, DOMAIN_SETTING, URL_SETTING] {
            if let Err(e) = self.store.set_setting(key, "") {
                tracing::warn!(key, error = %e, "could not clear a relay setting");
            }
        }
        self.library.set_relay_identity(None);
        *self.lock_link() = AccountLink::Unlinked;
        self.announce();

        Ok(self.status())
    }

    /// The linked answer arrived: persist it, publish under it, dial with it.
    fn finalize(self: &Arc<Self>, handle: String, domain: String, relay_url: Option<String>) {
        let url = relay_url.unwrap_or_else(|| DEFAULT_RELAY_URL.to_string());
        for (key, value) in [
            (HANDLE_SETTING, handle.as_str()),
            (DOMAIN_SETTING, domain.as_str()),
            (URL_SETTING, url.as_str()),
        ] {
            if let Err(e) = self.store.set_setting(key, value) {
                // Worth saying loudly: everything works until the next restart,
                // which is the worst kind of working.
                tracing::warn!(key, error = %e, "could not persist the link; it will not survive a restart");
            }
        }

        tracing::info!(%handle, %domain, "account linked");
        self.library
            .set_relay_identity(Some((handle.clone(), domain.clone())));
        *self.lock_link() = AccountLink::Linked { handle, domain };
        self.announce();

        if let Some(identity) = self.identity.clone() {
            self.spawn_relay(url, identity);
        }
    }

    /// Keep exactly one poll running. A finished task is replaced; a live one
    /// is left alone.
    fn ensure_polling(self: &Arc<Self>, key: String) {
        let mut poll = self.poll_task.lock().unwrap_or_else(|e| e.into_inner());
        if poll.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }

        // Outside a runtime there is nowhere to poll from. Only the sync unit
        // tests get here; the daemon always calls this on its runtime.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::debug!("no runtime; not polling the accounts API");
            return;
        };

        let manager = Arc::clone(self);
        *poll = Some(runtime.spawn(async move {
            let deadline = tokio::time::Instant::now() + manager.give_up_after;
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("a client with only a timeout set");
            let url = format!("{}/v1/installs/{key}", manager.cloud.api);

            loop {
                match client.get(&url).send().await {
                    Ok(response) => match response.json::<LinkAnswer>().await {
                        Ok(answer) if answer.linked => {
                            // A linked answer with an unusable handle is a
                            // broken cloud, not a broken install; keep asking
                            // rather than storing something hostnames cannot
                            // be built from.
                            if let Some(handle) = answer.handle.as_deref().and_then(valid_handle) {
                                let domain =
                                    answer.domain.unwrap_or_else(|| DEFAULT_DOMAIN.to_string());
                                manager.finalize(handle, domain, answer.relay_url);
                                return;
                            }
                            tracing::warn!(
                                "the cloud says this install is linked but sent no \
                                 usable handle; still waiting"
                            );
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::debug!(error = %e, "unreadable answer from the accounts API")
                        }
                    },
                    // Quietly: a laptop mid-checkout on hotel wifi is exactly
                    // where this runs, and a poll is built to just try again.
                    Err(e) => tracing::debug!(error = %e, "could not reach the accounts API"),
                }

                if tokio::time::Instant::now() >= deadline {
                    tracing::info!("no link arrived; giving up until the next attempt");
                    let was_waiting = {
                        let mut link = manager.lock_link();
                        let was = *link == AccountLink::Waiting;
                        if was {
                            *link = AccountLink::Unlinked;
                        }
                        was
                    };
                    if was_waiting {
                        manager.announce();
                    }
                    return;
                }
                tokio::time::sleep(manager.poll_every).await;
            }
        }));
    }

    /// Dial, replacing any earlier task rather than racing it: two connections
    /// presenting one install key means the edge supersedes one of them, and
    /// which one wins would be a coin toss.
    fn spawn_relay(self: &Arc<Self>, url: String, identity: Arc<InstallIdentity>) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::debug!("no runtime; not dialling the relay");
            return;
        };

        let mut task = self.relay_task.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = task.take() {
            previous.abort();
        }

        tracing::info!(%url, "relay configured; dialling");
        let events = self.events.clone();
        *task = Some(runtime.spawn(relay::run(
            relay::Config { url },
            identity,
            Arc::clone(&self.router),
            Arc::clone(&self.relay_status),
            // Pushed as it happens. A window that learns the tunnel dropped
            // sixty seconds late is a window that let somebody send a link
            // in the meantime.
            move |state| {
                events.send(Event::RelayChanged {
                    relay: state.into(),
                })
            },
        )));
    }

    fn announce(&self) {
        self.events.send(Event::AccountChanged {
            account: self.status(),
        });
    }

    fn lock_link(&self) -> std::sync::MutexGuard<'_, AccountLink> {
        self.link.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path, routing::get, Json};
    use std::sync::atomic::{AtomicBool, Ordering};

    fn manager_with(cloud: Cloud, store: Arc<Store>, identity: bool) -> Arc<Manager> {
        let mut manager = Manager::new(
            cloud,
            identity.then(|| Arc::new(InstallIdentity::generate())),
            store,
            Arc::new(Library::new()),
            Arc::new(RelayStatus::new()),
            Events::new(),
            Arc::new(axum::Router::new()),
        );
        manager.poll_every = Duration::from_millis(10);
        manager.give_up_after = Duration::from_millis(400);
        Arc::new(manager)
    }

    fn nowhere() -> Cloud {
        Cloud {
            site: "https://site.test".into(),
            // A port nothing listens on, so a poll that runs fails fast
            // rather than dialling the real cloud from a test.
            api: "http://127.0.0.1:1".into(),
        }
    }

    /// An accounts API of one endpoint, whose answer a test can flip.
    async fn stub_api(linked: Arc<AtomicBool>, handle: &'static str) -> Cloud {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port");
        let api = format!("http://{}", listener.local_addr().expect("bound"));

        let app = axum::Router::new().route(
            "/v1/installs/{key}",
            get(move |Path(_key): Path<String>| {
                let linked = Arc::clone(&linked);
                async move {
                    if linked.load(Ordering::SeqCst) {
                        Json(serde_json::json!({
                            "linked": true,
                            "handle": handle,
                            "domain": "kitchentable.cloud",
                            "relay_url": "wss://tunnel.kitchentable.cloud",
                        }))
                    } else {
                        Json(serde_json::json!({ "linked": false }))
                    }
                }
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serves");
        });

        Cloud {
            site: "https://site.test".into(),
            api,
        }
    }

    async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
        for _ in 0..200 {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("never happened: {what}");
    }

    #[tokio::test]
    async fn the_upgrade_url_names_this_install() {
        // The whole handshake: the page creates the checkout with this key in
        // the metadata, which is what lets the webhook link the right machine
        // with no code to type.
        let manager = manager_with(
            nowhere(),
            Arc::new(Store::in_memory().expect("opens")),
            true,
        );
        let started = manager.begin_upgrade().expect("starts");

        let key = manager.status().install_key.expect("has one");
        assert_eq!(
            started.url,
            format!("https://site.test/upgrade?install={key}")
        );
        assert_eq!(manager.status().link, AccountLink::Waiting);
    }

    #[tokio::test]
    async fn no_identity_means_no_upgrade_rather_than_a_dead_checkout() {
        // Without an install key the webhook would have nothing to link, and
        // the person would pay for exactly nothing to happen.
        let manager = manager_with(
            nowhere(),
            Arc::new(Store::in_memory().expect("opens")),
            false,
        );
        assert!(matches!(
            manager.begin_upgrade(),
            Err(AccountError::NoIdentity)
        ));
    }

    #[tokio::test]
    async fn a_landed_checkout_links_publishes_and_persists() {
        let store = Arc::new(Store::in_memory().expect("opens"));
        let linked = Arc::new(AtomicBool::new(false));
        let cloud = stub_api(Arc::clone(&linked), "devi").await;
        let manager = manager_with(cloud, Arc::clone(&store), true);

        manager.begin_upgrade().expect("starts");
        assert_eq!(manager.status().link, AccountLink::Waiting);

        // The checkout lands on the cloud's side...
        linked.store(true, Ordering::SeqCst);

        // ...and the daemon notices on its own.
        eventually("the link landed", || {
            matches!(manager.status().link, AccountLink::Linked { .. })
        })
        .await;

        assert_eq!(
            manager.library.relay_identity(),
            Some(("devi".into(), "kitchentable.cloud".into())),
            "public hostnames resolve from the moment the link lands"
        );

        // And a restart finds it without asking the cloud anything.
        let second = manager_with(nowhere(), store, true);
        second.start();
        assert_eq!(
            second.status().link,
            AccountLink::Linked {
                handle: "devi".into(),
                domain: "kitchentable.cloud".into(),
            }
        );
    }

    #[tokio::test]
    async fn pressing_the_button_twice_does_not_stack_polls() {
        let manager = manager_with(
            nowhere(),
            Arc::new(Store::in_memory().expect("opens")),
            true,
        );
        let first = manager.begin_upgrade().expect("starts");
        let second = manager.begin_upgrade().expect("still fine");
        assert_eq!(first.url, second.url);
    }

    #[tokio::test]
    async fn an_abandoned_checkout_ends_back_at_unlinked() {
        // The stub that never answers, standing in for a person who closed the
        // tab. Waiting forever would leave the window saying "finish in your
        // browser" for a browser that is long gone.
        let store = Arc::new(Store::in_memory().expect("opens"));
        let linked = Arc::new(AtomicBool::new(false));
        let cloud = stub_api(linked, "devi").await;
        let manager = manager_with(cloud, store, true);

        manager.begin_upgrade().expect("starts");
        eventually("it gave up", || {
            manager.status().link == AccountLink::Unlinked
        })
        .await;
    }

    #[tokio::test]
    async fn unlink_clears_the_stored_link_and_the_published_names() {
        let store = Arc::new(Store::in_memory().expect("opens"));
        let linked = Arc::new(AtomicBool::new(true));
        let cloud = stub_api(linked, "devi").await;
        let manager = manager_with(cloud, Arc::clone(&store), true);

        manager.begin_upgrade().expect("starts");
        eventually("the link landed", || {
            matches!(manager.status().link, AccountLink::Linked { .. })
        })
        .await;

        manager.unlink().expect("unlinks");
        assert_eq!(manager.status().link, AccountLink::Unlinked);

        // A restart agrees: nothing comes back from the store.
        let second = manager_with(nowhere(), store, true);
        second.start();
        assert_eq!(second.status().link, AccountLink::Unlinked);
    }

    #[tokio::test]
    async fn a_linked_answer_with_a_broken_handle_is_not_stored() {
        // A handle with a hyphen would make every hostname on this machine
        // ambiguous. The cloud enforces this too; this is the daemon refusing
        // to trust that it did.
        let store = Arc::new(Store::in_memory().expect("opens"));
        let linked = Arc::new(AtomicBool::new(true));
        let cloud = stub_api(linked, "not-a-handle").await;
        let manager = manager_with(cloud, Arc::clone(&store), true);
        manager.begin_upgrade().expect("starts");

        eventually("it gave up rather than linking", || {
            manager.status().link == AccountLink::Unlinked
        })
        .await;
        assert_eq!(stored(&store, HANDLE_SETTING), None);
    }

    #[tokio::test]
    async fn an_already_linked_install_is_told_so() {
        let store = Arc::new(Store::in_memory().expect("opens"));
        store.set_setting(HANDLE_SETTING, "devi").expect("writes");
        let manager = manager_with(nowhere(), store, true);
        manager.start();

        assert!(matches!(
            manager.begin_upgrade(),
            Err(AccountError::AlreadyLinked { .. })
        ));
    }

    #[test]
    fn a_handle_with_a_hyphen_or_a_dot_is_refused() {
        assert_eq!(valid_handle("devi"), Some("devi".into()));
        assert_eq!(valid_handle("  Devi "), Some("devi".into()));
        assert_eq!(valid_handle("de-vi"), None);
        assert_eq!(valid_handle("de.vi"), None);
        assert_eq!(valid_handle(""), None);
    }

    #[test]
    fn an_empty_setting_reads_as_absent() {
        // Unlink writes "" because the store has no delete; reading it back as
        // a value would resurrect the link it just cleared.
        let store = Store::in_memory().expect("opens");
        store.set_setting(HANDLE_SETTING, "").expect("writes");
        assert_eq!(stored(&store, HANDLE_SETTING), None);
    }
}
