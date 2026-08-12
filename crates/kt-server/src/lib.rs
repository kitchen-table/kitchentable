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
    extract::{FromRequestParts, Path as AxumPath, State},
    http::{header, request::Parts, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

pub mod gate;
pub mod pages;
pub mod paths;

/// What the server needs to know to serve one app.
#[derive(Debug, Clone)]
pub struct ServedApp {
    pub slug: String,
    pub name: String,
    /// Canonicalised once at registration, never per request.
    pub root: PathBuf,
    pub entry: String,
    pub visibility: kt_types::Visibility,
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

pub fn router<S: AppSource>(source: Arc<S>) -> Router {
    Router::new()
        // A request whose Host header names an announced app is served from
        // that app's root, whatever the path says. Everything else falls
        // through to prefix routing below.
        .route("/", get(root::<S>))
        // All three spellings are needed: a wildcard does not match an empty
        // tail, so `/trip-planner/` would 404 on the route below - and a
        // trailing slash is exactly what a link or a browser produces.
        .route("/{slug}", get(serve_root::<S>))
        .route("/{slug}/", get(serve_root::<S>))
        .route("/{slug}/{*path}", get(serve_path::<S>))
        .with_state(source)
}

/// `/` means different things depending on who was asked for: an app's own
/// hostname serves the app, anything else shows the index.
async fn root<S: AppSource>(State(source): State<Arc<S>>, host: Host) -> Response {
    match app_for_host(&*source, &host.0) {
        Some(app) => serve_app(&app, "").await,
        None => index(&*source).await,
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

async fn serve_root<S: AppSource>(
    State(source): State<Arc<S>>,
    host: Host,
    AxumPath(slug): AxumPath<String>,
) -> Response {
    serve(&*source, &host.0, &slug, "").await
}

async fn serve_path<S: AppSource>(
    State(source): State<Arc<S>>,
    host: Host,
    AxumPath((slug, path)): AxumPath<(String, String)>,
) -> Response {
    serve(&*source, &host.0, &slug, &path).await
}

async fn serve<S: AppSource>(source: &S, host: &str, slug: &str, path: &str) -> Response {
    // On an app's own hostname the whole path belongs to that app, so
    // `trip-planner.local/chores-rota/x` is a file lookup inside the trip
    // planner, never a way into another app.
    if let Some(app) = app_for_host(source, host) {
        let full = if path.is_empty() {
            slug.to_string()
        } else {
            format!("{slug}/{path}")
        };
        return serve_app(&app, &full).await;
    }

    let Some(app) = source.get(slug) else {
        return not_found("No app by that name is being served.");
    };
    serve_app(&app, path).await
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
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, mime.essence_str().to_string()),
                    // Apps are edited live; a cached bundle would make a deploy
                    // look like it did nothing.
                    (header::CACHE_CONTROL, "no-cache".to_string()),
                ],
                Body::from(bytes),
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
