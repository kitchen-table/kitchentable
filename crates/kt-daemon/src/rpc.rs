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

use kt_auth::{DeviceId, DeviceStatus, Invite, InvitePolicy, InviteToken};
use kt_store::Store;

use crate::library::Library;

pub struct Context {
    pub library: Arc<Library>,
    pub store: Arc<Store>,
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
            let record = ctx.library.record(&slug).ok_or_else(|| not_found(&slug))?;
            json(&record.to_app(&ctx.urls_for(&slug)))
        }

        // ---- sharing --------------------------------------------------
        "share.set_visibility" => {
            let slug = string_param(request, "slug")?;
            let raw = string_param(request, "visibility")?;
            let visibility = parse_visibility(&raw)?;

            let record = ctx.library.record(&slug).ok_or_else(|| not_found(&slug))?;
            // The manifest on disk is the source of truth, so write there and
            // let the watcher pick it up. Writing only to the database would
            // make the two disagree the moment anyone edited app.json.
            write_visibility(&record.path, visibility).map_err(|e| KtError {
                code: ErrorCode::Io,
                message: format!("could not update the manifest: {e}"),
                detail: None,
            })?;
            json(&serde_json::json!({ "slug": slug, "visibility": raw }))
        }

        "share.create_invite" => {
            let slug = string_param(request, "slug")?;
            if ctx.library.record(&slug).is_none() {
                return Err(not_found(&slug));
            }

            let label = optional_string(request, "label").unwrap_or_else(|| "Share link".into());
            let policy = InvitePolicy {
                expires_at: request
                    .params
                    .as_ref()
                    .and_then(|p| p.get("expires_in_secs"))
                    .and_then(serde_json::Value::as_i64)
                    .map(|secs| now() + secs),
                pin_to_first_device: optional_bool(request, "pin").unwrap_or(true),
                auto_approve_on_household: optional_bool(request, "auto_approve_at_home")
                    .unwrap_or(false),
            };

            let invite = Invite::new(&slug, &label, policy, now());
            ctx.store.create_invite(&invite).map_err(store_error)?;
            json(&invite_view(ctx, &invite))
        }

        "share.list_invites" => {
            let slug = string_param(request, "slug")?;
            let invites: Vec<_> = ctx
                .store
                .list_invites(&slug)
                .map_err(store_error)?
                .iter()
                .map(|i| invite_view(ctx, i))
                .collect();
            json(&invites)
        }

        "share.revoke_invite" => {
            let raw = string_param(request, "token")?;
            let token = InviteToken::parse(&raw).ok_or_else(|| KtError {
                code: ErrorCode::BadRequest,
                message: "that is not a share link token".into(),
                detail: None,
            })?;
            let revoked = ctx
                .store
                .revoke_invite(&token, now())
                .map_err(store_error)?;
            json(&serde_json::json!({ "revoked": revoked }))
        }

        // ---- devices --------------------------------------------------
        "device.list" => json(&ctx.store.list_devices().map_err(store_error)?),

        "device.approve" => set_device_status(ctx, request, DeviceStatus::Approved),
        "device.revoke" => set_device_status(ctx, request, DeviceStatus::Revoked),

        "device.rename" => {
            let id = device_param(request)?;
            let name = string_param(request, "name")?;
            let renamed = ctx.store.rename_device(&id, &name).map_err(store_error)?;
            json(&serde_json::json!({ "renamed": renamed }))
        }

        // ---- activity -------------------------------------------------
        "log.query" => {
            let slug = optional_string(request, "slug");
            let limit = request
                .params
                .as_ref()
                .and_then(|p| p.get("limit"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(100)
                .min(1000) as u32;

            let events: Vec<_> = ctx
                .store
                .recent_access(slug.as_deref(), limit)
                .map_err(store_error)?
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "at": e.at,
                        "app_slug": e.app_slug,
                        "device_id": e.device_id,
                        "actor": e.actor,
                        "action": e.action,
                        "detail": e.detail,
                    })
                })
                .collect();
            json(&events)
        }

        "sys.status" => json(&SysStatus {
            protocol_version: PROTOCOL_VERSION,
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            workspace: ctx.workspace.clone(),
            serving: ctx.serving.clone(),
            app_count: ctx.library.len() as u32,
            uptime_secs: ctx.started.elapsed().as_secs() as u32,
        }),

        other => Err(KtError {
            code: ErrorCode::BadRequest,
            message: format!("unknown method {other:?}"),
            detail: None,
        }),
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

