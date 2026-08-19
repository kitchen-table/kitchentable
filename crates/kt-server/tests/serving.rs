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

/// Whether the response is `html`, as served.
///
/// Not equality: an HTML document leaves here with one script tag appended, so
/// the page can tell the owner it is still open. That is the only edit made to
/// anyone's file, and this is the assertion that would catch anything else
/// starting to rewrite people's pages.
fn served(body: &str, html: &str) -> bool {
    body.starts_with(html) && body.contains("/__kt/live.js") && body.len() < html.len() + 200
}

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

        // A folder of photos and nothing else: no page anywhere in it. This is
        // the shape that used to 404 at its own front door.
        let album = dir.join("album");
        std::fs::create_dir_all(album.join("summer")).expect("creates dirs");
        std::fs::write(album.join("one.png"), "PNG").expect("writes");
        std::fs::write(album.join("two.jpg"), "JPG").expect("writes");
        std::fs::write(album.join("summer/beach.png"), "PNG").expect("writes");

        Self {
            apps: vec![
                ServedApp {
                    relay: Default::default(),
                    storage: Default::default(),
                    storage_backup: true,
                    slug: "trip".into(),
                    name: "Trip Planner".into(),
                    root: root.canonicalize().expect("canonicalises"),
                    entry: "index.html".into(),
                    visibility: kt_types::Visibility::Public,
                    paused: false,
                },
                ServedApp {
                    relay: Default::default(),
                    storage: Default::default(),
                    storage_backup: true,
                    slug: "chores".into(),
                    name: "Chores Rota".into(),
                    root: chores.canonicalize().expect("canonicalises"),
                    entry: "index.html".into(),
                    visibility: kt_types::Visibility::Public,
                    paused: false,
                },
                ServedApp {
                    relay: Default::default(),
                    storage: Default::default(),
                    storage_backup: true,
                    slug: "album".into(),
                    name: "Album".into(),
                    root: album.canonicalize().expect("canonicalises"),
                    // The entry the registry defaults to when it cannot choose.
                    // Nothing by this name exists in the folder.
                    entry: "index.html".into(),
                    visibility: kt_types::Visibility::Public,
                    paused: false,
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

async fn get_with_host(
    fixture: Arc<Fixture>,
    uri: &str,
    host: &str,
) -> (StatusCode, String, Option<String>) {
    let response = router(
        fixture,
        Arc::new(AllowAll),
        Arc::new(kt_server::Presence::new()),
    )
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
        assert!(
            served(&body, "<h1>home</h1>"),
            "{uri} should be the entry point, got {body}"
        );
    }
}

#[tokio::test]
async fn a_subdirectory_serves_its_own_entry_point() {
    let f = Arc::new(Fixture::new());
    let (status, body, _) = get(f, "/trip/sub/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(served(&body, "<h1>sub</h1>"), "got {body}");
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
    assert!(
        served(&body, "<h1>home</h1>"),
        "the app owns its own origin, got {body}"
    );
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
        assert!(
            served(&body, "<h1>home</h1>"),
            "{host} should route, got {body}"
        );
    }
}

#[tokio::test]
async fn paths_are_relative_to_the_app_on_its_own_hostname() {
    let f = Arc::new(Fixture::new());
    let (status, body, _) = get_with_host(f, "/sub/", "trip.local").await;

    assert_eq!(status, StatusCode::OK);
    assert!(served(&body, "<h1>sub</h1>"), "got {body}");
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
    assert!(served(&body, "<h1>home</h1>"), "got {body}");
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

// ---- folders with no page of their own -------------------------------------

#[tokio::test]
async fn a_folder_of_photos_opens_on_a_listing_instead_of_404() {
    let fixture = Arc::new(Fixture::new());
    let (status, body, _) = get(fixture, "/album/").await;

    assert_eq!(status, StatusCode::OK, "this used to be the 404 nobody saw");
    assert!(body.contains("one.png"), "the photos are listed: {body}");
    assert!(body.contains("two.jpg"));
    assert!(body.contains("class=\"grid\""), "all images, so a grid");
    assert!(body.contains("Album"), "titled with the app's name");
}

#[tokio::test]
async fn the_listing_is_a_live_page_like_any_other() {
    let fixture = Arc::new(Fixture::new());
    let (_, body, _) = get(fixture, "/album/").await;
    // Dropping a real index.html in should make the open tab notice, and that
    // only works if the generated page carries the live tag too.
    assert!(body.contains("/__kt/live.js"), "the listing reloads like a page");
}

#[tokio::test]
async fn a_subfolder_of_a_pageless_app_lists_too() {
    let fixture = Arc::new(Fixture::new());
    let (status, body, _) = get(fixture, "/album/summer/").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("beach.png"));
    assert!(body.contains("summer"), "it says where you are");
}

#[tokio::test]
async fn an_app_with_a_real_page_never_becomes_browsable() {
    let fixture = Arc::new(Fixture::new());
    // `trip` has an index.html at its root, so `sub/` follows its own entry...
    let (status, body, _) = get(Arc::clone(&fixture), "/trip/sub/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(served(&body, "<h1>sub</h1>"), "the page, not a listing");
    assert!(!body.contains("class=\"grid\""));
    assert!(!body.contains("style.css"), "and its assets are not enumerated");
}

#[tokio::test]
async fn a_missing_directory_is_still_a_flat_404() {
    let fixture = Arc::new(Fixture::new());
    // Listing exists for folders that are really there. Inventing one for a
    // path that is not would answer a question the 404 refuses to answer.
    let (status, _, _) = get(Arc::clone(&fixture), "/album/nope/").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _, _) = get(fixture, "/album/../").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "and traversal is unchanged");
}
