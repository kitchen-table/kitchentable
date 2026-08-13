//! An edge, for tests: enough of one to answer a handshake and no more.
//!
//! The real edge is a different repository and a different language, so this
//! exists to make the daemon's half provable on its own. It runs on localhost
//! over plain `ws://`, which is why [`super::Config`] accepts that scheme: a
//! test should not have to stand up a certificate authority to find out whether
//! a signature verifies.
//!
//! It is the *verifying* half written independently of the signing half, which
//! is the only way a handshake test says anything. A stub that echoed back
//! whatever it was sent would pass no matter what either side did.

use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use kt_tunnel_proto::{
    codec, Accepted, ClientHello, CloseReason, HelloResponse, Nonce, ServerHello, VersionRange,
};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;

/// What the stub decided about one connection.
#[derive(Debug)]
pub enum Handled {
    Accepted(Accepted),
    /// Carried for the Debug output when a test fails, which is the only place
    /// a refusal from the stub's own side needs to be legible.
    Refused(#[allow(dead_code)] CloseReason),
    /// The daemon never got far enough to be judged.
    Failed(String),
}

/// How the stub should behave, so a test can ask for the case it cares about.
///
/// Clone rather than Copy: a close reason can carry a detail string.
#[derive(Debug, Clone)]
pub enum Behaviour {
    /// Verify the hello properly and accept it if it holds up.
    Verify,
    /// Refuse with a given reason, whatever arrives.
    Refuse(CloseReason),
    /// Accept the connection and then vanish without a word, the way a crashed
    /// process or a dropped network does.
    HangUp,
}

/// Start a stub edge on an ephemeral port. Returns its URL and the outcome of
/// the single connection it will handle.
pub async fn spawn(
    behaviour: Behaviour,
    versions: VersionRange,
) -> (String, tokio::task::JoinHandle<Handled>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port");
    let addr: SocketAddr = listener.local_addr().expect("a bound address");
    let url = format!("ws://{addr}");

    let handle = tokio::spawn(async move { serve_one(listener, behaviour, versions).await });
    (url, handle)
}

async fn serve_one(listener: TcpListener, behaviour: Behaviour, versions: VersionRange) -> Handled {
    let Ok((stream, _)) = listener.accept().await else {
        return Handled::Failed("nothing connected".into());
    };
    let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
        return Handled::Failed("the websocket upgrade failed".into());
    };

    if let Behaviour::HangUp = &behaviour {
        drop(socket);
        return Handled::Failed("hung up on purpose".into());
    }

    let nonce = Nonce::generate();
    let server = ServerHello { versions, nonce };
    let Ok(frame) = codec::encode(&server) else {
        return Handled::Failed("could not encode the challenge".into());
    };
    if socket.send(Message::Binary(frame.into())).await.is_err() {
        return Handled::Failed("could not send the challenge".into());
    }

    if let Behaviour::Refuse(reason) = &behaviour {
        refuse(&mut socket, reason.clone()).await;
        return Handled::Refused(reason.clone());
    }

    // Read the answer and check it, which is the only part of this that is
    // doing real work.
    let Some(Ok(Message::Binary(bytes))) = socket.next().await else {
        return Handled::Failed("no hello arrived".into());
    };
    let hello: ClientHello = match codec::decode(&bytes) {
        Ok(Some(hello)) => hello,
        _ => return Handled::Failed("the hello did not decode".into()),
    };

    match server.accept(&hello) {
        Ok(accepted) => {
            let welcome = HelloResponse::Welcome {
                version: accepted.version,
            };
            if let Ok(frame) = codec::encode(&welcome) {
                let _ = socket.send(Message::Binary(frame.into())).await;
            }
            Handled::Accepted(accepted)
        }
        Err(e) => {
            refuse(&mut socket, CloseReason::AuthenticationFailed).await;
            Handled::Failed(format!("the hello did not verify: {e}"))
        }
    }
}

/// Refuse as a frame, then close.
///
/// The reason goes in a frame rather than in the websocket close frame, whose
/// reason field caps at 123 bytes and would truncate a long detail string
/// without saying so.
async fn refuse(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    reason: CloseReason,
) {
    let refused = HelloResponse::Refused { reason };
    if let Ok(frame) = codec::encode(&refused) {
        let _ = socket.send(Message::Binary(frame.into())).await;
    }
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code: CloseCode::Policy,
            reason: "".into(),
        })))
        .await;
}
