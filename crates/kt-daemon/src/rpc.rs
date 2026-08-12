//! The JSON-RPC surface, independent of the socket that carries it.
//!
//! Kept transport-free so it can be tested without a socket, and so the same
//! dispatch serves the CLI, the shell, and (in D6) kt-mcp.

use std::sync::Arc;
use std::time::Instant;

use kt_types::{
    protocol::{ResponsePayload, PROTOCOL_VERSION},
    ErrorCode, KtError, Request, Response, ServingState, SysStatus, Urls,
};

use crate::library::Library;

pub struct Context {
    pub library: Arc<Library>,
    pub workspace: String,
    /// URL shape for this run, minus the per-app hostname, which comes from
    /// the library because only it knows what was actually announced.
    pub urls: Urls,
    pub serving: ServingState,
    pub started: Instant,
}

impl Context {
    /// URLs for one app, with its announced hostname filled in.
    fn urls_for(&self, slug: &str) -> Urls {
        Urls {
            hostname: self.library.hostname(slug),
            ..self.urls.clone()
        }
    }
}

/// Dispatch one request. Never panics and never returns `Err`: a failure is a
/// [`KtError`] the caller can act on.
pub fn dispatch(ctx: &Context, request: &Request) -> ResponsePayload {
    match handle(ctx, request) {
        Ok(value) => ResponsePayload::Result(value),
        Err(e) => ResponsePayload::Error(e),
    }
}

fn handle(ctx: &Context, request: &Request) -> Result<serde_json::Value, KtError> {
    match request.method.as_str() {
        "app.list" => {
            let apps: Vec<_> = ctx
                .library
                .records()
                .iter()
                .map(|r| r.to_app(&ctx.urls_for(&r.manifest.slug)))
                .collect();
            json(&apps)
        }

        "app.get" => {
            let slug = string_param(request, "slug")?;
            let record = ctx.library.record(&slug).ok_or_else(|| KtError {
                code: ErrorCode::NotFound,
                message: format!("no app with slug {slug:?}"),
                detail: None,
            })?;
            json(&record.to_app(&ctx.urls_for(&slug)))
        }

        "sys.status" => json(&SysStatus {
            protocol_version: PROTOCOL_VERSION,
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            workspace: ctx.workspace.clone(),
            serving: ctx.serving.clone(),
            app_count: ctx.library.len() as u32,
            uptime_secs: ctx.started.elapsed().as_secs(),
        }),

        other => Err(KtError {
            code: ErrorCode::BadRequest,
            message: format!("unknown method {other:?}"),
            detail: None,
        }),
    }
}

fn string_param(request: &Request, name: &str) -> Result<String, KtError> {
    request
        .params
        .as_ref()
        .and_then(|p| p.get(name))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| KtError {
            code: ErrorCode::BadRequest,
            message: format!("{} requires a string {name:?} parameter", request.method),
            detail: None,
        })
}

fn json<T: serde::Serialize>(value: &T) -> Result<serde_json::Value, KtError> {
    serde_json::to_value(value).map_err(|e| KtError {
        code: ErrorCode::Internal,
        message: format!("could not serialise the response: {e}"),
        detail: None,
    })
}

/// Build a response envelope for a request.
pub fn respond(ctx: &Context, request: &Request) -> Response {
    Response {
        jsonrpc: kt_types::protocol::JsonRpcVersion,
        id: request.id,
        payload: dispatch(ctx, request),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kt_types::{AppManifest, AppRecord, Visibility};

    fn context() -> Context {
        let library = Arc::new(Library::new());
        let dir = std::env::temp_dir().display().to_string();
        library.replace(vec![AppRecord {
            manifest: AppManifest {
                name: "Trip Planner".into(),
                slug: "trip-planner".into(),
                icon: None,
                entry: "index.html".into(),
                visibility: Visibility::Private,
                version: 3,
                extra: serde_json::Map::new(),
            },
            path: dir.clone(),
        }]);

        Context {
            library,
            workspace: dir,
            urls: Urls {
                scheme: "http".into(),
                hostname: None,
                port_suffix: ":8420".into(),
                prefix_origin: "http://localhost:8420".into(),
                fallback_origin: "http://192.168.1.24:8420".into(),
            },
            serving: ServingState::Serving,
            started: Instant::now(),
        }
    }

    fn request(method: &str, params: Option<serde_json::Value>) -> Request {
        Request {
            jsonrpc: kt_types::protocol::JsonRpcVersion,
            id: Some(1),
            method: method.into(),
            params,
        }
    }

    fn result(ctx: &Context, req: &Request) -> serde_json::Value {
        match dispatch(ctx, req) {
            ResponsePayload::Result(v) => v,
            ResponsePayload::Error(e) => panic!("expected a result, got {e:?}"),
        }
    }

    fn error(ctx: &Context, req: &Request) -> KtError {
        match dispatch(ctx, req) {
            ResponsePayload::Error(e) => e,
            ResponsePayload::Result(v) => panic!("expected an error, got {v}"),
        }
    }

    #[test]
    fn app_list_returns_apps_with_derived_urls() {
        let ctx = context();
        let apps = result(&ctx, &request("app.list", None));

        assert_eq!(apps.as_array().expect("array").len(), 1);
        assert_eq!(apps[0]["slug"], "trip-planner");
        assert_eq!(apps[0]["url"], "http://localhost:8420/trip-planner");
        assert_eq!(
            apps[0]["fallback_url"],
            "http://192.168.1.24:8420/trip-planner"
        );
    }

    #[test]
    fn app_get_finds_by_slug() {
        let ctx = context();
        let req = request(
            "app.get",
            Some(serde_json::json!({ "slug": "trip-planner" })),
        );
        assert_eq!(result(&ctx, &req)["name"], "Trip Planner");
    }

    #[test]
    fn app_get_on_a_missing_slug_is_not_found() {
        let ctx = context();
        let req = request("app.get", Some(serde_json::json!({ "slug": "nope" })));
        assert_eq!(error(&ctx, &req).code, ErrorCode::NotFound);
    }

    #[test]
    fn a_missing_parameter_is_a_bad_request_not_a_panic() {
        let ctx = context();
        assert_eq!(
            error(&ctx, &request("app.get", None)).code,
            ErrorCode::BadRequest
        );
        assert_eq!(
            error(
                &ctx,
                &request("app.get", Some(serde_json::json!({ "slug": 7 })))
            )
            .code,
            ErrorCode::BadRequest
        );
    }

    #[test]
    fn sys_status_advertises_the_protocol_version() {
        let ctx = context();
        let status = result(&ctx, &request("sys.status", None));

        assert_eq!(status["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(status["app_count"], 1);
        assert_eq!(status["serving"]["state"], "serving");
    }

    #[test]
    fn an_unknown_method_is_a_bad_request() {
        let ctx = context();
        let e = error(&ctx, &request("app.destroy_everything", None));
        assert_eq!(e.code, ErrorCode::BadRequest);
        assert!(e.message.contains("app.destroy_everything"));
    }

    #[test]
    fn a_response_carries_the_request_id_back() {
        let ctx = context();
        let res = respond(&ctx, &request("sys.status", None));
        assert_eq!(res.id, Some(1));
    }
}
