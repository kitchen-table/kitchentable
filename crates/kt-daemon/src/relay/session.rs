//! What happens on a connection once it is up: streams, and the requests on
//! them.
//!
//! The edge opens a stream per viewer request and one long-lived control
//! stream. Each opens with a length-prefixed [`StreamOpen`] saying which it is,
//! and after that a request stream is the request body up and the response
//! down.
//!
//! # A relayed request goes through the same gate as any other
//!
//! It is served by calling the daemon's own router - the same one the LAN
//! listener uses - rather than by anything that reimplements serving. There is
//! one place a file leaves this machine and this is not a second one.
//!
//! The one thing added on the way in is [`kt_server::ArrivedByRelay`], and it
//! only ever takes trust away: it is what stops a request that arrived from the
//! internet being handed a Household app because *this laptop* happens to be at
//! home. See `Peer::context` in kt-server for the whole of that argument.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use futures_util::io::{AsyncReadExt, AsyncWriteExt};
use kt_tunnel_proto::{
    codec, ControlFrame, HttpRequestHead, HttpResponseHead, StreamOpen, HEARTBEAT_INTERVAL_SECS,
    HEARTBEAT_TIMEOUT_SECS, MAX_FRAME_LEN,
};
use tower::ServiceExt;

use super::Ended;

/// How often to say "still here", and how long silence lasts before the edge
/// is presumed gone.
///
/// A struct rather than the constants used directly, so a test can run the
/// whole timeout in milliseconds instead of a minute - the same shape the edge
/// uses, for the same reason. Not a configuration knob: both ends agreed these
/// numbers in `kt-tunnel-proto`, and a daemon that let them be tuned locally
/// could declare a healthy edge dead on a number the edge never heard about.
#[derive(Debug, Clone, Copy)]
pub struct Heartbeat {
    pub interval: Duration,
    pub timeout: Duration,
}

impl Default for Heartbeat {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(HEARTBEAT_INTERVAL_SECS as u64),
            timeout: Duration::from_secs(HEARTBEAT_TIMEOUT_SECS as u64),
        }
    }
}

/// The largest request body accepted over the relay.
///
/// Bodies arrive from strangers over the internet, so the first thing done with
/// one is refusing to hold an unbounded amount of it in memory. Apps here are
/// static files plus a small storage API; anything sending more than this is
/// not doing something the product supports.
const MAX_BODY: usize = 8 * 1024 * 1024;

/// Read the [`StreamOpen`] that begins a stream, and whatever arrived with it.
///
/// Returns the opener plus any body bytes that shared the read, because a
/// length-prefixed header and the start of a body can land together and
/// throwing the tail away would corrupt the request.
async fn read_open(stream: &mut yamux::Stream) -> std::io::Result<(StreamOpen, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];

    loop {
        match codec::try_decode::<StreamOpen>(&buf) {
            Ok(Some((open, consumed))) => return Ok((open, buf[consumed..].to_vec())),
            Ok(None) => {}
            Err(e) => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        }

        if buf.len() > MAX_FRAME_LEN as usize {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the stream opener never ended",
            ));
        }

        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "the stream ended before it said what it was for",
            ));
        }
        buf.extend_from_slice(&chunk[..read]);
    }
}

