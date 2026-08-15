//! Who may touch which store.
//!
//! Almost every test here is a refusal. The storage API's whole job is keeping
//! one app out of another app's data and one viewer out of another viewer's, so
//! the interesting assertions are the ones where nothing is served.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use kt_server::storage::{StorageEntry, StorageFault, StorageSource, WRITE_HEADER};
use kt_server::{router, router_with_storage, AppSource, Redemption, ServedApp, TrustSource};
use tower::ServiceExt;

/// App, device (`None` being the shared store), key. The same three things the
/// real store keys on, so a test that passes here is testing the real shape.
type Row = (String, Option<String>, String);

/// A store that keeps everything in a map, keyed exactly as the real one is.
#[derive(Default)]
struct Memory {
    rows: Mutex<HashMap<Row, String>>,
    /// Set to make every write answer "full", for the quota test.
    full: bool,
}

impl StorageSource for Memory {
    fn get(
        &self,
        slug: &str,
        device: Option<&str>,
        key: &str,
    ) -> Result<Option<String>, StorageFault> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .get(&(slug.into(), device.map(Into::into), key.into()))
            .cloned())
    }

    fn set(
        &self,
        slug: &str,
        device: Option<&str>,
        key: &str,
        value: &str,
    ) -> Result<(), StorageFault> {
        if self.full {
            return Err(StorageFault::Quota);
        }
        self.rows.lock().unwrap().insert(
            (slug.into(), device.map(Into::into), key.into()),
            value.into(),
        );
        Ok(())
    }

    fn delete(&self, slug: &str, device: Option<&str>, key: &str) -> Result<bool, StorageFault> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .remove(&(slug.into(), device.map(Into::into), key.into()))
            .is_some())
    }

    fn list(
        &self,
        slug: &str,
        device: Option<&str>,
        prefix: &str,
    ) -> Result<Vec<StorageEntry>, StorageFault> {
        let rows = self.rows.lock().unwrap();
        let mut found: Vec<_> = rows
            .iter()
            .filter(|((s, d, k), _)| s == slug && d.as_deref() == device && k.starts_with(prefix))
            .map(|((_, _, key), value)| StorageEntry {
                key: key.clone(),
                value: value.clone(),
                updated_at: 0,
            })
            .collect();
        found.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(found)
    }
}

struct Apps {
    apps: Vec<ServedApp>,
}

impl Apps {
    fn new(visibility: kt_types::Visibility) -> Self {
        let app = |slug: &str| ServedApp {
            slug: slug.into(),
            name: slug.into(),
            root: PathBuf::from("/nonexistent"),
            entry: "index.html".into(),
            visibility,
            paused: false,
            relay: Default::default(),
        };
        Self {
            apps: vec![app("trip"), app("chores")],
        }
    }
}

impl AppSource for Apps {
    fn get(&self, slug: &str) -> Option<ServedApp> {
        self.apps.iter().find(|a| a.slug == slug).cloned()
    }
    fn list(&self) -> Vec<ServedApp> {
        self.apps.clone()
    }
    fn get_by_hostname(&self, hostname: &str) -> Option<ServedApp> {
        self.get(hostname.strip_suffix(".local")?)
    }
}

/// No device, home network. Enough for a Public app and not for a Private one,
/// which is exactly the split these tests want.
struct Anonymous;

impl TrustSource for Anonymous {
    fn device_for(&self, _headers: &axum::http::HeaderMap) -> Option<kt_auth::Device> {
        None
    }
    fn on_household_network(&self) -> bool {
        true
    }
    fn redeem(&self, _headers: &axum::http::HeaderMap, _token: &str) -> Result<Redemption, String> {
        Err("not used here".into())
    }
    fn request_access(
        &self,
        _headers: &axum::http::HeaderMap,
        slug: &str,
        _peer: Option<std::net::IpAddr>,
    ) -> Redemption {
        Redemption {
            cookie: None,
            app_slug: slug.to_string(),
            pending: true,
        }
    }
    fn log(&self, _slug: &str, _device: Option<&kt_auth::DeviceId>, _action: &str) {}
}

struct Call {
    method: &'static str,
    uri: String,
    host: String,
    body: String,
    csrf: bool,
}

impl Call {
    fn get(uri: &str) -> Self {
        Self {
            method: "GET",
            uri: uri.into(),
            host: "trip.local".into(),
            body: String::new(),
            csrf: true,
        }
    }
    fn put(uri: &str, body: &str) -> Self {
        Self {
            method: "PUT",
            uri: uri.into(),
            host: "trip.local".into(),
            body: body.into(),
            csrf: true,
        }
    }
    fn host(mut self, host: &str) -> Self {
        self.host = host.into();
        self
    }
    fn no_csrf(mut self) -> Self {
        self.csrf = false;
        self
    }
}

async fn send(store: Arc<Memory>, apps: Apps, call: Call) -> (StatusCode, String) {
    let router = router_with_storage(
        Arc::new(apps),
        Arc::new(Anonymous),
        Arc::new(kt_server::Presence::new()),
        store,
    );
    let mut request = Request::builder()
        .method(call.method)
        .uri(&call.uri)
        .header("host", &call.host);
    if call.csrf {
        request = request.header(WRITE_HEADER, "1");
    }
    let response = router
        .oneshot(request.body(Body::from(call.body)).expect("builds"))
        .await
        .expect("answers");

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default();
    (status, body)
}

