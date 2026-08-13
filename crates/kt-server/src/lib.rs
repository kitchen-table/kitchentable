//! Serving apps over HTTP.
//!
//! Two ways in, in priority order. A request whose `Host` header names an
//! announced app is served from that app's root: the app owns the whole origin.
//! Anything else falls back to path prefixes (`/trip-planner/...`), which is
//! what keeps apps reachable on networks where `.local` does not resolve, and
//! on plain `localhost` (docs/architecture.md section 5).

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{ConnectInfo, FromRequestParts, Path as AxumPath, State},
    http::{header, request::Parts, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

pub mod gate;
pub mod live;
pub mod pages;
pub mod paths;
pub mod presence;

pub use presence::Presence;

/// What the server needs to know to serve one app.
#[derive(Debug, Clone)]
pub struct ServedApp {
    pub slug: String,
    pub name: String,
    /// Canonicalised once at registration, never per request.
    pub root: PathBuf,
    pub entry: String,
    pub visibility: kt_types::Visibility,
    /// Taken offline by the owner. Refused for everyone, including them.
    pub paused: bool,
}

/// Snapshot of what is currently servable. Swapped wholesale when the registry
/// rescans, so a request never sees a half-updated library.
pub trait AppSource: Send + Sync + 'static {
    fn get(&self, slug: &str) -> Option<ServedApp>;
    fn list(&self) -> Vec<ServedApp>;

    /// Resolve an announced hostname, e.g. `trip-planner.local`, to its app.
    ///
    /// Separate from [`AppSource::get`] because the hostname is not always the
    /// slug: a name conflict on the network gets suffixed, so `notes` may be
    /// serving as `notes-2.local`.
    fn get_by_hostname(&self, _hostname: &str) -> Option<ServedApp> {
        None
    }
}

/// What the server needs from the trust layer to answer a request.
///
/// A trait so kt-server never touches the database directly: the daemon owns
/// storage, and this crate stays a thing that turns requests into responses.
pub trait TrustSource: Send + Sync + 'static {
    /// The device behind this request, if its cookie verified.
    fn device_for(&self, headers: &axum::http::HeaderMap) -> Option<kt_auth::Device>;

    /// Whether this machine is currently on a network the owner marked as home.
    fn on_household_network(&self) -> bool;

    /// Redeem an invite for the device behind this request.
    ///
    /// Takes the headers rather than just a user agent so an existing session
    /// is reused: someone re-opening the link they were sent is the same
    /// device, and minting a new identity would make a pinned link look like
    /// it had been forwarded.
    fn redeem(&self, headers: &axum::http::HeaderMap, token: &str) -> Result<Redemption, String>;

    /// Note that a device asked for an app it cannot open yet, so the owner
    /// gets an approval prompt.
    ///
    /// `peer` is where the request came from. It is passed so the daemon can
    /// ask the network what that device calls itself - a far better suggested
    /// name than the user agent - and for nothing else. Nothing is authorised
    /// on the strength of an address the device asserts.
    fn request_access(
        &self,
        headers: &axum::http::HeaderMap,
        slug: &str,
        peer: Option<std::net::IpAddr>,
    ) -> Redemption;

    /// Record an access-log line.
    fn log(&self, slug: &str, device: Option<&kt_auth::DeviceId>, action: &str);

    /// Who has this app open changed.
    ///
    /// Defaulted to nothing because presence is a nicety: a `TrustSource` that
    /// does not care about it - every test double here - should not have to say
    /// so. The daemon overrides it to push the list at the owner's window.
    fn presence_changed(&self, _slug: &str, _viewers: Vec<kt_types::Viewer>) {}

    /// A name for the person sharing, for the viewer pages.
    fn owner_hint(&self) -> String {
        "Someone".to_string()
    }
}

/// The outcome of redeeming a link or asking for access.
#[derive(Debug, Clone)]
pub struct Redemption {
    /// Set-Cookie value, if a session was minted.
    pub cookie: Option<String>,
    /// Where to go next.
    pub app_slug: String,
    /// True when the owner still has to approve.
    pub pending: bool,
}

/// The `Host` header, or empty when there is none.
///
/// Hand-rolled because axum 0.8 moved its own out to axum-extra, and this needs
/// no proxy handling: the daemon is always the origin server, so
/// `X-Forwarded-Host` is not something to trust.
struct Host(String);

impl<S: Send + Sync> FromRequestParts<S> for Host {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Host(
            parts
                .headers
                .get(header::HOST)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string(),
        ))
    }
}

/// The peer address, when the server was started with connect info.
///
/// Absent rather than fatal if it is missing: the only thing it grants is the
/// owner's own-machine exemption, so losing it can refuse but never over-permit.
struct Peer(Option<std::net::IpAddr>);

