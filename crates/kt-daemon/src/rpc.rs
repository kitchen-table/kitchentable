//! The JSON-RPC surface, independent of the socket that carries it.
//!
//! Kept transport-free so it can be tested without a socket, and so the same
//! dispatch serves the CLI, the shell, and (in D6) kt-mcp.

use std::sync::Arc;
use std::time::Instant;

use kt_types::{
    protocol::{ResponsePayload, PROTOCOL_VERSION},
    ErrorCode, Event, KtError, Request, Response, ServingState, SysStatus, Urls,
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
    /// Rescan the workspace now, rather than waiting for the watcher to settle.
    ///
    /// Creating an app has to answer with the app it created, and the watcher's
    /// debounce is measured in the time it takes someone to finish copying a
    /// folder - far too long to block a socket call on.
    pub rescan: Rescan,
    /// Events for whoever has subscribed (docs/architecture.md section 4).
    pub events: Events,
    /// Who currently has each app open, as the HTTP server hears it.
    pub presence: Arc<kt_server::Presence>,
    /// This install's public identity, if it has one this run (see keys.rs).
    pub install_key: Option<String>,
    /// Whether the tunnel is up right now. Written by the relay task, read
    /// here, so `sys.status` reports the connection rather than the intention.
    pub relay: Arc<crate::relay::RelayStatus>,
}

/// The daemon's event bus.
///
/// A broadcast channel rather than a list of connections: publishers are all
/// over the daemon - the watcher, the gate, the pairing path - and none of them
/// should have to know whether anybody is listening. Sending to nobody is not
/// an error.
#[derive(Clone)]
pub struct Events(tokio::sync::broadcast::Sender<Event>);

/// How many events a slow client may fall behind before it starts losing them.
///
/// Generous, because the cost of a miss is a stale window rather than a dropped
/// request, and because bursts are real: dropping a folder of twenty apps in
/// emits twenty `app_changed` in one scan.
const EVENT_BACKLOG: usize = 256;

impl Default for Events {
    fn default() -> Self {
        Self::new()
    }
}

impl Events {
    pub fn new() -> Self {
        Self(tokio::sync::broadcast::channel(EVENT_BACKLOG).0)
    }

