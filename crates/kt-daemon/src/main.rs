//! Kitchen Table daemon: wiring, and the entrypoint.
//!
//! The daemon is the product. The Tauri shell, the CLI, and agents are all
//! clients of the socket API; nothing the UI can do is impossible from the
//! socket (docs/architecture.md section 1). It never depends on the shell:
//! killing the window changes nothing about serving.

use std::sync::Arc;
use std::time::Instant;

use kt_registry::Registry;
use kt_server::AppSource;
use kt_store::Store;
use kt_types::{paths, AppRecord, DegradedReason, Event, ServingState, Urls};

mod authoring;
mod keys;
mod library;
mod relay;
mod rpc;
mod socket;
mod storage;
mod trust;

use library::Library;

/// Ports we try, in order. Port 80 first so `http://hostname.local/app` works
/// with no port in the URL; a high port when something else already has it,
/// which on a developer's machine is most of the time.
const DEFAULT_PORTS: &[u16] = &[80, 8420, 8421, 8422];

/// Ports to try this run. `KT_PORTS` overrides, comma-separated, so a test can
/// pin a port instead of racing whatever else is on the machine.
fn ports() -> Vec<u16> {
    match std::env::var("KT_PORTS") {
        Ok(raw) => {
            let parsed: Vec<u16> = raw
                .split(',')
                .filter_map(|p| p.trim().parse().ok())
                .collect();
            if parsed.is_empty() {
                DEFAULT_PORTS.to_vec()
            } else {
                parsed
            }
        }
        Err(_) => DEFAULT_PORTS.to_vec(),
    }
}