impl<S: Send + Sync> FromRequestParts<S> for Peer {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Peer(
            parts
                .extensions
                .get::<ConnectInfo<std::net::SocketAddr>>()
                .map(|ConnectInfo(addr)| addr.ip()),
        ))
    }
}

/// Strip the port and lowercase, since a Host header carries both cases and an
/// optional `:port`.
fn normalise_host(host: &str) -> String {
    let without_port = match host.rfind(':') {
        // Leave IPv6 literals alone; they are never one of our hostnames.
        Some(i) if !host.contains(']') => &host[..i],
        _ => host,
    };
    without_port.trim().to_ascii_lowercase()
}

/// Both halves of what a request needs: what is being served, and who may see
/// it.
pub struct ServerState<S: AppSource, T: TrustSource> {
    pub apps: Arc<S>,
    pub trust: Arc<T>,
    /// Who currently has something open. Shared with the daemon, which is what
    /// lets the owner's window show it.
    pub presence: Arc<Presence>,
}

// Written out rather than derived: `#[derive(Clone)]` would demand `S: Clone`
// and `T: Clone`, which is not true and not needed - both fields are already
// behind an Arc, so cloning is two refcount bumps.
impl<S: AppSource, T: TrustSource> Clone for ServerState<S, T> {
    fn clone(&self) -> Self {
        Self {
            apps: Arc::clone(&self.apps),
            trust: Arc::clone(&self.trust),
            presence: Arc::clone(&self.presence),
        }
    }
}

pub fn router<S: AppSource, T: TrustSource>(
    apps: Arc<S>,
    trust: Arc<T>,
    presence: Arc<Presence>,
) -> Router {
    let state = ServerState {
        apps,
        trust,
        presence,
    };

    Router::new()
        // The live-view client and its check-ins. Static segments, so they win
        // against the `/{slug}` routes below rather than being mistaken for an
        // app called `__kt`.
        .route(&format!("{}/live.js", live::PREFIX), get(live_script))
        .route(&format!("{}/beat", live::PREFIX), get(beat::<S, T>))
        // sendBeacon posts; a plain fetch on unload may get a GET. Both mean
        // the same thing, so both are accepted.
        .route(
            &format!("{}/gone", live::PREFIX),
            get(gone::<S, T>).post(gone::<S, T>),
        )
        // Invite redemption lives on the app's own hostname so a shared link
        // never bounces the viewer to another domain.
        .route("/i/{token}", get(redeem::<S, T>))
        // A request whose Host header names an announced app is served from
        // that app's root, whatever the path says. Everything else falls
        // through to prefix routing below.
        .route("/", get(root::<S, T>))
        // All three spellings are needed: a wildcard does not match an empty
        // tail, so `/trip-planner/` would 404 on the route below - and a
        // trailing slash is exactly what a link or a browser produces.
        .route("/{slug}", get(serve_root::<S, T>))
        .route("/{slug}/", get(serve_root::<S, T>))
        .route("/{slug}/{*path}", get(serve_path::<S, T>))
        .with_state(state)
}

/// The live-view client. Static, and the same for every app.
async fn live_script() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            // It changes only when the daemon does, and a stale copy would
            // keep beating at the old interval.
            (header::CACHE_CONTROL, "no-cache"),
        ],
        live::script(),
    )
        .into_response()
}

/// Which app a check-in is about, and the page it came from.
#[derive(serde::Deserialize)]
struct Beat {
    app: String,
    #[serde(default)]
    path: String,
}

/// A page saying it is still open.
///
/// Gated exactly like the page itself: presence is a fact about who is reading
/// an app, so anyone who could not open it cannot appear in its list either.
/// Answers 204 whatever happens - it is a side channel, and a page must never
/// show an error because a check-in failed.
async fn beat<S: AppSource, T: TrustSource>(
    State(state): State<ServerState<S, T>>,
    axum::extract::Query(beat): axum::extract::Query<Beat>,
    headers: axum::http::HeaderMap,
    peer: Peer,
) -> Response {
    let Some(app) = state.apps.get(&beat.app) else {
        return StatusCode::NO_CONTENT.into_response();
    };
    let device = state.trust.device_for(&headers);
    let ctx = kt_auth::RequestContext {
        session: None,
        on_household_network: state.trust.on_household_network(),
        is_loopback: gate::is_loopback(peer.0),
    };
    if !matches!(
        gate::gate(app.visibility, app.paused, device.as_ref(), &ctx),
        gate::Gated::Serve
    ) {
        return StatusCode::NO_CONTENT.into_response();
    }

    // The owner's own browser has no device record, so it is named by what it
    // is rather than being dropped: they are reading it too.
    let who = device
        .as_ref()
        .map(|d| d.id.as_str().to_string())
        .unwrap_or_else(|| OWNER.to_string());

    let path = if beat.path.is_empty() {
        "/".to_string()
    } else {
        beat.path.clone()
    };
    if state.presence.beat(&app.slug, &who, &path) {
        state
            .trust
            .presence_changed(&app.slug, state.presence.viewers(&app.slug));
    }
    StatusCode::NO_CONTENT.into_response()
}