fn public() -> Apps {
    Apps::new(kt_types::Visibility::Public)
}

#[tokio::test]
async fn a_value_written_on_an_apps_own_address_comes_back() {
    let store = Arc::new(Memory::default());
    let (status, _) = send(
        Arc::clone(&store),
        public(),
        Call::put("/__kt/storage/items", r#"["passports"]"#),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = send(store, public(), Call::get("/__kt/storage/items")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, r#"["passports"]"#);
}

#[tokio::test]
async fn one_app_cannot_read_anothers_store() {
    // The rule the whole feature rests on. Each app is its own origin, and the
    // store is resolved from that origin and from nothing the caller can name.
    let store = Arc::new(Memory::default());
    send(
        Arc::clone(&store),
        public(),
        Call::put("/__kt/storage/secret", r#""trip only""#),
    )
    .await;

    let (status, _) = send(
        store,
        public(),
        Call::get("/__kt/storage/secret").host("chores.local"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn storage_is_refused_where_apps_share_an_origin() {
    // An app is also reachable at `<ip>/<slug>/`, for networks where mDNS does
    // not work. Every app served that way is one origin, so a page from one can
    // fetch another's storage route and the browser calls it same-origin. There
    // is no check on this side that can tell them apart, so the honest answer is
    // to refuse rather than serve isolation that is not there.
    let (status, body) = send(
        Arc::new(Memory::default()),
        public(),
        Call::get("/__kt/storage/items").host("192.168.0.5"),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("own address"), "should say why: {body}");
}

#[tokio::test]
async fn a_write_without_the_header_is_refused() {
    // A cross-site request cannot set a custom header without a preflight, and
    // nothing here answers one.
    let (status, _) = send(
        Arc::new(Memory::default()),
        public(),
        Call::put("/__kt/storage/items", "[]").no_csrf(),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_delete_without_the_header_is_refused() {
    let mut call = Call::get("/__kt/storage/items").no_csrf();
    call.method = "DELETE";

    let (status, _) = send(Arc::new(Memory::default()), public(), call).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_viewer_the_gate_turns_away_cannot_read_the_store() {
    // The same gate as the page: anyone who could not open the app cannot read
    // its data either. Private is satisfiable only from this machine, and these
    // requests carry no address at all.
    let (status, _) = send(
        Arc::new(Memory::default()),
        Apps::new(kt_types::Visibility::Private),
        Call::get("/__kt/storage/items"),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn personal_storage_without_a_device_is_refused_rather_than_shared() {
    // A Household app pairs nobody, so most of its viewers have no device.
    // Falling back to the shared store would put one viewer's private note in
    // front of the whole house.
    let (status, body) = send(
        Arc::new(Memory::default()),
        public(),
        Call::get("/__kt/storage/notes?personal=1"),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body.contains("paired device"), "should say why: {body}");
}

#[tokio::test]
async fn a_missing_key_is_not_found_rather_than_an_empty_body() {
    // So a client can tell "nothing stored" from "stored the value null".
    let (status, _) = send(
        Arc::new(Memory::default()),
        public(),
        Call::get("/__kt/storage/never-set"),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_oversized_value_is_refused_before_it_reaches_the_store() {
    let big = "x".repeat(kt_server::storage::MAX_VALUE_BYTES + 1);
    let (status, _) = send(
        Arc::new(Memory::default()),
        public(),
        Call::put("/__kt/storage/blob", &big),
    )
    .await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn a_full_store_says_so_rather_than_failing_opaquely() {
    let store = Arc::new(Memory {
        full: true,
        ..Default::default()
    });
    let (status, _) = send(store, public(), Call::put("/__kt/storage/items", "[]")).await;

    assert_eq!(status, StatusCode::INSUFFICIENT_STORAGE);
}

#[tokio::test]
async fn a_list_is_narrowed_by_prefix() {
    let store = Arc::new(Memory::default());
    send(
        Arc::clone(&store),
        public(),
        Call::put("/__kt/storage/day:1", "\"a\""),
    )
    .await;
    send(
        Arc::clone(&store),
        public(),
        Call::put("/__kt/storage/budget", "\"b\""),
    )
    .await;

    let (status, body) = send(store, public(), Call::get("/__kt/storage?prefix=day:")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("day:1"), "{body}");
    assert!(!body.contains("budget"), "{body}");
}

#[tokio::test]
async fn a_daemon_serving_no_storage_says_so_rather_than_failing() {
    // Every other test in this crate builds a router without a store, and an
    // embedding that only serves files is a supported shape.
    let response = router(
        Arc::new(public()),
        Arc::new(Anonymous),
        Arc::new(kt_server::Presence::new()),
    )
    .oneshot(
        Request::builder()
            .uri("/__kt/storage/items")
            .header("host", "trip.local")
            .body(Body::empty())
            .expect("builds"),
    )
    .await
    .expect("answers");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