fn not_found(slug: &str) -> KtError {
    KtError {
        code: ErrorCode::NotFound,
        message: format!("no app with slug {slug:?}"),
        detail: None,
    }
}

fn store_error(e: kt_store::StoreError) -> KtError {
    KtError {
        code: ErrorCode::Internal,
        message: e.to_string(),
        detail: None,
    }
}

fn parse_visibility(raw: &str) -> Result<kt_types::Visibility, KtError> {
    use kt_types::Visibility::*;
    match raw {
        "private" => Ok(Private),
        // `network` is the wire value; Household is only ever a label.
        "network" | "household" => Ok(Network),
        "invited" => Ok(Invited),
        "public" => Ok(Public),
        other => Err(KtError {
            code: ErrorCode::BadRequest,
            message: format!("{other:?} is not a visibility level"),
            detail: None,
        }),
    }
}

/// Rewrite `visibility` in an app's manifest, leaving everything else - unknown
/// keys included - exactly as it was.
fn write_visibility(path: &str, visibility: kt_types::Visibility) -> std::io::Result<()> {
    let manifest_path = std::path::Path::new(path).join("app.json");
    let raw = std::fs::read_to_string(&manifest_path)?;
    let mut manifest: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    manifest["visibility"] = serde_json::json!(match visibility {
        kt_types::Visibility::Private => "private",
        kt_types::Visibility::Network => "network",
        kt_types::Visibility::Invited => "invited",
        kt_types::Visibility::Public => "public",
    });

    let pretty = serde_json::to_string_pretty(&manifest)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&manifest_path, format!("{pretty}\n"))
}

/// An invite as a client sees it, with the URL already built.
fn invite_view(ctx: &Context, invite: &Invite) -> serde_json::Value {
    let origin = ctx
        .library
        .hostname(&invite.app_slug)
        .map(|h| format!("{}://{h}{}", ctx.urls.scheme, ctx.urls.port_suffix))
        .unwrap_or_else(|| ctx.urls.fallback_origin.clone());

    serde_json::json!({
        "token": invite.token.as_str(),
        "app_slug": invite.app_slug,
        "label": invite.label,
        "url": format!("{origin}/i/{}", invite.token),
        "expires_at": invite.policy.expires_at,
        "pinned": invite.pinned_device.is_some(),
        "pin_to_first_device": invite.policy.pin_to_first_device,
        "auto_approve_at_home": invite.policy.auto_approve_on_household,
        "redemptions": invite.redemptions,
        "revoked": invite.revoked_at.is_some(),
        "active": invite.is_active(now()),
    })
}

fn device_param(request: &Request) -> Result<DeviceId, KtError> {
    let raw = string_param(request, "id")?;
    DeviceId::parse(&raw).ok_or_else(|| KtError {
        code: ErrorCode::BadRequest,
        message: "that is not a device id".into(),
        detail: None,
    })
}

fn set_device_status(
    ctx: &Context,
    request: &Request,
    status: DeviceStatus,
) -> Result<serde_json::Value, KtError> {
    let id = device_param(request)?;
    let changed = ctx
        .store
        .set_device_status(&id, status)
        .map_err(store_error)?;
    if !changed {
        return Err(KtError {
            code: ErrorCode::NotFound,
            message: "no such device".into(),
            detail: None,
        });
    }
    json(&serde_json::json!({ "id": id.as_str(), "status": status }))
}

