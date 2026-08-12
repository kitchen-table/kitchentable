//! Talking to the daemon over the socket.
//!
//! Blocking and dependency-light on purpose: the CLI makes one request and
//! exits, so an async runtime would be weight for nothing.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

use kt_types::protocol::{JsonRpcVersion, ResponsePayload};
use kt_types::{KtError, Request, Response};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(
        "Kitchen Table does not seem to be running.\n\
         Start the desktop app, or run `cargo run -p kt-daemon`."
    )]
    NotRunning,
    #[error("could not talk to the daemon: {0}")]
    Io(#[from] std::io::Error),
    #[error("the daemon sent something unreadable: {0}")]
    Protocol(#[from] serde_json::Error),
    #[error("{}", .0.message)]
    Daemon(KtError),
    #[error("the daemon closed the connection without replying")]
    NoReply,
}

pub struct Client {
    stream: UnixStream,
    next_id: u64,
}

impl Client {
    pub fn connect(socket: &Path) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(socket).map_err(|e| match e.kind() {
            // Both mean the same thing to a person: nothing is listening.
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
                ClientError::NotRunning
            }
            _ => ClientError::Io(e),
        })?;

        Ok(Self { stream, next_id: 1 })
    }

    /// Send one request and wait for its reply.
    pub fn call(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, ClientError> {
        let id = self.next_id;
        self.next_id += 1;

        let request = Request {
            jsonrpc: JsonRpcVersion,
            id: Some(id),
            method: method.to_string(),
            params,
        };

        let mut line = serde_json::to_vec(&request)?;
        line.push(b'\n');
        self.stream.write_all(&line)?;
        self.stream.flush()?;

        let mut reply = String::new();
        BufReader::new(&self.stream).read_line(&mut reply)?;
        if reply.trim().is_empty() {
            return Err(ClientError::NoReply);
        }

        match serde_json::from_str::<Response>(&reply)?.payload {
            ResponsePayload::Result(value) => Ok(value),
            ResponsePayload::Error(e) => Err(ClientError::Daemon(e)),
        }
    }
}