/// Keep the connection honest in both directions, and say why it ended.
///
/// **The heartbeat is symmetric and this end has to hold up its half.**
/// `kt-tunnel-proto` says each side pings on the interval and presumes the
/// other gone after the timeout. The edge does both. For a long time this did
/// only the answering half, and the consequence was the failure the protocol's
/// own header describes: a NAT table drops an idle entry, the socket stays open
/// with nothing arriving on it, and a daemon that never asks anything never
/// finds out. It sat holding a dead tunnel, certain it was connected, while
/// every published app on the machine served 502 from an edge that had long
/// since written it off. Nothing in the logs, nothing in the window.
///
/// So: ping on the interval, and treat silence past the timeout as gone.
/// Returning is what starts the redial - see `carry`, which is listening.
pub async fn control(
    mut stream: yamux::Stream,
    arrived_with_opener: Vec<u8>,
    beat: Heartbeat,
) -> Ended {
    // Whatever shared a read with the opener comes first. Dropping it loses
    // real frames: the edge sends its opener and its first ping together, so
    // they routinely arrive in one read, and a control stream that threw the
    // tail away would sit waiting for a ping it had already been sent - and
    // the edge, hearing nothing, would decide this daemon was asleep and start
    // serving snapshots in its place.
    let mut buf = arrived_with_opener;
    let mut chunk = [0u8; 1024];

    let mut seq: u32 = 1;
    let mut last_heard = Instant::now();
    let mut ticker = tokio::time::interval(beat.interval);
    // The first tick fires immediately, and a ping in the same breath as the
    // handshake tells nobody anything.
    ticker.tick().await;

    loop {
        // Everything already buffered is dealt with before waiting for more,
        // or a read that delivered two frames would leave one unanswered.
        match codec::try_decode::<ControlFrame>(&buf) {
            Ok(Some((frame, consumed))) => {
                buf.drain(..consumed);
                // Any frame at all proves the edge is there. Pongs are the
                // point, but a ping does the job just as well.
                last_heard = Instant::now();
                match frame {
                    ControlFrame::Ping { seq } => {
                        let pong = ControlFrame::Pong { seq };
                        let Ok(bytes) = codec::encode(&pong) else {
                            return Ended::Protocol("could not encode a pong".into());
                        };
                        if let Err(e) = stream.write_all(&bytes).await {
                            return Ended::Transport(e.to_string());
                        }
                        if let Err(e) = stream.flush().await {
                            return Ended::Transport(e.to_string());
                        }
                    }
                    ControlFrame::Goodbye { reason } => {
                        tracing::debug!("the relay said goodbye: {reason:?}");
                        return Ended::Refused(reason);
                    }
                    // Pong and anything a newer edge invents. Both are
                    // deliberately survivable: dropping a connection because
                    // the other end learned a new word would turn every
                    // forward-compatible change into an outage.
                    ControlFrame::Pong { .. } | ControlFrame::Unknown => {}
                }
                continue;
            }
            Ok(None) => {}
            Err(e) => return Ended::Protocol(format!("a control frame did not parse: {e}")),
        }

        if buf.len() > MAX_FRAME_LEN as usize {
            return Ended::Protocol("a control frame never ended".into());
        }

        tokio::select! {
            // Cancel-safe: poll_read copies nothing until it is ready, so
            // losing this race loses no bytes.
            read = stream.read(&mut chunk) => match read {
                Ok(0) => return Ended::Transport("the relay closed the control stream".into()),
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(e) => return Ended::Transport(e.to_string()),
            },

            _ = ticker.tick() => {
                // Checked on the tick rather than on its own timer, so there is
                // one clock here instead of two that can disagree.
                if last_heard.elapsed() > beat.timeout {
                    return Ended::Transport(
                        "the relay stopped answering; presuming the tunnel is dead".into(),
                    );
                }
                let Ok(bytes) = codec::encode(&ControlFrame::Ping { seq }) else {
                    return Ended::Protocol("could not encode a ping".into());
                };
                seq = seq.wrapping_add(1);
                if let Err(e) = stream.write_all(&bytes).await {
                    return Ended::Transport(e.to_string());
                }
                if let Err(e) = stream.flush().await {
                    return Ended::Transport(e.to_string());
                }
            }
        }
    }
}

/// Serve one relayed request and write the answer back down its stream.
pub async fn request(
    mut stream: yamux::Stream,
    head: HttpRequestHead,
    body_so_far: Vec<u8>,
    router: axum::Router,
) {
    let response = match serve(&head, body_so_far, &mut stream, router).await {
        Ok(response) => response,
        Err(e) => {
            tracing::debug!(
                hostname = head.hostname,
                "could not serve a relayed request: {e}"
            );
            return;
        }
    };

    if let Err(e) = write_response(&mut stream, response).await {
        tracing::debug!("could not answer a relayed request: {e}");
    }
    let _ = stream.close().await;
}