#[derive(Debug, thiserror::Error)]
enum StartupError {
    #[error("HOME is not set, so there is nowhere to keep state")]
    NoHome,
    #[error("could not open the workspace: {0}")]
    Workspace(#[from] kt_registry::RegistryError),
    #[error("could not open the database: {0}")]
    Store(#[from] kt_store::StoreError),
    #[error(transparent)]
    Socket(#[from] socket::SocketError),
    #[error("no port available; tried {0:?}")]
    NoPort(Vec<u16>),
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "kt_daemon=info,kt_registry=info,kt_store=info,kt_mdns=info,kt_server=info".into()
            }),
        )
        .init();

    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), StartupError> {
    let home = paths::home().map_err(|_| StartupError::NoHome)?;

    // Overridable so tests and a second developer instance can run without
    // fighting over the real workspace.
    let workspace = std::env::var("KT_WORKSPACE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| paths::default_workspace(&home));

    let registry = Registry::new(&workspace);
    registry.ensure_workspace()?;

    let store = Arc::new(Store::open(&paths::system_db_path(&home))?);
    // Each app's own key-value store, one SQLite file each. Shared between the
    // HTTP layer, which apps write through, and the socket, which reads them
    // back for the owner's Storage tab.
    let stores = Arc::new(kt_store::Storage::new(paths::storage_dir(&home)));
    let library = Arc::new(Library::new());
    // Created before anything that publishes, so the watcher, the gate and the
    // pairing path all share one bus.
    let events = rpc::Events::new();
    // Who currently has an app open. Shared with the HTTP server, which hears
    // the check-ins, and with the socket, which reports them.
    let presence = Arc::new(kt_server::Presence::new());

    // The session key outlives the process, so a restart is invisible to
    // everyone who has already paired. Tests set KT_NO_KEYCHAIN: the e2e suite
    // spawns real daemons, and one shared login-keychain item would defeat the
    // per-test HOME isolation they depend on.
    let secrets: Arc<dyn kt_certs::SecretStore> = if std::env::var_os("KT_NO_KEYCHAIN").is_some() {
        Arc::new(kt_certs::FileStore::new(&paths::state_dir(&home)))
    } else {
        Arc::from(kt_certs::for_this_platform(&paths::state_dir(&home)))
    };
    let keys = keys::load_or_create(secrets);
    // The public half only. It is what the owner links to an account and what
    // support asks for; the private half stays inside the identity.
    let install_key = keys.install.as_ref().map(|i| i.public().to_string());

    let relay_identity = keys.install.clone();
    // Shared between the relay task, which writes it, and `sys.status`, which
    // reads it. Built here even when there is no relay, because "off" is a
    // real answer the window needs and not the absence of one.
    let relay_status = Arc::new(relay::RelayStatus::new());
    let trust = Arc::new(trust::Trust::new(
        Arc::clone(&store),
        keys.session,
        events.clone(),
    ));

    let (listener, port, port_degraded) = bind_http().await?;

    // Announce every app on the network before anything else can ask for a
    // URL, so app.list never reports a hostname that is not yet on the air.
    // Tests set KT_NO_MDNS so a CI run does not publish hostnames onto
    // whatever network it happens to be on.
    let announcer: Box<dyn kt_mdns::Announcer> = if std::env::var_os("KT_NO_MDNS").is_some() {
        Box::new(kt_mdns::UnsupportedAnnouncer)
    } else {
        kt_mdns::for_this_platform()
    };
    let addr = kt_mdns::primary_ipv4();
    if addr.is_none() {
        tracing::warn!("no network address found; apps are reachable on this machine only");
    }
    let mdns_live = announcer.is_live() && addr.is_some();

    let urls = Urls {
        scheme: "http".to_string(),
        hostname: None,    // filled per app from the library
        public_host: None, // likewise, and only once a handle is claimed
        port_suffix: if port == 80 {
            String::new()
        } else {
            format!(":{port}")
        },
        prefix_origin: origin_for("localhost", port),
        fallback_origin: origin_for(
            &addr
                .map(|a| a.to_string())
                .unwrap_or_else(|| "127.0.0.1".into()),
            port,
        ),
    };

    // No publish on the first scan: nobody can have subscribed yet, and every
    // app would look like a change.
    announce_all(&registry, &store, &library, announcer.as_ref(), addr, None);

    let serving = match (port_degraded, mdns_live) {
        (Some(reason), _) => ServingState::Degraded {
            reason,
            message: format!(
                "Port 80 was already in use, so apps are served on port {port} for now."
            ),
        },
        (None, false) => ServingState::Degraded {
            reason: DegradedReason::MdnsUnavailable,
            message: "Friendly .local names are not available, so apps are reachable \
                      by IP address instead."
                .to_string(),
        },
        (None, true) => ServingState::Serving,
    };

    let socket_path = paths::socket_path(&home);
    let socket_listener = socket::bind(&socket_path).await?;

    // One rescan closure, shared by the watcher and the socket. `app.create`
    // has to answer with the app it just made, and waiting out the watcher's
    // debounce - tuned for someone finishing a folder copy - is far too long to
    // block a socket call on.
    let rescan: rpc::Rescan = {
        let registry = Registry::new(&workspace);
        let store = Arc::clone(&store);
        let library = Arc::clone(&library);
        let announcer: Arc<dyn kt_mdns::Announcer> = if std::env::var_os("KT_NO_MDNS").is_some() {
            Arc::new(kt_mdns::UnsupportedAnnouncer)
        } else {
            Arc::from(kt_mdns::for_this_platform())
        };
        let events = events.clone();
        let urls = urls.clone();
        Arc::new(move || {
            announce_all(
                &registry,
                &store,
                &library,
                &*announcer,
                addr,
                Some((&events, &urls)),
            )
        })
    };

    let presence_for_http = Arc::clone(&presence);

    let ctx = Arc::new(rpc::Context {
        library: Arc::clone(&library),
        store: Arc::clone(&store),
        workspace: workspace.display().to_string(),
        urls: urls.clone(),
        serving,
        started: Instant::now(),
        rescan: Arc::clone(&rescan),
        events: events.clone(),
        presence: Arc::clone(&presence),
        install_key,
        relay: Arc::clone(&relay_status),
        stores: Arc::clone(&stores),
    });

    // Nobody tells us when a page stops checking in - that is the whole point
    // of a timeout - so someone has to look. Sweeping on a timer, rather than
    // only when the window asks, means the owner sees a tab close whether or
    // not they happen to be looking at that app.
    {
        let events = events.clone();
        let presence = Arc::clone(&presence);
        let library = Arc::clone(&library);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(kt_server::presence::LINGER / 4);
            loop {
                tick.tick().await;
                if presence.sweep() {
                    for record in library.records() {
                        let slug = record.manifest.slug;
                        events.send(Event::Presence {
                            viewers: presence.viewers(&slug),
                            app_slug: slug,
                        });
                    }
                }
            }
        });
    }

    // Rescan whenever the workspace settles. The watcher must outlive this
    // scope or the watch stops, hence the binding.
    let _watcher = Registry::new(&workspace).watch(move || rescan())?;

    tracing::info!(
        workspace = %workspace.display(),
        socket = %socket_path.display(),
        apps = library.len(),
        origin = %urls.prefix_origin,
        mdns = mdns_live,
        "kitchen table is serving"
    );

    // Choose a cryptography provider before anything can open a TLS connection.
    //
    // rustls 0.23 will not guess when zero or two are compiled in; it returns
    // an error at the first handshake instead. Nothing here caught that,
    // because the relay's tests use a stub edge on plain `ws://` - deliberately,
    // so that a test does not have to stand up a certificate authority - so the
    // first real `wss://` dial was the first TLS this code had ever done.
    //
    // Installed rather than left to feature detection so the choice is visible
    // in one place and cannot be changed by a dependency enabling the other.
    if rustls::crypto::ring::default_provider()
        .install_default()
        .is_err()
    {
        tracing::debug!("a cryptography provider was already installed");
    }

    // Which public names this machine answers to. A placeholder for the handle
    // claim that arrives with account linking, in the same spirit as
    // KT_RELAY_URL - and, like it, absent on every install today. Without one
    // the daemon resolves no relay hostname at all, which is correct: nothing
    // has been published.
    if let Some((handle, domain)) = relay_handle() {
        tracing::info!(%handle, %domain, "relay hostnames configured");
        library.set_relay_identity(Some((handle, domain)));
    }

    // Each app's own data, in its own file under the state directory. One
    // instance, shared with the socket so the Storage tab reads what the apps
    // wrote rather than a second set of connections to the same files.
    let app_storage: kt_server::storage::Shared = Arc::new(storage::Apps::new(Arc::clone(&stores)));
    let app = kt_server::router_with_storage(library, trust, presence_for_http, app_storage);

    // Dial the relay, if this install has one and knows who it is.
    //
    // Spawned and never awaited: serving on the local network must not wait on
    // a network dial, and must not stop because one failed. A daemon with no
    // relay is not a degraded daemon - it is the free tier.
    //
    // It gets the same router the LAN listener uses, so a relayed request meets
    // the same gate rather than a second implementation of one. What it does
    // not get is connect info: see relay::session.
    match (relay::Config::from_env(), relay_identity) {
        (Some(config), Some(identity)) => {
            tracing::info!(url = config.url, "relay configured; dialling");
            let events = events.clone();
            tokio::spawn(relay::run(
                config,
                identity,
                Arc::new(app.clone()),
                Arc::clone(&relay_status),
                // Pushed as it happens. A window that learns the tunnel dropped
                // sixty seconds late is a window that let somebody send a link
                // in the meantime.
                move |state| {
                    events.send(Event::RelayChanged {
                        relay: state.into(),
                    })
                },
            ));
        }
        (Some(_), None) => tracing::warn!(
            "a relay is configured but this install has no identity, so there \
             is nothing to dial with; see the keystore warnings above"
        ),
        (None, _) => {}
    }

    let http = tokio::spawn(async move {
        // with_connect_info, or the peer address never reaches the gate and
        // the owner's own browser would be asked to pair with itself.
        let service = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
        if let Err(e) = axum::serve(listener, service).await {
            tracing::error!(error = %e, "http server stopped");
        }
    });

    tokio::select! {
        _ = socket::serve(socket_listener, ctx) => {}
        _ = http => {}
        _ = tokio::signal::ctrl_c() => tracing::info!("shutting down"),
    }

    socket::cleanup(&socket_path);
    Ok(())
}

