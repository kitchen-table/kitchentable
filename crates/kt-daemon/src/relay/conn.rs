//! One connection: dial the edge, prove which machine this is, find out why it
//! ended.
//!
//! The handshake happens on the raw websocket, before any multiplexing, so an
//! unauthenticated peer never reaches a layer that can open streams. Stream
//! carrying comes next and builds on this.

use futures_util::{SinkExt, StreamExt};
use kt_tunnel_proto::{
    codec, Accepted, CloseReason, HelloResponse, InstallIdentity, Retry, ServerHello,
};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// The websocket to the edge, once it is up.
pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Why a connection attempt or a connection ended.
///
/// Everything that can go wrong lands here, so that exactly one place decides
/// what to do next - and so that "the edge refused us for good" and "the wifi
/// dropped" cannot be confused for one another, which is the distinction the
/// whole reconnect policy turns on.
#[derive(Debug)]
pub enum Ended {
    /// The edge said why, in the protocol's own words.
    Refused(CloseReason),
    /// Never got a usable connection, or lost one mid-flight. A closed lid, a
    /// rebooting router, an edge that is not there. Always worth retrying:
    /// nothing about it says anything is wrong with this daemon.
    Transport(String),
    /// The other end is not speaking this protocol properly. A bug somewhere,
    /// so it backs off rather than hammering, and says enough to be diagnosed.
    Protocol(String),
}

impl Ended {
    pub fn retry(&self) -> Retry {
        match self {
            Self::Refused(reason) => reason.retry(),
            Self::Transport(_) => Retry::Backoff,
            Self::Protocol(_) => Retry::Backoff,
        }
    }

    /// Whether the desktop window should say something rather than spin.
    pub fn needs_the_owner(&self) -> bool {
        matches!(self, Self::Refused(reason) if reason.needs_the_owner())
    }
}

impl std::fmt::Display for Ended {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(reason) => write!(f, "the relay refused this install: {reason:?}"),
            Self::Transport(detail) => write!(f, "the connection failed: {detail}"),
            Self::Protocol(detail) => write!(f, "the relay is not making sense: {detail}"),
        }
    }
}

/// Dial the edge and complete the handshake.
///
/// Returns the live socket and what the edge decided, or why it did not get
/// that far.
pub async fn connect(url: &str, identity: &InstallIdentity) -> Result<(Socket, Accepted), Ended> {
    let (mut socket, _) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|e| Ended::Transport(e.to_string()))?;

    let accepted = handshake(&mut socket, identity).await?;
    Ok((socket, accepted))
}

/// Prove which machine this is.
///
/// Three messages. The edge challenges, this answers it signed, and the edge
/// says whether that was good enough. The third one is not a formality: see
/// [`HelloResponse`] for what assuming acceptance costs.
pub async fn handshake(socket: &mut Socket, identity: &InstallIdentity) -> Result<Accepted, Ended> {
    let server: ServerHello = read_frame(socket).await?;

    let supported = server.versions;
    let hello = identity
        .hello(&server, env!("CARGO_PKG_VERSION"), std::env::consts::OS)
        // The only way this fails is no version in common. That is a real
        // state rather than a bug, and one the owner can act on by updating,
        // so it becomes the refusal that says exactly that.
        .map_err(|_| Ended::Refused(CloseReason::VersionUnsupported { supported }))?;

    let frame = codec::encode(&hello).map_err(|e| Ended::Protocol(e.to_string()))?;
    socket
        .send(Message::Binary(frame.into()))
        .await
        .map_err(|e| Ended::Transport(e.to_string()))?;

    // Wait to be let in rather than assuming it. Assuming it means a refused
    // daemon believes it is connected, resets its reconnect backoff on the way
    // past, and dials again immediately - forever.
    match read_frame::<HelloResponse>(socket).await? {
        HelloResponse::Welcome { version } => Ok(Accepted {
            version,
            install: hello.install,
        }),
        HelloResponse::Refused { reason } => Err(Ended::Refused(reason)),
    }
}

/// Read one length-prefixed frame from the socket.
///
/// Websocket messages carry their own length, so a frame arrives whole rather
/// than needing to be reassembled - but the length prefix is still read and
/// respected, because it is what the protocol says is there and because the
/// same bytes have to make sense over a stream that has no message boundaries.
async fn read_frame<T: serde::de::DeserializeOwned>(socket: &mut Socket) -> Result<T, Ended> {
    loop {
        let message = socket
            .next()
            .await
            .ok_or_else(|| Ended::Transport("the relay hung up".into()))?
            .map_err(|e| Ended::Transport(e.to_string()))?;

        return match message {
            Message::Binary(bytes) => codec::decode(&bytes)
                .map_err(|e| Ended::Protocol(e.to_string()))?
                .ok_or_else(|| Ended::Protocol("a frame arrived incomplete".into())),
            Message::Close(frame) => Err(closed(frame.map(|f| f.reason.to_string()))),
            // Tungstenite answers pings itself; both are noise at this layer.
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Text(_) => Err(Ended::Protocol("expected a binary frame".into())),
            Message::Frame(_) => continue,
        };
    }
}

/// A websocket close, turned into something the retry policy can read.
fn closed(reason: Option<String>) -> Ended {
    // The edge is expected to say goodbye in the protocol's own words before
    // closing. A bare close with no reason is not an error - a lid closing
    // produces one - it just carries no information, so it is treated as
    // transport rather than as a refusal.
    match reason.as_deref() {
        Some(text) if !text.is_empty() => match serde_json::from_str::<CloseReason>(text) {
            Ok(reason) => Ended::Refused(reason),
            Err(_) => Ended::Transport(format!("the relay closed: {text}")),
        },
        _ => Ended::Transport("the relay closed the connection".into()),
    }
}
