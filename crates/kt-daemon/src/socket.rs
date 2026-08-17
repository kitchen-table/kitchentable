//! The Unix domain socket: newline-delimited JSON-RPC.
//!
//! Mode 0600 is the entire authorisation model. Only the logged-in user can
//! open the socket, so there is no token to steal and no admin surface exposed
//! on any network interface (docs/architecture.md section 4).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kt_types::{protocol::JsonRpcVersion, ErrorCode, KtError, Request, Response};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::rpc::{self, Context};

#[derive(Debug, thiserror::Error)]
pub enum SocketError {
    #[error("another daemon already owns {path}")]
    AlreadyRunning { path: String },
    #[error("could not prepare {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Bind the socket, removing a stale one left by a crash.
///
/// "Stale" means nothing answers on it. A socket file that still has a listener
/// belongs to a running daemon and is left alone, which is what keeps two
/// daemons from serving one workspace.
pub async fn bind(path: &Path) -> Result<UnixListener, SocketError> {
    let io = |source| SocketError::Io {
        path: path.display().to_string(),
        source,
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }

    if path.exists() {
        if UnixStream::connect(path).await.is_ok() {
            return Err(SocketError::AlreadyRunning {
                path: path.display().to_string(),
            });
        }
        tracing::warn!(path = %path.display(), "removing a stale socket");
        std::fs::remove_file(path).map_err(io)?;
    }

    let listener = UnixListener::bind(path).map_err(io)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(io)?;

    Ok(listener)
}

/// Accept connections until cancelled. One task per client; a client that
/// misbehaves affects only its own connection.
pub async fn serve(listener: UnixListener, ctx: Arc<Context>) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let ctx = Arc::clone(&ctx);
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, ctx).await {
                        tracing::debug!(error = %e, "client disconnected");
                    }
                });
            }
            Err(e) => {
                tracing::warn!(error = %e, "accept failed");
                return;
            }
        }
    }
}

/// The method that turns a connection into a subscriber.
///
/// Opt-in rather than automatic: the CLI makes one call and leaves, and pushing
/// events at it would put unasked-for lines between its request and its answer.
const SUBSCRIBE: &str = "event.subscribe";

async fn handle_client(stream: UnixStream, ctx: Arc<Context>) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let mut events: Option<tokio::sync::broadcast::Receiver<kt_types::Event>> = None;

    loop {
        tokio::select! {
            // Biased so a request is always handled before a queued event.
            // Answers are what a caller is blocking on; events can wait a turn.
            biased;

            line = lines.next_line() => {
                let Some(line) = line? else { return Ok(()) };
                if line.trim().is_empty() {
                    continue;
                }

                let response = match serde_json::from_str::<Request>(&line) {
                    Ok(request) => {
                        if request.method == SUBSCRIBE && events.is_none() {
                            events = Some(ctx.events.subscribe());
                        }
                        // A notification expects no reply.
                        if request.id.is_none() {
                            rpc::dispatch(&ctx, &request);
                            continue;
                        }
                        rpc::respond(&ctx, &request)
                    }
                    Err(e) => parse_error(&e),
                };

                let mut bytes = serde_json::to_vec(&response).unwrap_or_else(|_| {
                    br#"{"jsonrpc":"2.0","id":null,"error":{"code":"internal","message":"unserialisable response"}}"#.to_vec()
                });
                bytes.push(b'\n');
                write.write_all(&bytes).await?;
            }

            event = next_event(&mut events) => {
                let mut bytes = match serde_json::to_vec(&notification(event)) {
                    Ok(bytes) => bytes,
                    // Skip rather than drop the connection: a client that cannot
                    // be told about one app is still owed the rest of them.
                    Err(e) => {
                        tracing::warn!(error = %e, "could not serialise an event");
                        continue;
                    }
                };
                bytes.push(b'\n');
                write.write_all(&bytes).await?;
            }
        }
    }
}

/// The next event for a subscriber, or never for a connection that has not
/// asked for any.
///
/// Lagging is survivable and is not a disconnection: the client has missed
/// some events, so it is told to resynchronise rather than left believing its
/// picture is current.
async fn next_event(
    events: &mut Option<tokio::sync::broadcast::Receiver<kt_types::Event>>,
) -> kt_types::Event {
    use tokio::sync::broadcast::error::RecvError;

    let Some(rx) = events.as_mut() else {
        return std::future::pending().await;
    };

    match rx.recv().await {
        Ok(event) => event,
        Err(RecvError::Lagged(missed)) => {
            tracing::warn!(missed, "a subscriber fell behind");
            kt_types::Event::Error {
                message: format!("{missed} events were missed; the window may be out of date"),
            }
        }
        // The daemon is shutting down; nothing more will arrive.
        Err(RecvError::Closed) => std::future::pending().await,
    }
}

/// An event as a JSON-RPC notification: no id, so no reply is expected.
fn notification(event: kt_types::Event) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "event",
        "params": event,
    })
}

fn parse_error(e: &serde_json::Error) -> Response {
    Response {
        jsonrpc: JsonRpcVersion,
        id: None,
        payload: kt_types::protocol::ResponsePayload::Error(KtError {
            code: ErrorCode::BadRequest,
            message: format!("could not parse the request: {e}"),
            detail: None,
        }),
    }
}

