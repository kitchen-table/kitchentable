//! The shell's client for the daemon socket.
//!
//! Everything the window shows comes through here. The Rust side stays a dumb
//! proxy: it does not interpret payloads, it moves them
//! (docs/architecture.md section 13).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;

/// A short timeout: this is a local socket, so anything slow is a hung daemon,
/// and the window should say so rather than spin forever.
const TIMEOUT: Duration = Duration::from_secs(5);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, thiserror::Error)]
pub enum SocketError {
    #[error("the daemon is not running")]
    NotRunning,
    #[error("could not talk to the daemon: {0}")]
    Io(#[from] std::io::Error),
    #[error("the daemon sent something unreadable: {0}")]
    Protocol(#[from] serde_json::Error),
    /// A typed error from the daemon, passed through untouched so the UI can
    /// branch on its code.
    #[error("{0}")]
    Daemon(Value),
}

impl SocketError {
    /// The shape the UI receives. Daemon errors keep their own structure;
    /// everything else is reported as a transport failure, which the UI shows
    /// as the disconnected state.
    pub fn to_payload(&self) -> Value {
        match self {
            Self::Daemon(value) => value.clone(),
            other => serde_json::json!({
                "code": "unavailable",
                "message": other.to_string(),
            }),
        }
    }
}

pub fn socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    kt_types::paths::socket_path(&home)
}

/// One request, one connection. A Unix socket connect is microseconds, and a
/// per-call connection means a wedged request can never poison later ones.
pub fn call(socket: &Path, method: &str, params: Option<Value>) -> Result<Value, SocketError> {
    let stream = UnixStream::connect(socket).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
            SocketError::NotRunning
        }
        _ => SocketError::Io(e),
    })?;

    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": NEXT_ID.fetch_add(1, Ordering::Relaxed),
        "method": method,
        "params": params,
    });

    let mut writer = &stream;
    let mut line = serde_json::to_vec(&request)?;
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()?;

    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply)?;

    let value: Value = serde_json::from_str(&reply)?;
    if let Some(error) = value.get("error") {
        return Err(SocketError::Daemon(error.clone()));
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

/// Whether a daemon is listening and speaking a protocol we understand.
///
/// A mismatched major means the app was updated but the old daemon is still
/// running; the supervisor restarts it rather than the UI failing oddly
/// (docs/architecture.md section 15).
pub fn handshake(socket: &Path) -> Handshake {
    match call(socket, "sys.status", None) {
        Ok(status) => {
            let theirs = status
                .get("protocol_version")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32;

            if theirs == kt_types::PROTOCOL_VERSION {
                Handshake::Ready
            } else {
                Handshake::WrongVersion {
                    theirs,
                    ours: kt_types::PROTOCOL_VERSION,
                }
            }
        }
        Err(SocketError::NotRunning) => Handshake::NotRunning,
        Err(e) => Handshake::Unhealthy(e.to_string()),
    }
}

#[derive(Debug, PartialEq)]
pub enum Handshake {
    Ready,
    NotRunning,
    WrongVersion { theirs: u32, ours: u32 },
    Unhealthy(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_socket_reads_as_not_running_not_as_a_crash() {
        let missing = Path::new("/nonexistent/kt.sock");
        assert!(matches!(
            call(missing, "sys.status", None),
            Err(SocketError::NotRunning)
        ));
        assert_eq!(handshake(missing), Handshake::NotRunning);
    }

    #[test]
    fn transport_failures_and_daemon_errors_look_different_to_the_ui() {
        let transport = SocketError::NotRunning.to_payload();
        assert_eq!(transport["code"], "unavailable");

        let from_daemon = SocketError::Daemon(serde_json::json!({
            "code": "not_found",
            "message": "no app with slug \"nope\""
        }));
        assert_eq!(from_daemon.to_payload()["code"], "not_found");
    }
}
