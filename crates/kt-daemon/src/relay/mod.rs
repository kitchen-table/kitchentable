//! The daemon's side of the relay tunnel.
//!
//! Kitchen Table serves from a machine behind a home router, which accepts no
//! incoming connections. The relay bridges that by turning the direction round:
//! the daemon dials *out* and holds the connection open, and viewer requests
//! come back down it. Nothing here asks anyone to forward a port.
//!
//! # Inert until asked
//!
//! Compiled in by default and doing nothing at all until it is configured, per
//! CLAUDE.md rule 10. [`Config::from_env`] returns `None` when `KT_RELAY_URL` is
//! unset, which is every install today, and [`run`] then never runs. A daemon
//! with no relay is not a degraded daemon - it is the free tier, which is the
//! whole product for most people.
//!
//! The environment variable is a placeholder for account linking, which is what
//! will eventually decide this. It is deliberately not a setting in `app.json`
//! or the database yet: making it persistent would mean designing where relay
//! configuration lives before there is a relay to configure.
//!
//! # What this module does and does not do
//!
//! It gets a connection up and keeps it up: dial, prove which machine this is,
//! prove it is still there, and decide what to do when it drops. It does not
//! carry requests yet - that is the mux layer, and it needs this to be solid
//! first.

mod backoff;
mod conn;
mod session;
#[cfg(test)]
mod stub;
mod wsbytes;

use std::sync::{Arc, RwLock};

use kt_tunnel_proto::{InstallIdentity, Retry};

pub use backoff::Backoff;
pub use conn::Ended;

/// Whether this machine is currently reachable from the internet.
///
/// A published app is only actually reachable while the tunnel is up, and for a
/// long time nothing anywhere said which. The window rendered a green ON badge
/// from the manifest - from the owner's *intention* - so an app that had been
/// silently unreachable for an hour looked perfectly healthy, and the way you
/// found out was a stranger telling you the link was broken.
///
/// This is the fact the badge should have been reading.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RelayState {
    /// No relay configured. Every install today, and not a fault: the free
    /// tier is the whole product for most people.
    #[default]
    Off,
    /// Dialling, or waiting to dial again after a drop.
    Connecting,
    /// Up. Published apps are reachable at their public addresses.
    Connected,
    /// Stopped trying, because waiting cannot fix it - a revoked install, an
    /// unlinked one, a version the edge will not speak. Somebody has to act,
    /// so the message is meant to be shown.
    NeedsAttention(String),
}

impl From<RelayState> for kt_types::RelayState {
    /// The internal state is the same shape as the wire one and deliberately a
    /// separate type: this module should not have to care that `sys.status`
    /// exists, and the wire type should not change because a field here did.
    fn from(state: RelayState) -> Self {
        match state {
            RelayState::Off => Self::Off,
            RelayState::Connecting => Self::Connecting,
            RelayState::Connected => Self::Connected,
            RelayState::NeedsAttention(message) => Self::NeedsAttention { message },
        }
    }
}

/// The current [`RelayState`], shared between the relay task and `sys.status`.
///
/// A lock rather than an event because the question "is it up *now*" has to be
/// answerable by anyone who asks at any time, including a window that has just
/// opened and missed every transition so far.
#[derive(Debug, Default)]
pub struct RelayStatus(RwLock<RelayState>);

impl RelayStatus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self) -> RelayState {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Returns whether this changed anything, so a caller can avoid announcing
    /// a transition that did not happen - a reconnect that fails four times
    /// should not push four identical events at the window.
    pub fn set(&self, state: RelayState) -> bool {
        let mut current = self.0.write().unwrap_or_else(|e| e.into_inner());
        let changed = *current != state;
        *current = state;
        changed
    }
}

#[cfg(test)]
mod state_tests {
    use super::*;

    #[test]
    fn an_unconfigured_relay_is_off_rather_than_unknown() {
        // Every install today. "Off" is an answer the window can render; the
        // absence of one is a spinner that never resolves.
        assert_eq!(RelayStatus::new().get(), RelayState::Off);
    }