/// A page saying it has gone, so the list empties without waiting for the
/// entry to age out.
async fn gone<S: AppSource, T: TrustSource>(
    State(state): State<ServerState<S, T>>,
    axum::extract::Query(beat): axum::extract::Query<Beat>,
    headers: axum::http::HeaderMap,
) -> Response {
    let who = state
        .trust
        .device_for(&headers)
        .map(|d| d.id.as_str().to_string())
        .unwrap_or_else(|| OWNER.to_string());

    if state.presence.left(&beat.app, &who) {
        state
            .trust
            .presence_changed(&beat.app, state.presence.viewers(&beat.app));
    }
    StatusCode::NO_CONTENT.into_response()
}

/// Stands in for the owner's own browser, which has no device record because it
/// never had to pair with itself.
pub const OWNER: &str = "owner";

/// Redeem an invite link.
///
/// On success the viewer gets a session cookie and lands in the app, or on the
/// wait page if the owner still has to approve. Every failure gets its own
/// sentence: "invalid link" leaves someone with nothing to do.
async fn redeem<S: AppSource, T: TrustSource>(
    State(state): State<ServerState<S, T>>,
    headers: axum::http::HeaderMap,
    AxumPath(token): AxumPath<String>,
) -> Response {
    match state.trust.redeem(&headers, &token) {
        Ok(redemption) => {
            let app_name = state
                .apps
                .get(&redemption.app_slug)
                .map(|a| a.name)
                .unwrap_or_else(|| redemption.app_slug.clone());

            let body = if redemption.pending {
                pages::waiting(&app_name, &state.trust.owner_hint())
            } else {
                pages::interstitial(&app_name, &state.trust.owner_hint())
            };

            let mut response = html(body);
            if let Some(cookie) = redemption.cookie {
                if let Ok(value) = cookie.parse() {
                    response.headers_mut().insert(header::SET_COOKIE, value);
                }
            }
            // The wait page refreshes onto the app root, so point it there.
            if let Ok(value) = format!("/{}/", redemption.app_slug).parse() {
                response.headers_mut().insert(header::REFRESH, value);
            }
            response
        }
        Err(reason) => {
            (StatusCode::FORBIDDEN, html_body(pages::bad_invite(&reason))).into_response()
        }
    }
}

/// `/` means different things depending on who was asked for: an app's own
/// hostname serves the app, anything else shows the index.
async fn root<S: AppSource, T: TrustSource>(
    State(state): State<ServerState<S, T>>,
    headers: axum::http::HeaderMap,
    peer: Peer,
    host: Host,
) -> Response {
    match app_for_host(&*state.apps, &host.0) {
        Some(app) => guarded(&state, &app, "", &headers, &peer).await,
        None => index(&*state.apps).await,
    }
}

/// Run the gate, then serve or show the appropriate page.
///
/// The single place a file is served from, so there is no path that reaches
/// content without passing through here.
async fn guarded<S: AppSource, T: TrustSource>(
    state: &ServerState<S, T>,
    app: &ServedApp,
    path: &str,
    headers: &axum::http::HeaderMap,
    peer: &Peer,
) -> Response {
    let device = state.trust.device_for(headers);
    let ctx = kt_auth::RequestContext {
        session: None,
        on_household_network: state.trust.on_household_network(),
        // The owner's own browser must never have to pair with itself.
        is_loopback: gate::is_loopback(peer.0),
    };

    match gate::gate(app.visibility, app.paused, device.as_ref(), &ctx) {
        gate::Gated::Serve => {
            state
                .trust
                .log(&app.slug, device.as_ref().map(|d| &d.id), "opened");
            serve_app(app, path).await
        }
        gate::Gated::Waiting => {
            let redemption = state.trust.request_access(headers, &app.slug, peer.0);
            let mut response = html(pages::waiting(&app.name, &state.trust.owner_hint()));
            if let Some(cookie) = redemption.cookie {
                if let Ok(value) = cookie.parse() {
                    response.headers_mut().insert(header::SET_COOKIE, value);
                }
            }
            response
        }
        gate::Gated::Refused(reason) => {
            state
                .trust
                .log(&app.slug, device.as_ref().map(|d| &d.id), "refused");
            let body = match reason {
                gate::Refusal::WrongNetwork => pages::wrong_network(&app.name),
                gate::Refusal::Paused => pages::paused(&app.name),
                gate::Refusal::NotShared | gate::Refusal::Revoked => pages::denied(&app.name),
            };
            // 503 rather than 403 for a pause: nothing is wrong with the
            // request or the requester, the app is simply not running.
            let status = match reason {
                gate::Refusal::Paused => StatusCode::SERVICE_UNAVAILABLE,
                _ => StatusCode::FORBIDDEN,
            };
            (status, html_body(body)).into_response()
        }
    }
}