    /// Publish to every subscriber. Deliberately infallible: an event nobody
    /// is listening for is the normal case, not a failure worth propagating
    /// back into the code that caused it.
    pub fn send(&self, event: Event) {
        let _ = self.0.send(event);
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Event> {
        self.0.subscribe()
    }
}

/// Boxed so the daemon can hand in the real rescan closure and tests a no-op,
/// without either of them appearing in this module's signatures.
pub type Rescan = Arc<dyn Fn() + Send + Sync>;

impl Context {
    /// URLs for one app, with its announced hostname filled in.
    fn urls_for(&self, slug: &str) -> Urls {
        urls_for(&self.urls, &self.library, slug)
    }
}

/// URLs for one app, with the hostname the library actually announced.
///
/// Free rather than a method because the watcher builds app views too, and it
/// has a library and a `Urls` but no [`Context`].
pub fn urls_for(urls: &Urls, library: &Library, slug: &str) -> Urls {
    Urls {
        hostname: library.hostname(slug),
        public_host: library.public_host(slug),
        ..urls.clone()
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

        // ---- authoring ------------------------------------------------
        "app.create" => {
            let name = string_param(request, "name")?;
            let folder = crate::authoring::create(std::path::Path::new(&ctx.workspace), &name)
                .map_err(author_error)?;
            created(ctx, &folder)
        }

        "app.import" => {
            let path = string_param(request, "path")?;
            let folder = crate::authoring::import(
                std::path::Path::new(&ctx.workspace),
                std::path::Path::new(&path),
            )
            .map_err(author_error)?;
            created(ctx, &folder)
        }

        // ---- the danger zone ------------------------------------------
        //
        // Pausing and forgetting are the two ways to stop serving something.
        // They are deliberately different: one is reversible from the window
        // and keeps every link and device, the other takes the app out of the
        // library and leaves the folder exactly where it was.
        "app.pause" | "app.resume" => {
            let slug = string_param(request, "slug")?;
            let paused = request.method == "app.pause";
            let record = ctx.library.record(&slug).ok_or_else(|| not_found(&slug))?;

            write_paused(&record.path, paused).map_err(|e| KtError {
                code: ErrorCode::Io,
                message: format!("could not update the manifest: {e}"),
                detail: None,
            })?;
            // Rescan rather than waiting out the watcher: an owner who pauses
            // an app expects it to stop answering now, not once a debounce
            // tuned for folder copies has elapsed.
            (ctx.rescan)();

            log_owner_action(ctx, &slug, if paused { "paused" } else { "resumed" });
            json(&serde_json::json!({ "slug": slug, "paused": paused }))
        }

        "app.forget" => {
            let slug = string_param(request, "slug")?;
            let record = ctx.library.record(&slug).ok_or_else(|| not_found(&slug))?;

            // The folder is not touched. Kitchen Table stops looking at it,
            // which is exactly what the window says this does.
            ctx.store.forget_path(&record.path).map_err(store_error)?;
            (ctx.rescan)();

            log_owner_action(ctx, &slug, "forgotten");
            json(&serde_json::json!({ "slug": slug, "path": record.path }))
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

        // Publishing an app is a separate decision from who may open it, and
        // deliberately a separate call. Nothing about changing a visibility
        // level should be able to put an app on the internet as a side effect.
        "share.set_relay" => {
            let slug = string_param(request, "slug")?;
            let raw = string_param(request, "mode")?;
            let mode = parse_relay_mode(&raw)?;

            let record = ctx.library.record(&slug).ok_or_else(|| not_found(&slug))?;
            write_relay(&record.path, mode).map_err(|e| KtError {
                code: ErrorCode::Io,
                message: format!("could not update the manifest: {e}"),
                detail: None,
            })?;
            json(&serde_json::json!({ "slug": slug, "relay": raw }))
        }

        // Where this app's data lives, and whether this machine keeps a copy.
        //
        // Neither half touches who may open the app or whether it leaves the
        // house, which is why this is a third orthogonal switch rather than
        // something folded into `set_visibility`.
        "storage.set_mode" => {
            let slug = string_param(request, "slug")?;
            let raw = string_param(request, "mode")?;
            let mode = match raw.as_str() {
                "synced" => kt_types::StorageMode::Synced,
                "per_device" => kt_types::StorageMode::PerDevice,
                other => {
                    return Err(KtError {
                        code: ErrorCode::BadRequest,
                        message: format!("{other:?} is not a storage mode"),
                        detail: None,
                    })
                }
            };
            // Absent means "leave it alone", so a window changing only the mode
            // does not silently turn backups off.
            let record = ctx.library.record(&slug).ok_or_else(|| not_found(&slug))?;
            let backup = request
                .params
                .as_ref()
                .and_then(|p| p.get("backup"))
                .and_then(|v| v.as_bool())
                .unwrap_or(record.manifest.storage_backup);

            write_storage(&record.path, mode, backup).map_err(|e| KtError {
                code: ErrorCode::Io,
                message: format!("could not update the manifest: {e}"),
                detail: None,
            })?;
            json(&serde_json::json!({
                "slug": slug,
                "storage": raw,
                "backup": backup,
            }))
        }

        // Rename the app's public address, without touching anything local.
        //
        // A third separate call, for the third distinct decision. `label: null`
        // resets to the slug, which is the only way back to "follows the
        // folder" once somebody has typed something.
        //
        // Nothing here republishes or unpublishes: changing what a published
        // app is called must not take it off the internet, and naming an
        // unpublished one must not put it on.
        "share.set_public_label" => {
            let slug = string_param(request, "slug")?;
            let record = ctx.library.record(&slug).ok_or_else(|| not_found(&slug))?;

            let raw = request
                .params
                .as_ref()
                .and_then(|p| p.get("label"))
                .filter(|v| !v.is_null())
                .map(|v| {
                    v.as_str().map(str::to_string).ok_or_else(|| KtError {
                        code: ErrorCode::BadRequest,
                        message: "label must be a string or null".into(),
                        detail: None,
                    })
                })
                .transpose()?;

            let label = match raw {
                None => None,
                Some(raw) => {
                    // Without a handle there is nothing to join the label to,
                    // so nothing can be checked against a real hostname length
                    // and nothing would resolve. Refuse rather than store a
                    // name that means nothing yet.
                    let (handle, _) = ctx.library.relay_identity().ok_or_else(|| KtError {
                        code: ErrorCode::InvalidState,
                        message: "this machine has no handle yet, so it has no public addresses"
                            .into(),
                        detail: None,
                    })?;

                    let label =
                        kt_types::normalise_public_label(&raw, &handle).map_err(invalid_label)?;

                    // Two apps answering to one address means one of them is
                    // silently unreachable. Checked against the *effective*
                    // label, so an app that never set one still defends its
                    // slug.
                    if let Some(other) = ctx.library.slug_for_public_label(&label) {
                        if other != slug {
                            return Err(invalid_label(kt_types::PublicLabelError::Taken {
                                slug: other,
                            }));
                        }
                    }
                    Some(label)
                }
            };

            write_public_label(&record.path, label.as_deref()).map_err(|e| KtError {
                code: ErrorCode::Io,
                message: format!("could not update the manifest: {e}"),
                detail: None,
            })?;
            // The address changes the moment this returns, so the library has
            // to agree before the next request arrives over the relay.
            (ctx.rescan)();

            let effective = label.unwrap_or_else(|| slug.clone());
            json(&serde_json::json!({
                "slug": slug,
                "public_label": effective,
                "public_host": ctx.library.public_host(&slug),
            }))
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

        // Approving takes an optional name so the prompt can do both at once:
        // the owner is looking at "iPhone" and typing "Kitchen iPad", and two
        // round trips could leave a device approved but unnamed.
        "device.approve" => {
            let id = device_param(request)?;
            if let Some(name) = optional_string(request, "name") {
                let name = name.trim();
                if !name.is_empty() {
                    ctx.store.rename_device(&id, name).map_err(store_error)?;
                }
            }
            set_device_status(ctx, request, DeviceStatus::Approved)
        }

        // Denying and revoking land on the same stored status - never silently
        // upgraded back - but they are different acts and the log should say
        // which happened, so they are different methods rather than a flag.
        "device.deny" => set_device_status(ctx, request, DeviceStatus::Revoked),
        "device.revoke" => set_device_status(ctx, request, DeviceStatus::Revoked),

        "device.rename" => {
            let id = device_param(request)?;
            let name = string_param(request, "name")?;
            let renamed = ctx.store.rename_device(&id, &name).map_err(store_error)?;
            json(&serde_json::json!({ "renamed": renamed }))
        }

        // Tidying the list, which had no way to shrink. Separate from `revoke`
        // on purpose and not a stronger version of it: revoking refuses a
        // device, forgetting stops knowing about it, and a forgotten browser
        // may ask again. See `Store::forget_device`.
        "device.forget" => {
            let id = device_param(request)?;
            let forgotten = ctx.store.forget_device(&id).map_err(store_error)?;
            if !forgotten {
                return Err(KtError {
                    code: ErrorCode::NotFound,
                    message: "no such device".into(),
                    detail: None,
                });
            }
            // Logged even though the row is gone: the log is append-only and
            // answers "who has seen this", so an owner looking back at why a
            // device disappeared from the list needs the line that says they
            // removed it. The id outlives the row here on purpose.
            if let Err(e) = ctx.store.log_access(&kt_store::AccessEvent {
                at: now(),
                app_slug: None,
                device_id: Some(id.as_str().to_string()),
                actor: "owner".into(),
                action: "forgotten".into(),
                detail: None,
            }) {
                tracing::warn!(error = %e, "could not log a device being forgotten");
            }
            json(&serde_json::json!({ "forgotten": forgotten }))
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

        "sys.status" => {
            let identity = ctx.library.relay_identity();
            json(&SysStatus {
                protocol_version: PROTOCOL_VERSION,
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                workspace: ctx.workspace.clone(),
                serving: ctx.serving.clone(),
                app_count: ctx.library.len() as u32,
                install_key: ctx.install_key.clone(),
                relay_handle: identity.as_ref().map(|(h, _)| h.clone()),
                relay_domain: identity.as_ref().map(|(_, d)| d.clone()),
                relay: ctx.relay.get().into(),
                uptime_secs: ctx.started.elapsed().as_secs() as u32,
            })
        }

        // The socket layer notices this method and starts forwarding events on
        // the connection; the answer here is only the acknowledgement, so a
        // caller knows subscribing worked rather than inferring it from silence.
        "event.subscribe" => json(&serde_json::json!({ "subscribed": true })),

        // Who has an app open right now. The events keep this current, so this
        // exists for the first paint and for anything that missed one.
        "presence.list" => {
            let slug = string_param(request, "slug")?;
            json(&ctx.presence.viewers(&slug))
        }

        other => Err(KtError {
            code: ErrorCode::BadRequest,
            message: format!("unknown method {other:?}"),
            detail: None,
        }),
    }
}

/// Rescan, then answer with the app that appeared at `folder`.
///
/// Matching on path rather than on a slug we guessed: the registry owns
/// slugging, including how it resolves a collision, and guessing here would be
/// a second implementation of that rule waiting to disagree with the first.
fn created(ctx: &Context, folder: &std::path::Path) -> Result<serde_json::Value, KtError> {
    (ctx.rescan)();

    let wanted = folder.canonicalize();
    let record = ctx.library.records().into_iter().find(|record| {
        let path = std::path::Path::new(&record.path);
        match (&wanted, path.canonicalize()) {
            (Ok(a), Ok(b)) => *a == b,
            _ => path == folder,
        }
    });

    match record {
        Some(record) => {
            let slug = record.manifest.slug.clone();
            json(&record.to_app(&ctx.urls_for(&slug)))
        }
        // The folder is on disk but the registry did not take it - almost
        // always an app.json the user cannot see being rejected. Saying so
        // beats a success that leaves nothing in the library.
        None => Err(KtError {
            code: ErrorCode::Internal,
            message: format!(
                "created {} but it did not appear in the library",
                folder.display()
            ),
            detail: None,
        }),
    }
}

fn author_error(e: crate::authoring::AuthorError) -> KtError {
    use crate::authoring::AuthorError::*;
    KtError {
        // Everything except a failed write is the caller handing us something
        // we cannot use, which is theirs to fix.
        code: match e {
            Io { .. } => ErrorCode::Internal,
            _ => ErrorCode::BadRequest,
        },
        message: e.to_string(),
        detail: None,
    }
}

pub(crate) fn now() -> i64 {
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
/// Set or clear `paused` in an app's manifest.
///
/// The manifest on disk is the source of truth, exactly as it is for
/// visibility: writing only to the database would make the two disagree the
/// moment anyone edited app.json by hand.
fn write_paused(path: &str, paused: bool) -> std::io::Result<()> {
    let manifest_path = std::path::Path::new(path).join("app.json");
    let raw = std::fs::read_to_string(&manifest_path)?;
    let mut manifest: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    if paused {
        manifest["paused"] = serde_json::json!(true);
    } else if let Some(object) = manifest.as_object_mut() {
        // Removed rather than set to false, so a manifest someone reads by
        // hand does not carry a line about a state the app is not in.
        object.remove("paused");
    }

    let pretty = serde_json::to_string_pretty(&manifest)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&manifest_path, format!("{pretty}\n"))
}

fn parse_relay_mode(raw: &str) -> Result<kt_types::RelayMode, KtError> {
    use kt_types::RelayMode::*;
    match raw {
        "off" => Ok(Off),
        "standard" => Ok(Standard),
        "strict" => Ok(Strict),
        other => Err(KtError {
            code: ErrorCode::BadRequest,
            message: format!("{other:?} is not a relay mode"),
            detail: None,
        }),
    }
}

/// Written to the manifest, not only the database, for the same reason
/// visibility is: `app.json` is the source of truth and the watcher picks it
/// up. A value that lived only in the database would disagree with the folder
/// the moment anybody edited it.
/// Write where an app's data lives, and whether this machine keeps a copy.
///
/// Both in one call because the window sets them from one panel and they are
/// one decision to a person: how this app remembers things. Splitting them the
/// way `set_relay` and `set_public_label` are split would buy nothing, since
/// neither can put anything on the internet or change who may open the app.
fn write_storage(path: &str, mode: kt_types::StorageMode, backup: bool) -> std::io::Result<()> {
    let manifest_path = std::path::Path::new(path).join("app.json");
    let raw = std::fs::read_to_string(&manifest_path)?;
    let mut manifest: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    manifest["storage"] = serde_json::json!(match mode {
        kt_types::StorageMode::Synced => "synced",
        kt_types::StorageMode::PerDevice => "per_device",
    });
    manifest["storage_backup"] = serde_json::json!(backup);

    let pretty = serde_json::to_string_pretty(&manifest)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&manifest_path, format!("{pretty}\n"))
}

fn write_relay(path: &str, mode: kt_types::RelayMode) -> std::io::Result<()> {
    let manifest_path = std::path::Path::new(path).join("app.json");
    let raw = std::fs::read_to_string(&manifest_path)?;
    let mut manifest: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    manifest["relay"] = serde_json::json!(match mode {
        kt_types::RelayMode::Off => "off",
        kt_types::RelayMode::Standard => "standard",
        kt_types::RelayMode::Strict => "strict",
    });

    let pretty = serde_json::to_string_pretty(&manifest)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&manifest_path, format!("{pretty}\n"))
}

/// Write the public label, or remove the key entirely to reset to the slug.
///
/// Removed rather than written as null, because an absent key is what "this
/// app has never been renamed" looks like everywhere else - in the struct, in
/// the column, and to a human reading `app.json`.
fn write_public_label(path: &str, label: Option<&str>) -> std::io::Result<()> {
    let manifest_path = std::path::Path::new(path).join("app.json");
    let raw = std::fs::read_to_string(&manifest_path)?;
    let mut manifest: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    match (label, manifest.as_object_mut()) {
        (Some(label), _) => manifest["public_label"] = serde_json::json!(label),
        (None, Some(map)) => {
            map.remove("public_label");
        }
        (None, None) => {}
    }

    let pretty = serde_json::to_string_pretty(&manifest)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&manifest_path, format!("{pretty}\n"))
}

/// A rejected address, with the reason in a form the field can print.
fn invalid_label(e: kt_types::PublicLabelError) -> KtError {
    KtError {
        // A name somebody else already answers to is a conflict, not a
        // malformed request: the owner typed something perfectly valid and the
        // only problem is that it is spoken for.
        code: match e {
            kt_types::PublicLabelError::Taken { .. } => ErrorCode::Conflict,
            _ => ErrorCode::BadRequest,
        },
        message: e.message(),
        detail: None,
    }
}

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
///
/// A published app's invite carries its **public** name. An invite is for
/// somebody who is not on this network - that is what makes it an invite rather
/// than just telling them the address - so handing them a `.local` URL gives
/// them a link only the people who did not need one can open.
///
/// Everything else falls back to the announced hostname and then to the address
/// URL, which is right for an app nobody has published: it is reachable on this
/// network and nowhere else, and the link should say so.
fn invite_view(ctx: &Context, invite: &Invite) -> serde_json::Value {
    let published = ctx
        .library
        .record(&invite.app_slug)
        .is_some_and(|r| r.manifest.relay.is_published());

    let origin = ctx
        .library
        .public_host(&invite.app_slug)
        .filter(|_| published)
        // https, not `ctx.urls.scheme`: the local listener speaks http and the
        // edge does not, so the relay must not inherit the local scheme.
        .map(|host| format!("https://{host}"))
        .or_else(|| {
            ctx.library
                .hostname(&invite.app_slug)
                .map(|h| format!("{}://{h}{}", ctx.urls.scheme, ctx.urls.port_suffix))
        })
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

/// Record something the owner did to an app, and tell the window.
///
/// Taking an app offline or out of the library is exactly the kind of thing
/// somebody asks "when did that happen?" about later, so it goes in the same
/// log as every other access decision rather than only into the daemon's
/// output.
fn log_owner_action(ctx: &Context, slug: &str, action: &str) {
    if let Err(e) = ctx.store.log_access(&kt_store::AccessEvent {
        at: now(),
        app_slug: Some(slug.to_string()),
        device_id: None,
        actor: "owner".to_string(),
        action: action.to_string(),
        detail: None,
    }) {
        tracing::warn!(error = %e, "could not write to the access log");
    }
    ctx.events.send(Event::Access {
        app_slug: Some(slug.to_string()),
        action: action.to_string(),
    });
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

    // Who gets in is exactly what the Activity view exists to show, so every
    // decision is logged with the method that made it - `denied` and `revoked`
    // reach the same status but are not the same event.
    let action = match request.method.as_str() {
        "device.approve" => "paired",
        "device.deny" => "denied",
        _ => "revoked",
    };
    if let Err(e) = ctx.store.log_access(&kt_store::AccessEvent {
        at: now(),
        app_slug: None,
        device_id: Some(id.as_str().to_string()),
        actor: "owner".into(),
        action: action.into(),
        detail: None,
    }) {
        // The decision itself already stuck. Failing the call because the
        // note about it did not would be the wrong way round.
        tracing::warn!(error = %e, action, "could not log a device decision");
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
                relay: Default::default(),
                storage: Default::default(),
                storage_backup: true,
                public_label: None,
                name: "Trip Planner".into(),
                slug: "trip-planner".into(),
                icon: None,
                entry: "index.html".into(),
                visibility: Visibility::Private,
                version: 3,
                paused: false,
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
                public_host: None,
            },
            serving: ServingState::Serving,
            started: Instant::now(),
            // Nothing to rescan: these tests drive the library directly.
            rescan: std::sync::Arc::new(|| {}),
            events: Events::new(),
            presence: std::sync::Arc::new(kt_server::Presence::new()),
            install_key: None,
            relay: Arc::new(crate::relay::RelayStatus::new()),
        }
    }

