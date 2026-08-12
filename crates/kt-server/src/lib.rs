//! Serving apps over HTTP.
//!
//! D1 routes by path prefix (`/trip-planner/...`), which is also the permanent
//! fallback for networks where `.local` does not resolve. Hostname routing by
//! `Host` header, and the per-app origin isolation it buys, arrives in D2
//! (docs/architecture.md section 5).

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path as AxumPath, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

pub mod paths;

/// What the server needs to know to serve one app.
#[derive(Debug, Clone)]
pub struct ServedApp {
    pub slug: String,
    pub name: String,
    /// Canonicalised once at registration, never per request.
    pub root: PathBuf,
    pub entry: String,
}

/// Snapshot of what is currently servable. Swapped wholesale when the registry
/// rescans, so a request never sees a half-updated library.
pub trait AppSource: Send + Sync + 'static {
    fn get(&self, slug: &str) -> Option<ServedApp>;
    fn list(&self) -> Vec<ServedApp>;
}

pub fn router<S: AppSource>(source: Arc<S>) -> Router {
    Router::new()
        .route("/", get(index::<S>))
        // All three spellings are needed: a wildcard does not match an empty
        // tail, so `/trip-planner/` would 404 on the route below - and a
        // trailing slash is exactly what a link or a browser produces.
        .route("/{slug}", get(serve_root::<S>))
        .route("/{slug}/", get(serve_root::<S>))
        .route("/{slug}/{*path}", get(serve_path::<S>))
        .with_state(source)
}

async fn index<S: AppSource>(State(source): State<Arc<S>>) -> Response {
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
    AxumPath(slug): AxumPath<String>,
) -> Response {
    serve(&*source, &slug, "").await
}

async fn serve_path<S: AppSource>(
    State(source): State<Arc<S>>,
    AxumPath((slug, path)): AxumPath<(String, String)>,
) -> Response {
    serve(&*source, &slug, &path).await
}

async fn serve<S: AppSource>(source: &S, slug: &str, path: &str) -> Response {
    let Some(app) = source.get(slug) else {
        return not_found("No app by that name is being served.");
    };

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