/// The app that owns this origin, if any.
fn app_for_host<S: AppSource>(source: &S, host: &str) -> Option<ServedApp> {
    source.get_by_hostname(&normalise_host(host))
}

async fn index<S: AppSource>(source: &S) -> Response {
    let apps = source.list();

    let body = if apps.is_empty() {
        "<p>No apps yet. Drop a folder into your workspace.</p>".to_string()
    } else {
        let items = apps
            .iter()
            .map(|a| {
                format!(
                    "<li><a href=\"/{slug}/\">{name}</a></li>",
                    slug = escape(&a.slug),
                    name = escape(&a.name)
                )
            })
            .collect::<String>();
        format!("<ul>{items}</ul>")
    };

    html(format!(
        "<!doctype html><meta charset=utf-8><title>Kitchen Table</title>\
         <h1>Kitchen Table</h1>{body}"
    ))
}

async fn serve_root<S: AppSource, T: TrustSource>(
    State(state): State<ServerState<S, T>>,
    headers: axum::http::HeaderMap,
    peer: Peer,
    host: Host,
    AxumPath(slug): AxumPath<String>,
) -> Response {
    serve(&state, &host.0, &slug, "", &headers, &peer).await
}

async fn serve_path<S: AppSource, T: TrustSource>(
    State(state): State<ServerState<S, T>>,
    headers: axum::http::HeaderMap,
    peer: Peer,
    host: Host,
    AxumPath((slug, path)): AxumPath<(String, String)>,
) -> Response {
    serve(&state, &host.0, &slug, &path, &headers, &peer).await
}

async fn serve<S: AppSource, T: TrustSource>(
    state: &ServerState<S, T>,
    host: &str,
    slug: &str,
    path: &str,
    headers: &axum::http::HeaderMap,
    peer: &Peer,
) -> Response {
    // On an app's own hostname the whole path belongs to that app, so
    // `trip-planner.local/chores-rota/x` is a file lookup inside the trip
    // planner, never a way into another app.
    if let Some(app) = app_for_host(&*state.apps, host) {
        let full = if path.is_empty() {
            slug.to_string()
        } else {
            format!("{slug}/{path}")
        };
        return guarded(state, &app, &full, headers, peer).await;
    }

    let Some(app) = state.apps.get(slug) else {
        return not_found("No app by that name is being served.");
    };
    guarded(state, &app, path, headers, peer).await
}

async fn serve_app(app: &ServedApp, path: &str) -> Response {
    let file = match paths::resolve_file(&app.root, path, &app.entry) {
        Ok(file) => file,
        // Deliberately identical responses: telling a caller apart a refusal
        // from a miss leaks whether a path exists outside the app folder.
        Err(paths::ResolveError::NotFound) | Err(paths::ResolveError::Escapes) => {
            return not_found("Not found.")
        }
    };

    match tokio::fs::read(&file).await {
        Ok(bytes) => {
            let mime = mime_guess::from_path(&file).first_or_octet_stream();

            // HTML documents carry the live-view tag; everything else goes out
            // byte for byte. This is the only edit made to anyone's files, it
            // happens on the way out, and nothing is written back to disk.
            //
            // Lossy rather than strict: a page with a stray invalid byte should
            // still render, which is what a browser would do with it anyway.
            let body = if mime.essence_str() == "text/html" {
                let html = String::from_utf8_lossy(&bytes);
                Body::from(live::inject(&html, &app.slug))
            } else {
                Body::from(bytes)
            };

            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, mime.essence_str().to_string()),
                    // Apps are edited live; a cached bundle would make a deploy
                    // look like it did nothing.
                    (header::CACHE_CONTROL, "no-cache".to_string()),
                ],
                body,
            )
                .into_response()
        }
        Err(e) => {
            tracing::warn!(path = %file.display(), error = %e, "failed to read file");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not read that file.",
            )
                .into_response()
        }
    }
}

fn not_found(message: &str) -> Response {
    (StatusCode::NOT_FOUND, message.to_string()).into_response()
}

fn html(body: String) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
}

/// An HTML body with its content type, for responses that are not 200.
fn html_body(body: String) -> ([(header::HeaderName, &'static str); 1], String) {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body)
}

/// App names come from folder names on disk, so they are untrusted input as far
/// as the index page is concerned.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_names_are_escaped_on_the_index() {
        assert_eq!(
            escape(r#"<script>alert("x")</script>"#),
            "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"
        );
    }
}
