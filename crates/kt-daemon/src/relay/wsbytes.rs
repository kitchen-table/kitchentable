//! A websocket, presented as a stream of bytes.
//!
//! yamux multiplexes over anything that reads and writes bytes. A websocket
//! does not: it carries discrete messages. This is the ~100 lines in between,
//! and the reason it is written by hand rather than taken off the shelf is that
//! the published adapter needs a websocket stack two major versions behind the
//! one this daemon uses, which would mean two of them in the binary.
//!
//! # Where this kind of code goes wrong
//!
//! Adapters like this fail quietly rather than loudly, so the failures are
//! listed here and each has a test:
//!
//! - **Losing the tail of a message.** A websocket message can be larger than
//!   the buffer the caller offers. Whatever does not fit has to be kept for the
//!   next read, and forgetting it drops bytes out of the middle of a stream
//!   without any error anywhere.
//! - **Returning `Ok(0)` when there is more to come.** To every reader in Rust
//!   that means end of stream. Returned early it truncates the connection and
//!   looks like the peer hung up.
//! - **Sending without asking the sink first.** A `Sink` must be polled ready
//!   before it is given anything, and skipping that either panics or silently
//!   drops what was sent.
//! - **Returning `Pending` with nobody registered to wake it.** The task parks
//!   forever and the tunnel simply stops, with no error and no log line.
//!
//! Every one of those is invisible to a test that pushes a small payload
//! through in one go, which is why the test at the bottom pushes megabytes
//! across several concurrent yamux streams with mismatched buffer sizes and
//! checks every byte.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::{Sink, Stream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

/// Wraps a websocket so yamux can treat it as a socket.
///
/// Generic over what the websocket runs on, so the same adapter serves the
/// daemon's TLS connection and both ends of a plain loopback pair in a test.
/// An adapter tested only against the type it ships with is tested against
/// itself.
pub struct WsBytes<S> {
    socket: WebSocketStream<S>,
    /// The part of the last message that did not fit in the caller's buffer.
    ///
    /// `Bytes` rather than a copy: the remainder is a view into the message
    /// that already arrived, so carrying it costs a pointer rather than the
    /// bytes themselves.
    unread: Option<Bytes>,
    /// Set once the peer has closed, so later reads keep reporting end of
    /// stream rather than polling a finished websocket.
    finished: bool,
}

impl<S> WsBytes<S> {
    pub fn new(socket: WebSocketStream<S>) -> Self {
        Self {
            socket,
            unread: None,
            finished: false,
        }
    }

    /// Take what fits, keep the rest.
    fn drain_into(&mut self, buf: &mut [u8]) -> usize {
        let Some(pending) = self.unread.as_mut() else {
            return 0;
        };

        let taken = pending.len().min(buf.len());
        buf[..taken].copy_from_slice(&pending[..taken]);

        if taken == pending.len() {
            self.unread = None;
        } else {
            // advance() is a slice, not a copy: the tail stays where it is.
            let _ = pending.split_to(taken);
        }
        taken
    }
}

impl<S> futures_util::io::AsyncRead for WsBytes<S>
where
    WebSocketStream<S>:
        Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        // A zero-length read is a legitimate no-op and must not be confused
        // with end of stream further down.
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }

        // Anything left over from last time comes first, or the stream would
        // arrive out of order.
        let taken = self.drain_into(buf);
        if taken > 0 {
            return Poll::Ready(Ok(taken));
        }

        if self.finished {
            return Poll::Ready(Ok(0));
        }

        loop {
            let message = match Pin::new(&mut self.socket).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                // The websocket ended. Ok(0) here is correct and is the only
                // place it is.
                Poll::Ready(None) => {
                    self.finished = true;
                    return Poll::Ready(Ok(0));
                }
                Poll::Ready(Some(Ok(message))) => message,
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Err(io::Error::other(e.to_string())))
                }
            };

            match message {
                Message::Binary(bytes) => {
                    // An empty message carries nothing. Returning Ok(0) for it
                    // would tell the reader the connection had ended.
                    if bytes.is_empty() {
                        continue;
                    }
                    self.unread = Some(bytes);
                    let taken = self.drain_into(buf);
                    return Poll::Ready(Ok(taken));
                }
                Message::Close(_) => {
                    self.finished = true;
                    return Poll::Ready(Ok(0));
                }
                // Tungstenite answers pings itself. Text has no business on a
                // byte stream, and treating it as data would corrupt the mux.
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                Message::Text(_) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "a text message arrived on the tunnel",
                    )))
                }
            }
        }
    }
}

