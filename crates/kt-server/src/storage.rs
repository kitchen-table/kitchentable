//! The storage API an app's own JavaScript calls.
//!
//! Four operations over one app's key-value store, under the same reserved
//! prefix the live-view script uses. Everything interesting here is about who
//! is allowed to touch which store.
//!
//! **The app is resolved from the `Host` header and from nothing else.** This
//! is the rule the whole feature rests on: if an app could name the store it
//! wanted, one app could read another's data by asking for it. Note that the
//! presence check-in next door *does* take a slug in its query string - that is
//! safe there because the worst it buys is claiming to read a page you are
//! allowed to read, and it would not be safe here.
//!
//! **Which means storage needs the app's own hostname, and says so when it does
//! not have one.** An app is also reachable by path from a bare IP, for the
//! networks where mDNS does not work. Every app served that way shares one
//! origin, so the browser itself stops enforcing anything: a page loaded from
//! `192.168.0.5/packing-list/` can fetch `/chores-rota/__kt/storage/…` and it is
//! a same-origin request. No check on this side can tell those two pages apart,
//! because to the browser they are the same page. So storage is refused on that
//! route rather than quietly served without the isolation it promises.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::{app_for_host, gate, Host, Peer, ServerState};
use crate::{AppSource, TrustSource};

/// Where the storage routes live. Under the same reserved prefix as the
/// live-view script, so an app has one name to avoid rather than two.
pub const PREFIX: &str = "/__kt/storage";

/// The header a write has to carry.
///
/// A cross-site request cannot set a custom header without a CORS preflight,
/// and nothing here answers `OPTIONS`, so the browser refuses before the write
/// is ever sent. The value is not checked: its presence is the whole signal.
/// This sits on top of the session cookie being `SameSite=Lax`, which already
/// keeps it off cross-site writes - two locks on one door, because the cost is
/// one header and the failure is somebody else's page writing to your app.
pub const WRITE_HEADER: &str = "x-kitchen-table";

/// The largest single value an app may store.
///
/// Well under the per-app quota, so an app hits a comprehensible "that value is
/// too big" before it hits an opaque "you are full". Also a cap on what one
/// request can make the daemon hold in memory.
pub const MAX_VALUE_BYTES: usize = 1024 * 1024;

/// What went wrong underneath, in the two flavours the caller can act on.
#[derive(Debug)]
pub enum StorageFault {
    /// The app is at its quota. The app can fix this by storing less.
    Quota,
    /// Anything else. The app cannot fix it and should not be told the detail.
    Failed(String),
}

/// One entry, as the API hands it back.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StorageEntry {
    pub key: String,
    pub value: String,
    pub updated_at: i64,
}

/// The store, as the request layer needs it.
///
/// A trait, and free of any database type, for the same reason [`TrustSource`]
/// is: the daemon owns storage and this crate stays a thing that turns requests
/// into responses.
///
/// `device` is `None` for the app's shared store and `Some(id)` for one
/// device's own. Passing the id rather than a scope type keeps the two crates
/// from having to agree on more than a string.
pub trait StorageSource: Send + Sync + 'static {
    fn get(
        &self,
        slug: &str,
        device: Option<&str>,
        key: &str,
    ) -> Result<Option<String>, StorageFault>;

    fn set(
        &self,
        slug: &str,
        device: Option<&str>,
        key: &str,
        value: &str,
    ) -> Result<(), StorageFault>;

    fn delete(&self, slug: &str, device: Option<&str>, key: &str) -> Result<bool, StorageFault>;

    fn list(
        &self,
        slug: &str,
        device: Option<&str>,
        prefix: &str,
    ) -> Result<Vec<StorageEntry>, StorageFault>;
}

#[derive(Debug, Default, Deserialize)]
pub struct Params {
    /// `?personal=1` for this device's own store rather than the shared one.
    #[serde(default)]
    personal: Option<String>,
    /// `?prefix=day:` to narrow a list.
    #[serde(default)]
    prefix: Option<String>,
}

impl Params {
    fn personal(&self) -> bool {
        matches!(self.personal.as_deref(), Some("1") | Some("true"))
    }
}

/// Everything a storage request needs, once it has earned the right to proceed.
struct Allowed {
    slug: String,
    /// `None` is the shared store.
    device: Option<String>,
}

/// The checks every storage request runs, in the order that makes them cheap.
///
/// Returns the refusal itself rather than a reason code, because each one wants
/// a different status and there is no caller that would want to re-decide.
/// The refusal is boxed because a `Response` is large and this is the unhappy
/// path: every caller returns it immediately, so the indirection costs nothing
/// and keeps the success path's `Result` small.
type Refused = Box<Response>;

