//! What the router actually does with a URL.
//!
//! The path-resolution unit tests cover which files may be reached; these cover
//! whether a request reaches the resolver at all. That gap is real: a wildcard
//! route silently does not match an empty tail, so `/app/` once 404'd while
//! `/app` and `/app/index.html` both worked.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use kt_server::{router, AppSource, Redemption, ServedApp, TrustSource};
use tower::ServiceExt;

struct Fixture {
    apps: Vec<ServedApp>,
    dir: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEQ: AtomicU32 = AtomicU32::new(0);

        let dir = std::env::temp_dir().join(format!(
            "kt-serving-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);

        let root = dir.join("trip");
        std::fs::create_dir_all(root.join("sub")).expect("creates dirs");
        std::fs::write(root.join("index.html"), "<h1>home</h1>").expect("writes");
        std::fs::write(root.join("sub/index.html"), "<h1>sub</h1>").expect("writes");
        std::fs::write(root.join("style.css"), "body{color:red}").expect("writes");

        // A file outside any app, to prove traversal cannot reach it.
        std::fs::write(dir.join("secret.txt"), "PRIVATE").expect("writes");

        let chores = dir.join("chores");
        std::fs::create_dir_all(&chores).expect("creates dirs");
        std::fs::write(chores.join("index.html"), "<h1>chores</h1>").expect("writes");

        Self {
            apps: vec![
                ServedApp {
                    slug: "trip".into(),
                    name: "Trip Planner".into(),
                    root: root.canonicalize().expect("canonicalises"),
                    entry: "index.html".into(),
                    visibility: kt_types::Visibility::Public,
                },
                ServedApp {
                    slug: "chores".into(),
                    name: "Chores Rota".into(),
                    root: chores.canonicalize().expect("canonicalises"),
                    entry: "index.html".into(),
                    visibility: kt_types::Visibility::Public,
                },
            ],
            dir,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl AppSource for Fixture {
    fn get(&self, slug: &str) -> Option<ServedApp> {
        self.apps.iter().find(|a| a.slug == slug).cloned()
    }
    fn list(&self) -> Vec<ServedApp> {
        self.apps.clone()
    }
    fn get_by_hostname(&self, hostname: &str) -> Option<ServedApp> {
        // The daemon keeps the real map; here the announced name is the slug
        // plus `.local`, plus one deliberately renamed app.
        let slug = hostname.strip_suffix(".local")?;
        let slug = if slug == "chores-2" { "chores" } else { slug };
        self.get(slug)
    }
}

async fn get(fixture: Arc<Fixture>, uri: &str) -> (StatusCode, String, Option<String>) {
    get_with_host(fixture, uri, "localhost").await
}

/// Lets every request straight through, so these tests stay about routing.
/// The gate has its own matrix in kt-auth.
struct AllowAll;

impl TrustSource for AllowAll {
    fn device_for(&self, _headers: &axum::http::HeaderMap) -> Option<kt_auth::Device> {
        None
    }
    fn on_household_network(&self) -> bool {
        true
    }
    fn redeem(&self, _headers: &axum::http::HeaderMap, _token: &str) -> Result<Redemption, String> {
        Err("not used here".into())
    }
    fn request_access(&self, _headers: &axum::http::HeaderMap, slug: &str) -> Redemption {
        Redemption {
            cookie: None,
            app_slug: slug.to_string(),
            pending: true,
        }
    }
    fn log(&self, _slug: &str, _device: Option<&kt_auth::DeviceId>, _action: &str) {}
}

async fn get_with_host(
    fixture: Arc<Fixture>,
    uri: &str,
    host: &str,
) -> (StatusCode, String, Option<String>) {
    let response = router(fixture, Arc::new(AllowAll))
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("host", host)
                .body(Body::empty())
                .expect("builds request"),
        )
        .await
        .expect("serves");

    let status = response.status();
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("reads body");

    (
        status,
        String::from_utf8_lossy(&bytes).into_owned(),
        content_type,
    )
}

#[tokio::test]
async fn every_spelling_of_the_app_root_serves_the_entry_point() {
    let f = Arc::new(Fixture::new());

    for uri in ["/trip", "/trip/", "/trip/index.html"] {
        let (status, body, _) = get(Arc::clone(&f), uri).await;
        assert_eq!(status, StatusCode::OK, "{uri} should serve");
        assert_eq!(body, "<h1>home</h1>", "{uri} should be the entry point");
    }
}

#[tokio::test]
async fn a_subdirectory_serves_its_own_entry_point() {
    let f = Arc::new(Fixture::new());
    let (status, body, _) = get(f, "/trip/sub/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "<h1>sub</h1>");
}

#[tokio::test]
async fn content_type_comes_from_the_extension() {
    let f = Arc::new(Fixture::new());

    let (_, _, ct) = get(Arc::clone(&f), "/trip/style.css").await;
    assert_eq!(ct.as_deref(), Some("text/css"));

    let (_, _, ct) = get(f, "/trip/index.html").await;
    assert_eq!(ct.as_deref(), Some("text/html"));
}

#[tokio::test]
async fn the_index_lists_what_is_being_served() {
    let f = Arc::new(Fixture::new());
    let (status, body, _) = get(f, "/").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Trip Planner"), "index should name the app");
    assert!(body.contains("/trip/"), "index should link to the app");
}

#[tokio::test]
async fn an_unknown_app_is_not_found() {
    let f = Arc::new(Fixture::new());
    let (status, _, _) = get(f, "/nope/").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn traversal_out_of_the_app_is_refused() {
    let f = Arc::new(Fixture::new());

    for uri in [
        "/trip/../secret.txt",
        "/trip/sub/../../secret.txt",
        "/trip/%2e%2e/secret.txt",
        "/trip/..%2Fsecret.txt",
    ] {
        let (status, body, _) = get(Arc::clone(&f), uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri} should be refused");
        assert!(
            !body.contains("PRIVATE"),
            "{uri} leaked a file outside the app"
        );
    }
}

#[tokio::test]
async fn a_refusal_is_indistinguishable_from_a_miss() {
    // Different responses would tell a caller whether a path exists outside
    // the app folder.
    let f = Arc::new(Fixture::new());

    let (escape_status, escape_body, _) = get(Arc::clone(&f), "/trip/../secret.txt").await;
    let (miss_status, miss_body, _) = get(f, "/trip/no-such-file.txt").await;

    assert_eq!(escape_status, miss_status);
    assert_eq!(escape_body, miss_body);
}

// ---- host-based routing ----

#[tokio::test]
async fn an_app_hostname_serves_that_app_at_the_root() {
    let f = Arc::new(Fixture::new());
    let (status, body, _) = get_with_host(f, "/", "trip.local").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "<h1>home</h1>", "the app owns its own origin");
}

#[tokio::test]
async fn a_host_header_is_matched_case_insensitively_and_without_the_port() {
    let f = Arc::new(Fixture::new());

    for host in [
        "trip.local",
        "TRIP.local",
        "Trip.Local:80",
        "trip.local:8420",
    ] {
        let (status, body, _) = get_with_host(Arc::clone(&f), "/", host).await;
        assert_eq!(status, StatusCode::OK, "{host} should route");
        assert_eq!(body, "<h1>home</h1>", "{host} should route");
    }
}

#[tokio::test]
async fn paths_are_relative_to_the_app_on_its_own_hostname() {
    let f = Arc::new(Fixture::new());
    let (status, body, _) = get_with_host(f, "/sub/", "trip.local").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "<h1>sub</h1>");
}

#[tokio::test]
async fn one_app_hostname_cannot_reach_another_app() {
    // The isolation property host routing exists for: on trip.local, a path
    // that looks like another app's prefix is just a missing file, not a way in.
    let f = Arc::new(Fixture::new());
    let (status, _, _) = get_with_host(f, "/chores/index.html", "trip.local").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_renamed_app_is_reachable_at_the_name_it_actually_got() {
    // A name conflict on the network suffixes the hostname, so the announced
    // name and the slug diverge. Both have to work.
    let f = Arc::new(Fixture::new());

    let (status, _, _) = get_with_host(Arc::clone(&f), "/", "chores-2.local").await;
    assert_eq!(status, StatusCode::OK, "the announced name must resolve");

    let (status, _, _) = get_with_host(f, "/chores/", "localhost").await;
    assert_eq!(status, StatusCode::OK, "the slug prefix must still work");
}

#[tokio::test]
async fn an_unknown_host_falls_back_to_prefix_routing() {
    let f = Arc::new(Fixture::new());

    let (status, body, _) = get_with_host(Arc::clone(&f), "/", "some-mac.local").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Trip Planner"), "should be the index");

    let (status, body, _) = get_with_host(f, "/trip/", "some-mac.local").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "<h1>home</h1>");
}

#[tokio::test]
async fn traversal_is_refused_on_an_app_hostname_too() {
    let f = Arc::new(Fixture::new());

    for uri in ["/../secret.txt", "/sub/../../secret.txt"] {
        let (status, body, _) = get_with_host(Arc::clone(&f), uri, "trip.local").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri} should be refused");
        assert!(!body.contains("PRIVATE"), "{uri} leaked");
    }
}