impl<S> futures_util::io::AsyncWrite for WsBytes<S>
where
    WebSocketStream<S>: Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }

        // Ask before sending. A Sink that is not ready will drop or panic on
        // what it is given, and this is the step it is easiest to skip.
        match Pin::new(&mut self.socket).poll_ready(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(io::Error::other(e.to_string()))),
            Poll::Ready(Ok(())) => {}
        }

        // One message per write. yamux writes a header and then a body, so this
        // is a small number of messages per frame rather than one per byte; if
        // that ever shows up in a profile, a write buffer flushed on poll_flush
        // is the fix, and it is not worth the extra state until then.
        let message = Message::Binary(Bytes::copy_from_slice(buf));
        match Pin::new(&mut self.socket).start_send(message) {
            Ok(()) => Poll::Ready(Ok(buf.len())),
            Err(e) => Poll::Ready(Err(io::Error::other(e.to_string()))),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket)
            .poll_flush(cx)
            .map_err(|e| io::Error::other(e.to_string()))
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket)
            .poll_close(cx)
            .map_err(|e| io::Error::other(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use yamux::{Config, Connection, Mode};

    /// Deterministic filler, so a mismatch says where it went wrong rather than
    /// just that it did. Every byte depends on its position, which a test using
    /// a repeated pattern would let slide.
    fn payload(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    /// A websocket pair on loopback, both ends wrapped.
    async fn pair() -> (WsBytes<MaybeTls>, WsBytes<TcpStream>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let addr = listener.local_addr().expect("bound");

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accepted");
            tokio_tungstenite::accept_async(stream)
                .await
                .expect("upgraded")
        });

        let (client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .expect("connected");

        (
            WsBytes::new(client),
            WsBytes::new(server.await.expect("joined")),
        )
    }

    type MaybeTls = tokio_tungstenite::MaybeTlsStream<TcpStream>;

    /// Drive a yamux connection, echoing every inbound stream back byte for
    /// byte. This is the far end of the tunnel, standing in for the edge.
    async fn echo_server(socket: WsBytes<TcpStream>) {
        let mut connection = Connection::new(socket, Config::default(), Mode::Server);
        while let Some(stream) = std::future::poll_fn(|cx| connection.poll_next_inbound(cx))
            .await
            .transpose()
            .expect("an inbound stream")
        {
            tokio::spawn(async move {
                let mut stream = stream;
                let mut buf = vec![0u8; 4096];
                loop {
                    match stream.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if stream.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                    }
                }
                let _ = stream.close().await;
            });
        }
    }

    #[tokio::test]
    async fn a_payload_larger_than_any_read_buffer_arrives_whole() {
        // The failure this adapter is most likely to have: a websocket message
        // bigger than the buffer offered, whose tail is quietly dropped. Two
        // megabytes through 4 KiB reads means thousands of chances to lose it,
        // and the check is byte for byte rather than a length.
        let (client, server) = pair().await;
        tokio::spawn(echo_server(server));

        let mut connection = Connection::new(client, Config::default(), Mode::Client);
        let stream = std::future::poll_fn(|cx| connection.poll_new_outbound(cx))
            .await
            .expect("a stream");

        // The connection has to be driven while the stream is used.
        tokio::spawn(async move {
            let _ = std::future::poll_fn(|cx| connection.poll_next_inbound(cx)).await;
        });

        let sent = payload(2 * 1024 * 1024);

        // Write and read concurrently rather than one after the other.
        // Writing two megabytes and only then reading would deadlock the
        // moment the echo server's receive window filled - which is exactly
        // the flow control this whole adapter exists to preserve, so the test
        // has to respect it.
        let (mut reader, mut writer) = AsyncReadExt::split(stream);
        let expected = sent.clone();
        let checker = tokio::spawn(async move {
            let mut received = vec![0u8; expected.len()];
            reader
                .read_exact(&mut received)
                .await
                .expect("reads it back");
            received
        });

        writer.write_all(&sent).await.expect("writes");
        writer.flush().await.expect("flushes");

        let received = checker.await.expect("the reader did not panic");
        assert_eq!(received.len(), sent.len());
        // Compare positionally so a failure names the offset.
        if let Some(i) = (0..sent.len()).find(|&i| sent[i] != received[i]) {
            panic!("first difference at byte {i} of {}", sent.len());
        }
    }

    #[tokio::test]
    async fn several_streams_at_once_do_not_bleed_into_each_other() {
        // What the multiplexing is for, and the failure a single-stream test
        // cannot see: bytes from one stream surfacing on another. Each stream
        // carries a payload only it could have produced.
        let (client, server) = pair().await;
        tokio::spawn(echo_server(server));

        let mut connection = Connection::new(client, Config::default(), Mode::Client);

        let mut streams = Vec::new();
        for _ in 0..8 {
            streams.push(
                std::future::poll_fn(|cx| connection.poll_new_outbound(cx))
                    .await
                    .expect("a stream"),
            );
        }

        tokio::spawn(async move {
            let _ = std::future::poll_fn(|cx| connection.poll_next_inbound(cx)).await;
        });

        let mut work = Vec::new();
        for (n, mut stream) in streams.into_iter().enumerate() {
            work.push(tokio::spawn(async move {
                // Distinct length and content per stream.
                let sent: Vec<u8> = payload(64 * 1024 + n * 1024)
                    .into_iter()
                    .map(|b| b.wrapping_add(n as u8))
                    .collect();

                stream.write_all(&sent).await.expect("writes");
                stream.flush().await.expect("flushes");

                let mut received = vec![0u8; sent.len()];
                stream.read_exact(&mut received).await.expect("reads back");
                assert_eq!(received, sent, "stream {n} came back wrong");
            }));
        }

        for task in work {
            task.await.expect("no stream panicked");
        }
    }

    #[tokio::test]
    async fn a_cleanly_closed_websocket_reads_as_end_of_stream_and_stays_that_way() {
        // Ok(0) means end of stream to every reader in Rust, so it must be
        // returned exactly when the websocket has really finished - and must
        // keep being returned rather than polling a socket that is done.
        let (mut client, mut server) = pair().await;
        server.close().await.expect("closes cleanly");

        let mut buf = [0u8; 64];
        assert_eq!(client.read(&mut buf).await.expect("reads"), 0);
        assert_eq!(client.read(&mut buf).await.expect("reads again"), 0);
    }

    #[tokio::test]
    async fn a_peer_that_vanishes_is_an_error_rather_than_a_quiet_ending() {
        // Dropping without a closing handshake is what a crash, a closed lid
        // or a dropped network looks like on the wire. Reporting it as end of
        // stream would make it indistinguishable from a peer that finished
        // normally - and the difference matters, because one of them is worth
        // reconnecting after and saying so in a log, and the other is not.
        let (mut client, server) = pair().await;
        drop(server);

        let mut buf = [0u8; 64];
        assert!(
            client.read(&mut buf).await.is_err(),
            "a reset was reported as a clean end of stream"
        );
    }

    #[tokio::test]
    async fn a_zero_length_read_is_not_end_of_stream() {
        let (mut client, _server) = pair().await;
        let mut empty: [u8; 0] = [];
        assert_eq!(client.read(&mut empty).await.expect("reads"), 0);
    }
}