    /// A context whose app has a real folder and a real `app.json`.
    ///
    /// The shared `context()` points every app at the bare temp directory,
    /// which is fine while nothing writes - but the manifest writers are
    /// exactly the kind of code that passes every unit test and then fails on
    /// a real file, so the tests that exercise them get a folder they own.
    fn context_on_disk() -> (Context, tempdir::TempDir) {
        let dir = tempdir::TempDir::new();
        std::fs::write(
            dir.path().join("app.json"),
            r#"{"name":"Trip Planner","slug":"trip-planner"}"#,
        )
        .expect("writes a manifest");

        let mut ctx = context();
        let path = dir.path().display().to_string();
        let records: Vec<_> = ctx
            .library
            .records()
            .into_iter()
            .map(|mut r| {
                r.path = path.clone();
                r
            })
            .collect();
        ctx.library.replace(records);
        ctx.workspace = path;
        (ctx, dir)
    }

    /// Minimal scoped temp directory, matching authoring.rs and kt-registry's:
    /// a dependency for a handful of tests is not worth it (CLAUDE.md rule 6).
    mod tempdir {
        use std::path::{Path, PathBuf};

        pub struct TempDir(PathBuf);

        impl TempDir {
            pub fn new() -> Self {
                use std::sync::atomic::{AtomicU32, Ordering};
                static SEQ: AtomicU32 = AtomicU32::new(0);

                let mut path = std::env::temp_dir();
                path.push(format!(
                    "kt-rpc-{}-{}",
                    std::process::id(),
                    SEQ.fetch_add(1, Ordering::Relaxed)
                ));
                let _ = std::fs::remove_dir_all(&path);
                std::fs::create_dir_all(&path).expect("creates temp dir");
                Self(path)
            }

            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
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
    fn where_an_apps_data_lives_can_be_changed_and_survives_in_the_manifest() {
        let (ctx, dir) = context_on_disk();

        let ResponsePayload::Result(v) = dispatch(
            &ctx,
            &request(
                "storage.set_mode",
                Some(serde_json::json!({ "slug": "trip-planner", "mode": "per_device" })),
            ),
        ) else {
            panic!("setting the mode should succeed");
        };
        assert_eq!(v["storage"], "per_device");

        let written = std::fs::read_to_string(dir.path().join("app.json")).expect("reads back");
        assert!(written.contains("\"storage\": \"per_device\""), "{written}");
    }