fn optional_string(request: &Request, name: &str) -> Option<String> {
    request
        .params
        .as_ref()
        .and_then(|p| p.get(name))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn optional_bool(request: &Request, name: &str) -> Option<bool> {
    request
        .params
        .as_ref()
        .and_then(|p| p.get(name))
        .and_then(serde_json::Value::as_bool)
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
        library.replace(vec![AppRecord::unmeasured(
            AppManifest {
                name: "Trip Planner".into(),
                slug: "trip-planner".into(),
                icon: None,
                entry: "index.html".into(),
                visibility: Visibility::Private,
                version: 3,
                extra: serde_json::Map::new(),
            },
            dir.clone(),
        )]);

        Context {
            library,
            store: Arc::new(kt_store::Store::in_memory().expect("opens")),
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
    fn creating_a_share_link_returns_a_url_someone_can_send() {
        let ctx = context();
        let req = request(
            "share.create_invite",
            Some(serde_json::json!({ "slug": "trip-planner", "label": "For the family" })),
        );

        let invite = result(&ctx, &req);
        assert_eq!(invite["label"], "For the family");
        assert!(
            invite["url"].as_str().expect("a url").contains("/i/"),
            "a share link has to be a link"
        );
        assert_eq!(invite["pin_to_first_device"], true, "links pin by default");
        assert_eq!(invite["active"], true);
    }

    #[test]
    fn a_share_link_for_an_app_that_does_not_exist_is_refused() {
        let ctx = context();
        let req = request(
            "share.create_invite",
            Some(serde_json::json!({ "slug": "nope" })),
        );
        assert_eq!(error(&ctx, &req).code, ErrorCode::NotFound);
    }

    #[test]
    fn links_can_be_listed_and_revoked() {
        let ctx = context();
        let created = result(
            &ctx,
            &request(
                "share.create_invite",
                Some(serde_json::json!({ "slug": "trip-planner" })),
            ),
        );
        let token = created["token"].as_str().expect("a token").to_string();

        let listed = result(
            &ctx,
            &request(
                "share.list_invites",
                Some(serde_json::json!({ "slug": "trip-planner" })),
            ),
        );
        assert_eq!(listed.as_array().expect("array").len(), 1);

        let revoked = result(
            &ctx,
            &request(
                "share.revoke_invite",
                Some(serde_json::json!({ "token": token })),
            ),
        );
        assert_eq!(revoked["revoked"], true);

        let after = result(
            &ctx,
            &request(
                "share.list_invites",
                Some(serde_json::json!({ "slug": "trip-planner" })),
            ),
        );
        assert_eq!(after[0]["active"], false);
    }

    #[test]
    fn household_is_accepted_as_a_synonym_for_the_wire_value() {
        // The UI may send either; both mean the same level.
        assert_eq!(
            parse_visibility("network").expect("valid"),
            kt_types::Visibility::Network
        );
        assert_eq!(
            parse_visibility("household").expect("valid"),
            kt_types::Visibility::Network
        );
        assert!(parse_visibility("nonsense").is_err());
    }

    #[test]
    fn approving_a_device_that_does_not_exist_is_not_found() {
        let ctx = context();
        let req = request(
            "device.approve",
            Some(serde_json::json!({ "id": kt_auth::DeviceId::generate().as_str() })),
        );
        assert_eq!(error(&ctx, &req).code, ErrorCode::NotFound);
    }

    #[test]
    fn a_malformed_device_id_is_a_bad_request_not_a_lookup() {
        let ctx = context();
        let req = request(
            "device.approve",
            Some(serde_json::json!({ "id": "../../etc" })),
        );
        assert_eq!(error(&ctx, &req).code, ErrorCode::BadRequest);
    }

    #[test]
    fn the_activity_log_is_readable_and_capped() {
        let ctx = context();
        for i in 0..5 {
            ctx.store
                .log_access(&kt_store::AccessEvent {
                    at: 1000 + i,
                    app_slug: Some("trip-planner".into()),
                    device_id: None,
                    actor: "viewer".into(),
                    action: "opened".into(),
                    detail: None,
                })
                .expect("logs");
        }

        let events = result(
            &ctx,
            &request("log.query", Some(serde_json::json!({ "limit": 3 }))),
        );
        assert_eq!(events.as_array().expect("array").len(), 3);
        assert_eq!(events[0]["action"], "opened");
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