/// Rescan, persist, and announce.
fn announce_all(
    registry: &Registry,
    store: &Store,
    library: &Library,
    announcer: &dyn kt_mdns::Announcer,
    addr: Option<std::net::Ipv4Addr>,
    publish: Option<(&rpc::Events, &Urls)>,
) {
    sync_with(registry, store, library, Some(announcer), addr, publish)
}

/// Rescan the workspace and make the store and the live library agree.
///
/// Infallible by design: a transient filesystem error must not take serving
/// down, so it is logged and the previous library keeps serving.
fn sync_with(
    registry: &Registry,
    store: &Store,
    library: &Library,
    announcer: Option<&dyn kt_mdns::Announcer>,
    addr: Option<std::net::Ipv4Addr>,
    publish: Option<(&rpc::Events, &Urls)>,
) {
    let records = match registry.scan() {
        Ok(records) => records,
        Err(e) => {
            tracing::warn!(error = %e, "workspace scan failed; keeping the current library");
            return;
        }
    };

    // Folders the owner told us to forget. Dropped here rather than in the
    // registry because the registry's job is to report what is on disk, and
    // this is a decision about what Kitchen Table does with it. The folder is
    // still there, untouched, which is what the window promised.
    let forgotten = store.forgotten_paths().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "could not read the forgotten list");
        Vec::new()
    });
    let records: Vec<_> = records
        .into_iter()
        .filter(|record| !forgotten.contains(&record.path))
        .collect();

    let slugs: Vec<String> = records.iter().map(|r| r.manifest.slug.clone()).collect();

    for record in &records {
        if let Err(e) = store.upsert_app(record) {
            tracing::warn!(slug = %record.manifest.slug, error = %e, "could not persist app");
        }
    }
    match store.retain_apps(&slugs) {
        Ok(removed) if !removed.is_empty() => tracing::info!(?removed, "apps left the workspace"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "could not prune removed apps"),
    }

    // What the library held before the swap, so the events describe changes
    // rather than restating the whole workspace on every scan. A watcher that
    // fires on any write would otherwise have the window refetching everything
    // because one file's timestamp moved.
    let was: std::collections::HashMap<String, AppFingerprint> = library
        .records()
        .iter()
        .map(|record| (record.manifest.slug.clone(), fingerprint(record)))
        .collect();

    let before = library.list().len();
    library.replace_announcing(records, announcer, addr);
    let after = library.len();
    if before != after {
        tracing::info!(apps = after, "library updated");
    }

    let Some((events, urls)) = publish else {
        return;
    };

    let mut still_here = std::collections::HashSet::new();
    for record in library.records() {
        let slug = record.manifest.slug.clone();
        still_here.insert(slug.clone());
        if was.get(&slug) != Some(&fingerprint(&record)) {
            let app_urls = rpc::urls_for(urls, library, &slug);
            events.send(Event::AppChanged {
                app: Box::new(record.to_app(&app_urls)),
            });
        }
    }
    for slug in was.into_keys() {
        if !still_here.contains(&slug) {
            events.send(Event::AppRemoved { slug });
        }
    }
}