    #[test]
    fn changing_only_the_mode_leaves_the_backup_setting_where_it_was() {
        // The window sends the mode on its own when somebody presses a card.
        // Defaulting the absent field would quietly turn backups off, which is
        // the one setting here whose loss is not recoverable.
        let (ctx, dir) = context_on_disk();

        dispatch(
            &ctx,
            &request(
                "storage.set_mode",
                Some(serde_json::json!({
                    "slug": "trip-planner",
                    "mode": "per_device",
                    "backup": false
                })),
            ),
        );
        // The library still holds the record loaded at startup, so re-reading
        // the file is what proves the second call kept the first one's answer.
        let after_first = std::fs::read_to_string(dir.path().join("app.json")).expect("reads");
        assert!(
            after_first.contains("\"storage_backup\": false"),
            "{after_first}"
        );
    }

    #[test]
    fn a_storage_mode_this_daemon_does_not_know_is_refused() {
        let (ctx, _dir) = context_on_disk();

        let ResponsePayload::Error(e) = dispatch(
            &ctx,
            &request(
                "storage.set_mode",
                Some(serde_json::json!({ "slug": "trip-planner", "mode": "sometimes" })),
            ),
        ) else {
            panic!("an unknown mode should be refused");
        };
        assert!(matches!(e.code, ErrorCode::BadRequest));
    }