async fn serve(
    head: &HttpRequestHead,
    body_so_far: Vec<u8>,
    stream: &mut yamux::Stream,
    router: axum::Router,
) -> std::io::Result<axum::response::Response> {
    let mut body = body_so_far;
    let mut chunk = [0u8; 8192];
    loop {
        if body.len() > MAX_BODY {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the request body is larger than this daemon accepts",
            ));
        }
        match stream.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(e),
        }
    }

    let mut request = axum::http::Request::builder()
        .method(head.method.as_str())
        .uri(&head.path);

    for (name, value) in &head.headers {
        request = request.header(name, value);
    }

    // The one addition, and it only ever takes trust away.
    let mut request = request
        .body(Body::from(body))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    request.extensions_mut().insert(kt_server::ArrivedByRelay);

    // Deliberately no ConnectInfo. The address the edge saw is not evidence of
    // anything here, and the extractor that reads it grants the owner's
    // own-machine exemption.
    Ok(router.oneshot(request).await.unwrap_or_else(|e| match e {}))
}

async fn write_response(
    stream: &mut yamux::Stream,
    response: axum::response::Response,
) -> std::io::Result<()> {
    let (parts, body) = response.into_parts();

    let head = HttpResponseHead {
        status: parts.status.as_u16(),
        headers: parts
            .headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.as_str().to_string(), v.to_string()))
            })
            .collect(),
    };

    let frame = codec::encode(&head)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    stream.write_all(&frame).await?;

    let bytes = axum::body::to_bytes(body, MAX_BODY)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    stream.write_all(&bytes).await?;
    stream.flush().await
}