fn allow<S: AppSource, T: TrustSource>(
    state: &ServerState<S, T>,
    host: &str,
    headers: &HeaderMap,
    peer: &Peer,
    personal: bool,
) -> Result<Allowed, Refused> {
    // The app comes from the hostname, and only from the hostname. See the
    // module header for why the path route cannot be supported.
    let Some(app) = app_for_host(&*state.apps, host) else {
        return Err(Box::new(refuse(
            StatusCode::BAD_REQUEST,
            "Storage needs this app's own address. Apps reached by IP and path \
             share one origin, so their data could not be kept apart.",
        )));
    };

    // The same gate as the page. Anyone who could not open the app cannot read
    // or write its data either, and this is the only place that is decided.
    let device = state.trust.device_for(headers);
    let ctx = peer.context(state.trust.as_ref());
    if !matches!(
        gate::gate(app.visibility, app.paused, app.relay, device.as_ref(), &ctx),
        gate::Gated::Serve
    ) {
        return Err(Box::new(refuse(
            StatusCode::FORBIDDEN,
            "Not available to this device.",
        )));
    }

    if !personal {
        return Ok(Allowed {
            slug: app.slug,
            device: None,
        });
    }

    // Personal scope is keyed to a device, so a viewer without one has nothing
    // to key on. That is the common case on a Household app, which pairs
    // nobody - falling back to the shared store would put one viewer's private
    // note in front of the whole house, so this refuses instead.
    match device {
        Some(device) => Ok(Allowed {
            slug: app.slug,
            device: Some(device.id.to_string()),
        }),
        None => Err(Box::new(refuse(
            StatusCode::CONFLICT,
            "Personal storage needs a paired device, and this viewer does not \
             have one.",
        ))),
    }
}

/// A write also has to carry the header no cross-site request can set.
fn csrf_ok(headers: &HeaderMap) -> bool {
    headers.contains_key(WRITE_HEADER)
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, message.to_string()).into_response()
}

fn fault(error: StorageFault) -> Response {
    match error {
        StorageFault::Quota => refuse(
            StatusCode::INSUFFICIENT_STORAGE,
            "This app has used all of its storage.",
        ),
        StorageFault::Failed(detail) => {
            // Logged rather than returned: the app can do nothing with it and
            // the detail is about this machine.
            tracing::warn!(%detail, "storage failed");
            refuse(StatusCode::INTERNAL_SERVER_ERROR, "Storage is unavailable.")
        }
    }
}

fn json(body: String) -> Response {
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

pub(crate) async fn read<S: AppSource, T: TrustSource>(
    State(state): State<ServerState<S, T>>,
    Path(key): Path<String>,
    Query(params): Query<Params>,
    host: Host,
    headers: HeaderMap,
    peer: Peer,
) -> Response {
    let Some(store) = state.storage.clone() else {
        return unavailable();
    };
    let allowed = match allow(&state, &host.0, &headers, &peer, params.personal()) {
        Ok(allowed) => allowed,
        Err(response) => return *response,
    };

    match store.get(&allowed.slug, allowed.device.as_deref(), &key) {
        // A missing key is a 404 rather than a null body, so a client can tell
        // "nothing stored" from "stored the JSON value null".
        Ok(None) => refuse(StatusCode::NOT_FOUND, "No such key."),
        Ok(Some(value)) => json(value),
        Err(error) => fault(error),
    }
}

pub(crate) async fn write<S: AppSource, T: TrustSource>(
    State(state): State<ServerState<S, T>>,
    Path(key): Path<String>,
    Query(params): Query<Params>,
    host: Host,
    headers: HeaderMap,
    peer: Peer,
    body: String,
) -> Response {
    let Some(store) = state.storage.clone() else {
        return unavailable();
    };
    if !csrf_ok(&headers) {
        return refuse(
            StatusCode::FORBIDDEN,
            "A write needs the x-kitchen-table header.",
        );
    }
    if body.len() > MAX_VALUE_BYTES {
        return refuse(
            StatusCode::PAYLOAD_TOO_LARGE,
            "That value is larger than one megabyte.",
        );
    }
    let allowed = match allow(&state, &host.0, &headers, &peer, params.personal()) {
        Ok(allowed) => allowed,
        Err(response) => return *response,
    };

    match store.set(&allowed.slug, allowed.device.as_deref(), &key, &body) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => fault(error),
    }
}

pub(crate) async fn remove<S: AppSource, T: TrustSource>(
    State(state): State<ServerState<S, T>>,
    Path(key): Path<String>,
    Query(params): Query<Params>,
    host: Host,
    headers: HeaderMap,
    peer: Peer,
) -> Response {
    let Some(store) = state.storage.clone() else {
        return unavailable();
    };
    if !csrf_ok(&headers) {
        return refuse(
            StatusCode::FORBIDDEN,
            "A write needs the x-kitchen-table header.",
        );
    }
    let allowed = match allow(&state, &host.0, &headers, &peer, params.personal()) {
        Ok(allowed) => allowed,
        Err(response) => return *response,
    };

    match store.delete(&allowed.slug, allowed.device.as_deref(), &key) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => fault(error),
    }
}

pub(crate) async fn list<S: AppSource, T: TrustSource>(
    State(state): State<ServerState<S, T>>,
    Query(params): Query<Params>,
    host: Host,
    headers: HeaderMap,
    peer: Peer,
) -> Response {
    let Some(store) = state.storage.clone() else {
        return unavailable();
    };
    let allowed = match allow(&state, &host.0, &headers, &peer, params.personal()) {
        Ok(allowed) => allowed,
        Err(response) => return *response,
    };

    let prefix = params.prefix.unwrap_or_default();
    match store.list(&allowed.slug, allowed.device.as_deref(), &prefix) {
        Ok(entries) => match serde_json::to_string(&entries) {
            Ok(body) => json(body),
            Err(error) => fault(StorageFault::Failed(error.to_string())),
        },
        Err(error) => fault(error),
    }
}

/// A daemon built without a store. Every existing test does this, and so does
/// any embedding that only serves files.
fn unavailable() -> Response {
    refuse(
        StatusCode::NOT_FOUND,
        "This daemon is not serving app storage.",
    )
}

/// Re-exported for the daemon, which has to build one of these.
pub type Shared = Arc<dyn StorageSource>;