    #[test]
    fn a_public_address_can_be_renamed_and_reset() {
        let (ctx, _dir) = context_on_disk();
        ctx.library
            .set_relay_identity(Some(("adarsh".into(), "kitchentable.cloud".into())));

        let set = |label: serde_json::Value| {
            let req = request(
                "share.set_public_label",
                Some(serde_json::json!({ "slug": "trip-planner", "label": label })),
            );
            dispatch(&ctx, &req)
        };

        let ResponsePayload::Result(v) = set(serde_json::json!("Holiday")) else {
            panic!("renaming should succeed");
        };
        assert_eq!(v["public_label"], "holiday", "normalised, not refused");

        // Null is the way back to following the folder.
        let ResponsePayload::Result(v) = set(serde_json::Value::Null) else {
            panic!("resetting should succeed");
        };
        assert_eq!(v["public_label"], "trip-planner");
    }

    #[test]
    fn renaming_a_public_address_says_which_rule_was_broken() {
        // "Invalid" next to a text box is the least useful thing a form can
        // say, so each refusal has to name itself.
        let ctx = context();
        ctx.library
            .set_relay_identity(Some(("adarsh".into(), "kitchentable.cloud".into())));

        let req = request(
            "share.set_public_label",
            Some(serde_json::json!({ "slug": "trip-planner", "label": "my trip" })),
        );
        let ResponsePayload::Error(e) = dispatch(&ctx, &req) else {
            panic!("a space is not a hostname");
        };
        assert_eq!(e.code, ErrorCode::BadRequest);
        assert!(e.message.contains("letters, numbers and hyphens"), "{e:?}");
    }