    #[test]
    fn only_a_real_transition_is_announced() {
        // A reconnect that fails four times passes through Connecting four
        // times. Pushing four identical events would make a quiet retry look
        // like a machine thrashing.
        let status = RelayStatus::new();
        assert!(status.set(RelayState::Connecting), "off -> connecting");
        assert!(!status.set(RelayState::Connecting), "no change, no event");
        assert!(status.set(RelayState::Connected), "connecting -> connected");
    }
}

/// Where to dial, and as whom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The edge's websocket URL. `wss://` in anger; `ws://` exists so that a
    /// test and a local dev stack do not have to stand up a certificate to
    /// exercise the handshake.
    pub url: String,
}

impl Config {
    /// Read the relay configuration, if there is any.
    ///
    /// `None` is the normal answer and not an error. It means this install has
    /// no relay, which is true of every install today.
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("KT_RELAY_URL").ok()?;
        let url = url.trim().to_string();
        if url.is_empty() {
            return None;
        }
        Some(Self { url })
    }
}

/// Keep a relay connection up for as long as the daemon runs.
///
/// Returns when there is no point trying again - a revoked install, an
/// unlinked one, a version the edge will not speak - and only then. Every
/// other ending is a wait and another attempt.
///
/// Spawned rather than awaited by the daemon: the relay being down must never
/// be the reason apps stop being served on the local network, which is the
/// whole product for anyone who has not paid for anything.
pub async fn run(
    config: Config,
    identity: Arc<InstallIdentity>,
    router: Arc<axum::Router>,
    status: Arc<RelayStatus>,
    on_change: impl Fn(RelayState) + Send + 'static,
) {
    let mut backoff = Backoff::new();
    // One place the state is written, so the window can never be told
    // something the loop does not believe.
    let announce = |state: RelayState| {
        if status.set(state.clone()) {
            on_change(state);
        }
    };

    announce(RelayState::Connecting);

    loop {
        let ended = match conn::connect(&config.url, &identity).await {
            Ok((socket, accepted)) => {
                // Only now, on a connection that got all the way through the
                // handshake. An edge that accepts TCP and hangs up must not
                // look like success, or the backoff resets into a tight loop.
                backoff.succeeded();
                tracing::info!(
                    version = accepted.version,
                    install = %accepted.install,
                    "relay connected"
                );
                announce(RelayState::Connected);

                let ended = carry(socket, Arc::clone(&router)).await;
                tracing::debug!("the relay connection ended: {ended}");
                ended
            }
            Err(ended) => ended,
        };

        let retry = ended.retry();
        if ended.needs_the_owner() {
            // The states where waiting changes nothing and somebody has to act.
            tracing::warn!("the relay is unavailable and needs attention: {ended}");
        } else {
            tracing::debug!("relay connection ended: {ended}");
        }

        let Some(delay) = backoff.next_delay(retry) else {
            tracing::warn!("not dialling the relay again: {ended}");
            announce(RelayState::NeedsAttention(ended.to_string()));
            return;
        };

        // Between a drop and the next attempt an app is genuinely unreachable,
        // and saying "connected" through it would be the same lie in a smaller
        // window.
        announce(RelayState::Connecting);

        if retry == Retry::Backoff {
            tracing::debug!(seconds = delay.as_secs(), "waiting before dialling again");
        }
        tokio::time::sleep(delay).await;
    }
}

/// Carry streams until the connection ends.
///
/// The daemon is the yamux *client* even though every stream on it is opened by
/// the edge. Which end dials and which end opens streams are separate
/// questions, and the daemon dials because that is the only direction a home
/// router allows.
async fn carry(socket: conn::Socket, router: Arc<axum::Router>) -> Ended {
    carry_with(socket, router, session::Heartbeat::default()).await
}