/// Best-effort cleanup so the next start does not have to treat the socket as
/// stale.
pub fn cleanup(path: &PathBuf) {
    if let Err(e) = std::fs::remove_file(path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(path = %path.display(), error = %e, "could not remove socket");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kt_types::ServingState;
    use std::time::Instant;

    fn temp_socket(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEQ: AtomicU32 = AtomicU32::new(0);

        std::env::temp_dir().join(format!(
            "kt-sock-{}-{}-{name}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn context() -> Arc<Context> {
        Arc::new(Context {
            library: Arc::new(crate::library::Library::new()),
            store: Arc::new(kt_store::Store::in_memory().expect("opens")),
            workspace: "/tmp/ws".into(),
            workspace_locked: false,
            state_dir: std::path::PathBuf::from("/tmp/kt-state-not-used"),
            urls: kt_types::Urls {
                scheme: "http".into(),
                hostname: None,
                port_suffix: ":8420".into(),
                prefix_origin: "http://localhost:8420".into(),
                fallback_origin: "http://127.0.0.1:8420".into(),
                public_host: None,
            },
            serving: ServingState::Serving,
            started: Instant::now(),
            // Nothing to rescan: these tests drive the library directly.
            events: crate::rpc::Events::new(),
            presence: std::sync::Arc::new(kt_server::Presence::new()),
            install_key: None,
            relay: Arc::new(crate::relay::RelayStatus::new()),
            stores: Arc::new(kt_store::Storage::new(
                std::env::temp_dir().join("kt-socket-stores"),
            )),
            rescan: std::sync::Arc::new(|| {}),
        })
    }

    async fn round_trip(path: &Path, line: &str) -> String {
        let stream = UnixStream::connect(path).await.expect("connects");
        let (read, mut write) = stream.into_split();
        write.write_all(line.as_bytes()).await.expect("writes");
        write.write_all(b"\n").await.expect("writes newline");

        let mut lines = BufReader::new(read).lines();
        lines.next_line().await.expect("reads").expect("a line")
    }

    #[tokio::test]
    async fn the_socket_is_only_readable_by_its_owner() {
        let path = temp_socket("perms");
        let listener = bind(&path).await.expect("binds");

        let mode = std::fs::metadata(&path)
            .expect("stats")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "socket must not be group or world accessible"
        );

        drop(listener);
        cleanup(&path);
    }

    #[tokio::test]
    async fn a_stale_socket_is_replaced() {
        let path = temp_socket("stale");
        std::fs::create_dir_all(path.parent().expect("has parent")).expect("creates");
        std::fs::write(&path, b"not really a socket").expect("writes");

        let listener = bind(&path).await.expect("replaces the stale file");
        drop(listener);
        cleanup(&path);
    }

    #[tokio::test]
    async fn a_live_socket_is_refused_rather_than_stolen() {
        let path = temp_socket("live");
        let first = bind(&path).await.expect("binds");
        tokio::spawn(serve(first, context()));

        // Give the accept loop a moment to be listening.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert!(matches!(
            bind(&path).await,
            Err(SocketError::AlreadyRunning { .. })
        ));
        cleanup(&path);
    }

    #[tokio::test]
    async fn answers_sys_status() {
        let path = temp_socket("status");
        let listener = bind(&path).await.expect("binds");
        tokio::spawn(serve(listener, context()));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let reply = round_trip(&path, r#"{"jsonrpc":"2.0","id":1,"method":"sys.status"}"#).await;
        let value: serde_json::Value = serde_json::from_str(&reply).expect("parses");

        assert_eq!(value["id"], 1);
        assert_eq!(
            value["result"]["protocol_version"],
            kt_types::PROTOCOL_VERSION
        );
        cleanup(&path);
    }

    #[tokio::test]
    async fn malformed_json_gets_an_error_not_a_dropped_connection() {
        let path = temp_socket("malformed");
        let listener = bind(&path).await.expect("binds");
        tokio::spawn(serve(listener, context()));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let reply = round_trip(&path, "{ not json").await;
        let value: serde_json::Value = serde_json::from_str(&reply).expect("parses");
        assert_eq!(value["error"]["code"], "bad_request");
        cleanup(&path);
    }

    #[tokio::test]
    async fn several_requests_share_one_connection() {
        let path = temp_socket("pipeline");
        let listener = bind(&path).await.expect("binds");
        tokio::spawn(serve(listener, context()));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let stream = UnixStream::connect(&path).await.expect("connects");
        let (read, mut write) = stream.into_split();
        write
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"sys.status\"}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"app.list\"}\n",
            )
            .await
            .expect("writes");

        let mut lines = BufReader::new(read).lines();
        let first: serde_json::Value =
            serde_json::from_str(&lines.next_line().await.expect("reads").expect("line"))
                .expect("parses");
        let second: serde_json::Value =
            serde_json::from_str(&lines.next_line().await.expect("reads").expect("line"))
                .expect("parses");

        assert_eq!(first["id"], 1);
        assert_eq!(second["id"], 2);
        cleanup(&path);
    }
}