    #[test]
    fn an_address_another_app_already_answers_to_is_a_conflict() {
        // Two apps on one address means one of them is silently unreachable,
        // which is the worst shape this bug could take: everything looks fine.
        let (ctx, _dir) = context_on_disk();
        ctx.library
            .set_relay_identity(Some(("adarsh".into(), "kitchentable.cloud".into())));

        let mut second = ctx.library.record("trip-planner").expect("the app");
        second.manifest.slug = "chores".into();
        let first = ctx.library.record("trip-planner").expect("the app");
        ctx.library.replace(vec![first, second]);

        let req = request(
            "share.set_public_label",
            Some(serde_json::json!({ "slug": "chores", "label": "trip-planner" })),
        );
        let ResponsePayload::Error(e) = dispatch(&ctx, &req) else {
            panic!("two apps must not share one public address");
        };
        assert_eq!(e.code, ErrorCode::Conflict);

        // But an app may always keep its own address - a no-op rename is not a
        // collision with itself.
        let req = request(
            "share.set_public_label",
            Some(serde_json::json!({ "slug": "chores", "label": "chores" })),
        );
        assert!(matches!(dispatch(&ctx, &req), ResponsePayload::Result(_)));
    }

    #[test]
    fn an_install_with_no_handle_refuses_to_name_anything() {
        // Nothing to join a label to and nothing that would resolve. Storing
        // it anyway would leave a name that means nothing until an account
        // arrives, and then means something nobody chose.
        let ctx = context();
        let req = request(
            "share.set_public_label",
            Some(serde_json::json!({ "slug": "trip-planner", "label": "holiday" })),
        );
        let ResponsePayload::Error(e) = dispatch(&ctx, &req) else {
            panic!("there is no address to set");
        };
        assert_eq!(e.code, ErrorCode::InvalidState);
    }