/// Take one stream and do whatever it says it is for.
///
/// `ended` is how the control stream reaches the connection it belongs to.
/// Every stream runs on its own task, so a control stream that decides the edge
/// has gone cannot simply return and expect anyone to notice - it has to say
/// so, and `carry` is listening.
pub async fn dispatch(
    mut stream: yamux::Stream,
    router: Arc<axum::Router>,
    beat: Heartbeat,
    ended: tokio::sync::mpsc::Sender<Ended>,
) {
    let (open, rest) = match read_open(&mut stream).await {
        Ok(opened) => opened,
        Err(e) => {
            tracing::debug!("a relay stream did not open properly: {e}");
            return;
        }
    };

    match open {
        StreamOpen::Control => {
            let reason = control(stream, rest, beat).await;
            // A full channel means another stream got there first, and a closed
            // one means the connection is already coming down. Either way the
            // ending is known and this one is redundant.
            let _ = ended.try_send(reason);
        }
        StreamOpen::Http(head) => request(stream, head, rest, (*router).clone()).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kt_tunnel_proto::{HttpRequestHead, ViewerOrigin};
    use std::net::IpAddr;
    use std::time::Duration;
    use tokio_util::compat::TokioAsyncReadCompatExt;

    /// Nothing here should take more than a moment, and a hang in this layer
    /// looks like a hang rather than a failure - so the suite says so quickly
    /// instead of sitting there.
    const PATIENCE: Duration = Duration::from_secs(10);

    /// A tunnel with the daemon's dispatch on the far end, and one stream open.
    ///
    /// Both yamux connections are driven by their own task. That is not
    /// ceremony: a yamux connection only makes progress while it is polled, so
    /// awaiting `dispatch` on the same task that drives the connection
    /// deadlocks the pair. `carry` in the parent module spawns for the same
    /// reason.
    async fn open(router: axum::Router) -> yamux::Stream {
        let (a, b) = tokio::io::duplex(256 * 1024);
        let mut edge =
            yamux::Connection::new(a.compat(), yamux::Config::default(), yamux::Mode::Client);
        let mut daemon =
            yamux::Connection::new(b.compat(), yamux::Config::default(), yamux::Mode::Server);

        let router = Arc::new(router);
        // These tests are about streams, not timings, so the heartbeat is the
        // real one and never fires inside a test's lifetime. The verdict
        // channel is drained and dropped for the same reason.
        let (ended_tx, _ended_rx) = tokio::sync::mpsc::channel(1);
        tokio::spawn(async move {
            while let Some(Ok(stream)) =
                std::future::poll_fn(|cx| daemon.poll_next_inbound(cx)).await
            {
                tokio::spawn(dispatch(
                    stream,
                    Arc::clone(&router),
                    Heartbeat::default(),
                    ended_tx.clone(),
                ));
            }
        });

        let stream = std::future::poll_fn(|cx| edge.poll_new_outbound(cx))
            .await
            .expect("a stream");
        tokio::spawn(async move {
            while std::future::poll_fn(|cx| edge.poll_next_inbound(cx))
                .await
                .is_some()
            {}
        });
        stream
    }

    /// A router that reports what the gate was told, so the assertion is about
    /// the trust a relayed request carries rather than a status code that could
    /// be right for the wrong reason.
    fn probe_router() -> axum::Router {
        axum::Router::new().route(
            "/probe",
            axum::routing::get(|req: axum::extract::Request| async move {
                let relayed = req
                    .extensions()
                    .get::<kt_server::ArrivedByRelay>()
                    .is_some();
                let connect_info = req
                    .extensions()
                    .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                    .is_some();
                format!("relayed={relayed} connect_info={connect_info}")
            }),
        )
    }

    fn head(path: &str) -> HttpRequestHead {
        HttpRequestHead {
            hostname: "trip-adarsh.kitchentable.cloud".into(),
            method: "GET".into(),
            path: path.into(),
            headers: vec![("accept".into(), "text/plain".into())],
            viewer: ViewerOrigin {
                address: Some("203.0.113.9".parse::<IpAddr>().expect("an address")),
                tls: true,
            },
        }
    }

    async fn round_trip(
        head: HttpRequestHead,
        router: axum::Router,
    ) -> (HttpResponseHead, Vec<u8>) {
        let mut stream = open(router).await;

        let opener = codec::encode(&StreamOpen::Http(head)).expect("encodes");
        stream.write_all(&opener).await.expect("writes the opener");
        stream.flush().await.expect("flushes");
        // The daemon reads the body until end of stream, so say there is none.
        stream.close().await.expect("closes the write side");

        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .await
            .expect("reads the answer");

        let (response, consumed) = codec::try_decode::<HttpResponseHead>(&raw)
            .expect("decodes")
            .expect("a whole head");
        (response, raw[consumed..].to_vec())
    }

    #[tokio::test]
    async fn a_relayed_request_reaches_the_router_marked_as_relayed() {
        // The point of the whole path. The request arrives at the daemon's own
        // router carrying the one marker that takes trust away, and carrying no
        // connect info - because the extractor that reads connect info is what
        // grants the owner's own-machine exemption.
        let (response, body) =
            tokio::time::timeout(PATIENCE, round_trip(head("/probe"), probe_router()))
                .await
                .expect("no deadlock");

        assert_eq!(response.status, 200);
        assert_eq!(
            String::from_utf8_lossy(&body),
            "relayed=true connect_info=false"
        );
    }

    #[tokio::test]
    async fn the_status_and_headers_come_back_unchanged() {
        // Whatever the gate decided has to survive the trip. A 503 from a
        // paused app is as much an answer as a 200, and the relay adds nothing.
        let router = axum::Router::new().route(
            "/paused",
            axum::routing::get(|| async {
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    [("retry-after", "120")],
                    "asleep",
                )
            }),
        );

        let (response, body) = tokio::time::timeout(PATIENCE, round_trip(head("/paused"), router))
            .await
            .expect("no deadlock");

        assert_eq!(response.status, 503);
        assert!(response
            .headers
            .iter()
            .any(|(n, v)| n == "retry-after" && v == "120"));
        assert_eq!(body, b"asleep");
    }

    #[tokio::test]
    async fn the_path_and_query_reach_the_app_undecoded() {
        // Canonicalisation is the daemon's job and it has a traversal suite for
        // it. Nothing on the way past may helpfully normalise.
        let router = axum::Router::new().route(
            "/echo",
            axum::routing::get(|req: axum::extract::Request| async move { req.uri().to_string() }),
        );

        let (response, body) =
            tokio::time::timeout(PATIENCE, round_trip(head("/echo?a=1&b=%2e%2e"), router))
                .await
                .expect("no deadlock");

        assert_eq!(response.status, 200);
        assert_eq!(String::from_utf8_lossy(&body), "/echo?a=1&b=%2e%2e");
    }

    #[tokio::test]
    async fn request_headers_are_carried_across() {
        let router = axum::Router::new().route(
            "/headers",
            axum::routing::get(|req: axum::extract::Request| async move {
                req.headers()
                    .get("accept")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("none")
                    .to_string()
            }),
        );

        let (response, body) = tokio::time::timeout(PATIENCE, round_trip(head("/headers"), router))
            .await
            .expect("no deadlock");

        assert_eq!(response.status, 200);
        assert_eq!(String::from_utf8_lossy(&body), "text/plain");
    }

    #[tokio::test]
    async fn a_body_larger_than_one_read_arrives_whole() {
        // The request body can span many reads, and a handler that took only
        // the first would corrupt every upload without erroring.
        let router = axum::Router::new().route(
            "/upload",
            axum::routing::post(|body: axum::body::Bytes| async move { body.len().to_string() }),
        );

        let mut posted = head("/upload");
        posted.method = "POST".into();

        let sent = vec![b'x'; 300 * 1024];
        let served = tokio::time::timeout(PATIENCE, async {
            let mut stream = open(router).await;
            let opener = codec::encode(&StreamOpen::Http(posted)).expect("encodes");
            stream.write_all(&opener).await.expect("writes");
            stream.write_all(&sent).await.expect("writes the body");
            stream.flush().await.expect("flushes");
            stream.close().await.expect("closes");

            let mut raw = Vec::new();
            stream.read_to_end(&mut raw).await.expect("reads");
            let (response, consumed) = codec::try_decode::<HttpResponseHead>(&raw)
                .expect("decodes")
                .expect("a whole head");
            (response, raw[consumed..].to_vec())
        })
        .await
        .expect("no deadlock");

        assert_eq!(served.0.status, 200);
        assert_eq!(String::from_utf8_lossy(&served.1), sent.len().to_string());
    }

    #[tokio::test]
    async fn a_stream_that_never_says_what_it_is_for_gets_no_answer() {
        // There is no safe default for a stream whose purpose cannot be read,
        // so it must not be treated as an http request.
        let answered = tokio::time::timeout(PATIENCE, async {
            let mut stream = open(probe_router()).await;
            stream.write_all(b"not a frame").await.expect("writes");
            stream.close().await.expect("closes");

            let mut raw = Vec::new();
            let _ = stream.read_to_end(&mut raw).await;
            raw
        })
        .await
        .expect("no deadlock");

        assert!(answered.is_empty(), "an unreadable stream got a response");
    }

    #[tokio::test]
    async fn the_control_stream_answers_a_ping() {
        // The edge decides whether to serve a snapshot on the strength of this,
        // so a daemon that stops answering is shown to the world as asleep.
        let frame = tokio::time::timeout(PATIENCE, async {
            let mut stream = open(probe_router()).await;
            let opener = codec::encode(&StreamOpen::Control).expect("encodes");
            stream.write_all(&opener).await.expect("writes");
            let ping = codec::encode(&ControlFrame::Ping { seq: 7 }).expect("encodes");
            stream.write_all(&ping).await.expect("writes");
            stream.flush().await.expect("flushes");

            let mut buf = vec![0u8; 128];
            let n = stream.read(&mut buf).await.expect("reads");
            codec::try_decode::<ControlFrame>(&buf[..n])
                .expect("decodes")
                .expect("a whole frame")
                .0
        })
        .await
        .expect("no deadlock");

        assert_eq!(
            frame,
            ControlFrame::Pong { seq: 7 },
            "the ping went unanswered"
        );
    }

    /// A control stream with the daemon's `control` on the far end.
    ///
    /// Deliberately the same shape as [`open`] rather than a second harness:
    /// both yamux connections are driven by their own task forever, because a
    /// connection only makes progress while something polls it and getting that
    /// wrong deadlocks the pair *silently*.
    ///
    /// Returns the edge's side, and the channel `control` reports its verdict
    /// on - which is the thing under test.
    async fn control_stream(
        beat: Heartbeat,
    ) -> (yamux::Stream, tokio::sync::mpsc::Receiver<Ended>) {
        let (a, b) = tokio::io::duplex(64 * 1024);
        let mut edge =
            yamux::Connection::new(a.compat(), yamux::Config::default(), yamux::Mode::Client);
        let mut daemon =
            yamux::Connection::new(b.compat(), yamux::Config::default(), yamux::Mode::Server);

        let router = Arc::new(probe_router());
        let (ended_tx, ended_rx) = tokio::sync::mpsc::channel(1);
        tokio::spawn(async move {
            while let Some(Ok(stream)) =
                std::future::poll_fn(|cx| daemon.poll_next_inbound(cx)).await
            {
                tokio::spawn(dispatch(
                    stream,
                    Arc::clone(&router),
                    beat,
                    ended_tx.clone(),
                ));
            }
        });

        let mut stream = std::future::poll_fn(|cx| edge.poll_new_outbound(cx))
            .await
            .expect("a stream");
        tokio::spawn(async move {
            while std::future::poll_fn(|cx| edge.poll_next_inbound(cx))
                .await
                .is_some()
            {}
        });

        // Saying what the stream is for is what makes dispatch run `control`.
        let opener = codec::encode(&StreamOpen::Control).expect("encodes");
        stream.write_all(&opener).await.expect("writes");
        stream.flush().await.expect("flushes");

        (stream, ended_rx)
    }

    #[tokio::test]
    async fn the_daemon_asks_as_well_as_answers() {
        // The protocol says *each side* pings. For a long time this end only
        // replied, so it could never discover an edge that had gone - which is
        // exactly the failure kt-tunnel-proto's header warns about.
        let beat = Heartbeat {
            interval: Duration::from_millis(20),
            timeout: Duration::from_secs(30),
        };
        let frame = tokio::time::timeout(PATIENCE, async {
            let (mut stream, _ended) = control_stream(beat).await;
            let mut buf = vec![0u8; 128];
            let n = stream.read(&mut buf).await.expect("reads");
            codec::try_decode::<ControlFrame>(&buf[..n])
                .expect("decodes")
                .expect("a whole frame")
                .0
        })
        .await
        .expect("the daemon never asked anything");

        assert!(
            matches!(frame, ControlFrame::Ping { .. }),
            "expected an unprompted ping, got {frame:?}"
        );
    }

    #[tokio::test]
    async fn a_silent_edge_is_given_up_on_so_the_daemon_redials() {
        // The bug this exists for: an edge that stops answering while the
        // socket stays open. Every published app 502s and the daemon never
        // notices. Short timeout, because the claim is that it *fires*.
        let beat = Heartbeat {
            interval: Duration::from_millis(10),
            timeout: Duration::from_millis(30),
        };
        let (_stream, mut ended) = control_stream(beat).await;

        let ended = tokio::time::timeout(PATIENCE, ended.recv())
            .await
            .expect("a silent edge was never given up on")
            .expect("control reported a verdict");

        assert!(
            matches!(ended, Ended::Transport(ref why) if why.contains("stopped answering")),
            "expected the tunnel to be presumed dead, got {ended:?}"
        );
        // And it has to be worth retrying, or noticing achieves nothing.
        assert_eq!(ended.retry(), kt_tunnel_proto::Retry::Backoff);
    }

    #[tokio::test]
    async fn an_edge_that_answers_is_left_alone() {
        // The other half of the claim, and the one with a trap in it: a tight
        // timeout here would measure the runner's scheduler rather than this
        // code, and pass a hundred times locally before failing in CI where a
        // few tens of milliseconds of noise is nothing. A heartbeat test wants
        // timings *asymmetric to its claim* - short when proving a timeout
        // fires, long when proving one does not. So the timeout is long, the
        // interval is short, and the assertion is that nothing happens in a
        // window many intervals wide.
        let beat = Heartbeat {
            interval: Duration::from_millis(10),
            timeout: Duration::from_secs(30),
        };
        let (mut stream, mut ended) = control_stream(beat).await;

        let answering = tokio::spawn(async move {
            let mut buf = vec![0u8; 256];
            while let Ok(n) = stream.read(&mut buf).await {
                if n == 0 {
                    return;
                }
                if let Ok(Some((ControlFrame::Ping { seq }, _))) =
                    codec::try_decode::<ControlFrame>(&buf[..n])
                {
                    let pong = codec::encode(&ControlFrame::Pong { seq }).expect("encodes");
                    if stream.write_all(&pong).await.is_err() {
                        return;
                    }
                    let _ = stream.flush().await;
                }
            }
        });

        // Many intervals, nowhere near the timeout.
        let quit = tokio::time::timeout(Duration::from_millis(300), ended.recv()).await;
        assert!(quit.is_err(), "a healthy tunnel was torn down: {quit:?}");
        answering.abort();
    }
}