/// Enough of an app to tell "nothing moved" from "something the window should
/// redraw". Deliberately not the whole record: the mtime of a file nobody
/// serves is not a change anyone needs to hear about.
type AppFingerprint = (u32, kt_types::Visibility, u64, Option<u64>, bool, String);

fn fingerprint(record: &AppRecord) -> AppFingerprint {
    (
        record.manifest.version,
        record.manifest.visibility,
        record.size_bytes,
        record.deployed_at,
        record.entry_exists,
        record.manifest.name.clone(),
    )
}

/// An origin with the port left off when it is the default, because a URL
/// someone reads aloud or texts to a family member should be as short as it
/// can honestly be.
/// This install's handle and the domain its apps hang off, if it has been told.
///
/// `KT_RELAY_HANDLE=adarsh` plus an optional `KT_RELAY_DOMAIN`, defaulting to
/// the one the product uses. Both are placeholders for the handle claim that
/// arrives with account linking; neither is set on any install today.
///
/// A handle containing a hyphen or a dot is refused rather than used: hostnames
/// split at the last hyphen, so a handle with one in it makes every name
/// ambiguous. Better to answer nothing than to answer the wrong app.
fn relay_handle() -> Option<(String, String)> {
    let handle = std::env::var("KT_RELAY_HANDLE").ok()?;
    let handle = handle.trim().to_ascii_lowercase();
    if handle.is_empty() || handle.contains('-') || handle.contains('.') {
        if !handle.is_empty() {
            tracing::warn!(%handle, "a relay handle may not contain a hyphen or a dot; ignoring it");
        }
        return None;
    }

    let domain = std::env::var("KT_RELAY_DOMAIN")
        .ok()
        .map(|d| d.trim().to_ascii_lowercase())
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "kitchentable.cloud".to_string());

    Some((handle, domain))
}

fn origin_for(host: &str, port: u16) -> String {
    if port == 80 {
        format!("http://{host}")
    } else {
        format!("http://{host}:{port}")
    }
}

/// Try each port in turn. Returns the listener, the port, and why we are
/// degraded if we did not get the one we wanted.
async fn bind_http() -> Result<(tokio::net::TcpListener, u16, Option<DegradedReason>), StartupError>
{
    let candidates = ports();
    for (i, port) in candidates.iter().enumerate() {
        match tokio::net::TcpListener::bind(("0.0.0.0", *port)).await {
            Ok(listener) => {
                let degraded = (i > 0).then_some(DegradedReason::PortFallback);
                if degraded.is_some() {
                    tracing::warn!(port, "port 80 unavailable, using a fallback port");
                }
                return Ok((listener, *port, degraded));
            }
            Err(e) => tracing::debug!(port, error = %e, "port unavailable"),
        }
    }
    Err(StartupError::NoPort(candidates))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_port_is_left_out_of_urls() {
        assert_eq!(origin_for("localhost", 80), "http://localhost");
        assert_eq!(origin_for("trip.local", 80), "http://trip.local");
    }

    #[test]
    fn a_fallback_port_is_shown_because_it_has_to_be_typed() {
        assert_eq!(origin_for("localhost", 8420), "http://localhost:8420");
    }
}