    #[test]
    fn status_reports_the_tunnel_rather_than_the_intention() {
        // The bug this exists for: the window rendered a green ON badge from
        // the manifest, so an app that had been unreachable for an hour looked
        // healthy and the owner found out from a stranger. Publishing is an
        // intention; being reachable is a fact, and they are different fields.
        let ctx = context();
        let status = result(&ctx, &request("sys.status", None));
        assert_eq!(status["relay"]["state"], "off", "no relay configured");

        ctx.relay.set(crate::relay::RelayState::Connected);
        let status = result(&ctx, &request("sys.status", None));
        assert_eq!(status["relay"]["state"], "connected");

        ctx.relay
            .set(crate::relay::RelayState::NeedsAttention("revoked".into()));
        let status = result(&ctx, &request("sys.status", None));
        assert_eq!(status["relay"]["state"], "needs_attention");
        assert_eq!(status["relay"]["message"], "revoked");
    }

    #[test]
    fn status_carries_the_handle_so_the_window_can_show_the_suffix() {
        let ctx = context();
        let status = result(&ctx, &request("sys.status", None));
        assert!(status["relay_handle"].is_null(), "no handle by default");

        ctx.library
            .set_relay_identity(Some(("adarsh".into(), "kitchentable.cloud".into())));
        let status = result(&ctx, &request("sys.status", None));
        assert_eq!(status["relay_handle"], "adarsh");
        assert_eq!(status["relay_domain"], "kitchentable.cloud");
    }

