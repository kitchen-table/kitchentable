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
use kt_types::{paths, DegradedReason, ServingState, Urls};

mod library;
mod rpc;
mod socket;

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
    let library = Arc::new(Library::new());

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
        hostname: None, // filled per app from the library
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

    announce_all(&registry, &store, &library, announcer.as_ref(), addr);

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

    let ctx = Arc::new(rpc::Context {
        library: Arc::clone(&library),
        workspace: workspace.display().to_string(),
        urls: urls.clone(),
        serving,
        started: Instant::now(),
    });

    // Rescan whenever the workspace settles. The watcher must outlive this
    // scope or the watch stops, hence the binding.
    let _watcher = {
        let registry = Registry::new(&workspace);
        let store = Arc::clone(&store);
        let library = Arc::clone(&library);
        let announcer: Arc<dyn kt_mdns::Announcer> = if std::env::var_os("KT_NO_MDNS").is_some() {
            Arc::new(kt_mdns::UnsupportedAnnouncer)
        } else {
            Arc::from(kt_mdns::for_this_platform())
        };
        Registry::new(&workspace)
            .watch(move || announce_all(&registry, &store, &library, &*announcer, addr))?
    };

    tracing::info!(
        workspace = %workspace.display(),
        socket = %socket_path.display(),
        apps = library.len(),
        origin = %urls.prefix_origin,
        mdns = mdns_live,
        "kitchen table is serving"
    );

    let http = tokio::spawn(async move {
        let app = kt_server::router(library);
        if let Err(e) = axum::serve(listener, app).await {
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
) {
    sync_with(registry, store, library, Some(announcer), addr)
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
) {
    let records = match registry.scan() {
        Ok(records) => records,
        Err(e) => {
            tracing::warn!(error = %e, "workspace scan failed; keeping the current library");
            return;
        }
    };

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

    let before = library.list().len();
    library.replace_announcing(records, announcer, addr);
    let after = library.len();
    if before != after {
        tracing::info!(apps = after, "library updated");
    }
}

/// An origin with the port left off when it is the default, because a URL
/// someone reads aloud or texts to a family member should be as short as it
/// can honestly be.
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