/// The body of [`carry`], with the heartbeat timings exposed.
///
/// Split out only so a test can run the whole silence timeout in milliseconds
/// rather than a minute. Nothing else should pass anything but the default:
/// the numbers are the protocol's, not this daemon's.
async fn carry_with(
    socket: conn::Socket,
    router: Arc<axum::Router>,
    beat: session::Heartbeat,
) -> Ended {
    let mut connection = yamux::Connection::new(
        wsbytes::WsBytes::new(socket),
        yamux::Config::default(),
        yamux::Mode::Client,
    );

    // How the control stream reports a tunnel that has gone quiet. Without
    // this the heartbeat could notice and nothing would happen: the control
    // stream runs on its own task, so its return value goes nowhere, and this
    // loop would keep politely polling a dead connection for new streams.
    let (ended_tx, mut ended_rx) = tokio::sync::mpsc::channel::<Ended>(1);

    loop {
        tokio::select! {
            // Biased towards the verdict: once the tunnel is known to be dead
            // there is nothing worth accepting a new stream for.
            biased;

            Some(ended) = ended_rx.recv() => return ended,

            inbound = std::future::poll_fn(|cx| connection.poll_next_inbound(cx)) => {
                match inbound {
                    Some(Ok(stream)) => {
                        // One task per stream, so a slow app cannot hold up the
                        // heartbeat or anybody else's page.
                        tokio::spawn(session::dispatch(
                            stream,
                            Arc::clone(&router),
                            beat,
                            ended_tx.clone(),
                        ));
                    }
                    Some(Err(e)) => return Ended::Transport(e.to_string()),
                    None => return Ended::Transport("the relay closed the tunnel".into()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `std::env::set_var` is process-wide, so these run under one lock rather
    /// than racing each other through the same variable.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_relay_url<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("KT_RELAY_URL").ok();

        // SAFETY: the lock above is the only thing that touches this variable
        // in this process, and it is restored before the lock is released.
        unsafe {
            match value {
                Some(v) => std::env::set_var("KT_RELAY_URL", v),
                None => std::env::remove_var("KT_RELAY_URL"),
            }
        }

        let result = body();

        unsafe {
            match previous {
                Some(v) => std::env::set_var("KT_RELAY_URL", v),
                None => std::env::remove_var("KT_RELAY_URL"),
            }
        }
        result
    }

    #[test]
    fn a_daemon_with_no_relay_configured_has_no_relay() {
        // The normal case, and emphatically not an error: this is the free
        // tier, which is the whole product for most people.
        assert_eq!(with_relay_url(None, Config::from_env), None);
    }

    #[test]
    fn an_empty_setting_counts_as_unset() {
        // `KT_RELAY_URL=` in a shell profile or a launchd plist is somebody
        // turning it off, not somebody asking to dial the empty string.
        assert_eq!(with_relay_url(Some(""), Config::from_env), None);
        assert_eq!(with_relay_url(Some("   "), Config::from_env), None);
    }

    #[test]
    fn a_configured_url_is_taken_as_given() {
        let config = with_relay_url(Some("wss://tunnel.kitchentable.cloud"), Config::from_env);
        assert_eq!(
            config,
            Some(Config {
                url: "wss://tunnel.kitchentable.cloud".into()
            })
        );
    }

    #[test]
    fn surrounding_whitespace_does_not_become_part_of_the_url() {
        let config = with_relay_url(Some("  ws://localhost:9000  "), Config::from_env);
        assert_eq!(config.expect("configured").url, "ws://localhost:9000");
    }

    // The handshake, against a stub edge that verifies independently rather
    // than echoing. Over a real socket, because a signature that only ever
    // travels between two functions in one process has not been tested.

    use kt_tunnel_proto::{CloseReason, VersionRange};
    use stub::{Behaviour, Handled};

    fn identity() -> Arc<InstallIdentity> {
        Arc::new(InstallIdentity::generate())
    }

    #[tokio::test]
    async fn a_daemon_proves_which_machine_it_is() {
        let identity = identity();
        let (url, edge) = stub::spawn(Behaviour::Verify, VersionRange::supported()).await;

        let (_socket, accepted) = conn::connect(&url, &identity)
            .await
            .expect("the handshake completes");

        // Both ends agree, and the edge reached that view by verifying a
        // signature rather than by being told.
        assert_eq!(accepted.install, identity.public());
        match edge.await.expect("the stub finishes") {
            Handled::Accepted(seen) => assert_eq!(seen.install, identity.public()),
            other => panic!("the edge did not accept: {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_edge_that_is_not_there_is_worth_retrying() {
        // Nothing is listening on this port. A closed lid and a rebooting
        // router look the same, and none of them says anything is wrong with
        // this daemon.
        let ended = conn::connect("ws://127.0.0.1:1", &identity())
            .await
            .expect_err("dialling nothing fails");

        assert!(matches!(ended, Ended::Transport(_)), "{ended:?}");
        assert_eq!(ended.retry(), Retry::Backoff);
        assert!(!ended.needs_the_owner());
    }

    #[tokio::test]
    async fn a_revoked_install_stops_dialling_and_says_so() {
        // The case the whole reason vocabulary exists for. Retrying forever
        // would be a bill, a noisy log, and a window that never gets round to
        // telling the owner the one thing that would help.
        let (url, edge) = stub::spawn(
            Behaviour::Refuse(CloseReason::InstallRevoked),
            VersionRange::supported(),
        )
        .await;

        let ended = conn::connect(&url, &identity())
            .await
            .expect_err("a refusal is not a connection");

        assert_eq!(ended.retry(), Retry::Never);
        assert!(ended.needs_the_owner(), "the window must say something");
        let _ = edge.await;
    }

    #[tokio::test]
    async fn a_drain_comes_straight_back() {
        // The opposite case, and the reason a refusal is not simply fatal: an
        // edge deploy must look like a blip.
        let (url, edge) = stub::spawn(
            Behaviour::Refuse(CloseReason::Draining),
            VersionRange::supported(),
        )
        .await;

        let ended = conn::connect(&url, &identity())
            .await
            .expect_err("a drain closes the connection");

        assert_eq!(ended.retry(), Retry::Soon);
        assert!(!ended.needs_the_owner());
        let _ = edge.await;
    }

    #[tokio::test]
    async fn an_edge_from_the_future_is_the_owners_problem_to_fix() {
        // No version in common, and the daemon is the side that can be out of
        // date. Retrying cannot help; updating can, so the owner is told.
        let (url, edge) = stub::spawn(Behaviour::Verify, VersionRange::new(900, 999)).await;

        let ended = conn::connect(&url, &identity())
            .await
            .expect_err("no shared version");

        assert!(
            matches!(
                ended,
                Ended::Refused(CloseReason::VersionUnsupported { .. })
            ),
            "{ended:?}"
        );
        assert_eq!(ended.retry(), Retry::Never);
        assert!(ended.needs_the_owner());
        let _ = edge.await;
    }

    #[tokio::test]
    async fn an_edge_that_vanishes_mid_handshake_is_transport() {
        // Accepted the connection and then disappeared, the way a crash does.
        let (url, edge) = stub::spawn(Behaviour::HangUp, VersionRange::supported()).await;

        let ended = conn::connect(&url, &identity())
            .await
            .expect_err("nothing was said");

        assert!(matches!(ended, Ended::Transport(_)), "{ended:?}");
        assert_eq!(ended.retry(), Retry::Backoff);
        let _ = edge.await;
    }

    #[tokio::test]
    async fn a_signature_from_a_different_machine_is_refused() {
        // The property the install key exists for, checked by a verifier that
        // does not share code with the signer. Two identities: one signs, the
        // hello claims the other.
        let signer = InstallIdentity::generate();
        let impostor = InstallIdentity::generate();
        let (url, edge) = stub::spawn(Behaviour::Verify, VersionRange::supported()).await;

        // Reach past connect() to send a hello whose key and signature disagree.
        let (mut socket, _) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("the stub accepts a connection");
        let server: kt_tunnel_proto::ServerHello = {
            use futures_util::StreamExt;
            let msg = socket.next().await.expect("a challenge").expect("readable");
            match msg {
                tokio_tungstenite::tungstenite::Message::Binary(b) => {
                    kt_tunnel_proto::codec::decode(&b)
                        .expect("decodes")
                        .expect("whole")
                }
                other => panic!("unexpected: {other:?}"),
            }
        };

        let mut hello = signer.hello(&server, "0.1.0", "macos").expect("signs");
        hello.install = impostor.public();

        {
            use futures_util::SinkExt;
            let frame = kt_tunnel_proto::codec::encode(&hello).expect("encodes");
            socket
                .send(tokio_tungstenite::tungstenite::Message::Binary(
                    frame.into(),
                ))
                .await
                .expect("sends");
        }

        match edge.await.expect("the stub finishes") {
            Handled::Failed(why) => assert!(why.contains("verify"), "{why}"),
            other => panic!("an impostor got in: {other:?}"),
        }
    }
}