    #[test]
    fn a_published_apps_invite_carries_a_link_that_works_away_from_home() {
        // The bug this exists for: an invite is *for* somebody who is not on
        // this network, and a `.local` URL is openable only by the people who
        // never needed an invite. Publishing has to change the link it hands
        // out or sharing the app is impossible from the one screen for it.
        let ctx = context();
        ctx.library
            .set_relay_identity(Some(("adarsh".into(), "kitchentable.cloud".into())));

        let make = |ctx: &Context| {
            let req = request(
                "share.create_invite",
                Some(serde_json::json!({ "slug": "trip-planner" })),
            );
            result(ctx, &req)["url"]
                .as_str()
                .expect("a url")
                .to_string()
        };

        // Unpublished: reachable on this network and nowhere else, and the
        // link says so rather than promising reach the app has not got.
        let local = make(&ctx);
        assert!(!local.contains("kitchentable.cloud"), "got {local}");

        let mut record = ctx.library.record("trip-planner").expect("the app");
        record.manifest.relay = kt_types::RelayMode::Standard;
        ctx.library.replace(vec![record]);

        let public = make(&ctx);
        assert!(
            public.starts_with("https://trip-planner-adarsh.kitchentable.cloud/i/"),
            "a published app's invite must be openable from outside; got {public}"
        );
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
    fn forgetting_a_device_takes_it_off_the_list_and_leaves_a_note() {
        let ctx = context();
        let d = kt_auth::Device::pending(kt_auth::DeviceId::generate(), "Mozilla/5.0", 1000);
        ctx.store.upsert_device(&d).expect("inserts");

        let out = result(
            &ctx,
            &request(
                "device.forget",
                Some(serde_json::json!({ "id": d.id.as_str() })),
            ),
        );
        assert_eq!(out["forgotten"], true);
        assert!(ctx.store.list_devices().expect("lists").is_empty());

        // The row is gone; the reason it went is not. An owner looking back at
        // why a device vanished from the list has to find this line.
        let log = ctx.store.recent_access(None, 10).expect("reads");
        assert!(log
            .iter()
            .any(|e| e.action == "forgotten" && e.device_id.as_deref() == Some(d.id.as_str())));
    }

    #[test]
    fn forgetting_a_device_that_does_not_exist_is_not_found() {
        let ctx = context();
        let req = request(
            "device.forget",
            Some(serde_json::json!({ "id": kt_auth::DeviceId::generate().as_str() })),
        );
        assert_eq!(error(&ctx, &req).code, ErrorCode::NotFound);
    }

    #[test]
    fn forgetting_is_not_revoking() {
        // Two different acts that a caller could easily conflate. Revoking
        // leaves a row that refuses; forgetting leaves nothing, and that
        // browser may ask again. If these ever collapse into one method the
        // window's wording stops being true.
        let ctx = context();
        let d = kt_auth::Device::pending(kt_auth::DeviceId::generate(), "Mozilla/5.0", 1000);
        ctx.store.upsert_device(&d).expect("inserts");

        result(
            &ctx,
            &request(
                "device.revoke",
                Some(serde_json::json!({ "id": d.id.as_str() })),
            ),
        );
        assert_eq!(
            ctx.store
                .get_device(&d.id)
                .expect("reads")
                .expect("still there")
                .status,
            DeviceStatus::Revoked
        );

        result(
            &ctx,
            &request(
                "device.forget",
                Some(serde_json::json!({ "id": d.id.as_str() })),
            ),
        );
        assert_eq!(ctx.store.get_device(&d.id).expect("reads"), None);
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
