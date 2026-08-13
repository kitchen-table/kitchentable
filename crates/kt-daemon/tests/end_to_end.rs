//! The whole daemon, driven the way a real client drives it.
//!
//! Spawns the actual binary against a scratch workspace, then talks to it over
//! the real socket and the real HTTP port. Everything below this is unit
//! tested; this is the layer that catches wiring mistakes - a route that never
//! gets registered, a manifest that never reaches the store, a URL that is
//! right in a test and wrong on the wire.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// Long enough for a debug-build daemon to boot on a loaded CI runner.
const READY_TIMEOUT: Duration = Duration::from_secs(30);
/// The workspace watcher debounces, so give a change time to land.
const CHANGE_TIMEOUT: Duration = Duration::from_secs(15);

struct Daemon {
    child: Child,
    home: PathBuf,
    workspace: PathBuf,
    port: u16,
}

impl Daemon {
    fn start() -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEQ: AtomicU32 = AtomicU32::new(0);

        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        // Deliberately /tmp rather than std::env::temp_dir(). A Unix socket
        // path cannot exceed roughly 104 bytes, and macOS hands out temp
        // directories under /var/folders/... which blows the budget once the
        // daemon appends Library/Application Support/KitchenTable/kt.sock.
        let base = PathBuf::from("/tmp").join(format!("kt-e2e-{}-{seq}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);

        let home = base.join("home");
        let workspace = base.join("workspace");
        std::fs::create_dir_all(&workspace).expect("creates workspace");
        std::fs::create_dir_all(&home).expect("creates home");

        // A high port, because the real one is very likely taken by whatever
        // else is running on the machine. The sequence number matters: tests
        // run in parallel, so a port derived from the pid alone would have
        // every test in this binary fighting for one socket.
        let port = 18400 + (std::process::id() % 500) as u16 * 20 + seq as u16;

        let daemon = Self {
            child: spawn(&home, &workspace, port),
            home,
            workspace,
            port,
        };
        daemon.wait_until_ready();
        daemon
    }

    /// Stop the daemon and start it again on the same HOME, workspace and
    /// port - the update-and-relaunch a household actually experiences.
    fn restart(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.child = spawn(&self.home, &self.workspace, self.port);
        self.wait_until_ready();
    }

    fn socket(&self) -> PathBuf {
        kt_types::paths::socket_path(&self.home.display().to_string())
    }

    fn wait_until_ready(&self) {
        let deadline = Instant::now() + READY_TIMEOUT;
        while Instant::now() < deadline {
            if self.try_call("sys.status", None).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("the daemon never became ready");
    }

    fn try_call(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        let stream = UnixStream::connect(self.socket()).map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;

        let request = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": method, "params": params,
        });
        let mut writer = &stream;
        writer
            .write_all(format!("{request}\n").as_bytes())
            .map_err(|e| e.to_string())?;

        let mut reply = String::new();
        BufReader::new(&stream)
            .read_line(&mut reply)
            .map_err(|e| e.to_string())?;

        let value: serde_json::Value = serde_json::from_str(&reply).map_err(|e| e.to_string())?;
        if let Some(error) = value.get("error") {
            return Err(error.to_string());
        }
        Ok(value["result"].clone())
    }

    fn call(&self, method: &str, params: Option<serde_json::Value>) -> serde_json::Value {
        self.try_call(method, params)
            .unwrap_or_else(|e| panic!("{method} failed: {e}"))
    }

    fn get(&self, path: &str) -> (u16, String) {
        self.get_with_host(path, "localhost")
    }

    /// A minimal HTTP/1.1 GET. A client crate would be a dependency for this
    /// one file, and hand-writing the request keeps the Host header explicit -
    /// which is the thing under test.
    fn get_with_host(&self, path: &str, host: &str) -> (u16, String) {
        use std::net::TcpStream;

        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).expect("connects");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("sets timeout");
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
        )
        .expect("writes request");

        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut stream, &mut raw).expect("reads response");
        let text = String::from_utf8_lossy(&raw).into_owned();

        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");

        (status, body.to_string())
    }

    /// Connect from the machine's real LAN address rather than 127.0.0.1, so
    /// the request is not treated as the owner's own browser. This is the only
    /// way to exercise the gate over a real socket.
    fn get_as_stranger(&self, path: &str) -> (u16, String) {
        let (status, body, _) = self.get_as_stranger_with_cookie(path, None);
        (status, body)
    }

    /// A stranger's request carrying `cookie`, and whatever session the daemon
    /// hands back.
    ///
    /// Returning the cookie is what makes a *second* visit testable, and the
    /// second visit is where the interesting bugs live.
    fn get_as_stranger_with_cookie(
        &self,
        path: &str,
        cookie: Option<&str>,
    ) -> (u16, String, Option<String>) {
        use std::net::TcpStream;

        let Some(addr) = kt_mdns::primary_ipv4() else {
            // No network at all: skip rather than fail, so an isolated CI
            // runner does not report a false problem.
            return (0, String::new(), None);
        };

        let mut stream = TcpStream::connect((addr, self.port)).expect("connects");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("sets timeout");
        let cookie_header = match cookie {
            Some(c) => format!("Cookie: {c}\r\n"),
            None => String::new(),
        };
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {addr}\r\n{cookie_header}Connection: close\r\n\r\n"
        )
        .expect("writes request");

        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut stream, &mut raw).expect("reads response");
        let text = String::from_utf8_lossy(&raw).into_owned();
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let (headers, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));

        // Just the name=value pair, dropping Path/HttpOnly/Max-Age, which is
        // all a browser would send back.
        let minted = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("set-cookie: ")
                    .or(line.strip_prefix("Set-Cookie: "))
            })
            .map(|value| value.split(';').next().unwrap_or(value).trim().to_string());

        (status, body.to_string(), minted)
    }

    fn set_visibility(&self, folder: &str, visibility: &str) {
        let path = self.workspace.join(folder).join("app.json");
        let raw = std::fs::read_to_string(&path).expect("manifest exists");
        let mut manifest: serde_json::Value = serde_json::from_str(&raw).expect("parses");
        manifest["visibility"] = serde_json::json!(visibility);
        std::fs::write(&path, manifest.to_string()).expect("writes manifest");
    }

    fn add_app(&self, folder: &str, html: &str) {
        let dir = self.workspace.join(folder);
        std::fs::create_dir_all(&dir).expect("creates app folder");
        std::fs::write(dir.join("index.html"), html).expect("writes index.html");
    }

    /// Poll until `predicate` holds, so tests do not depend on watcher timing.
    fn wait_for(&self, what: &str, predicate: impl Fn(&serde_json::Value) -> bool) {
        let deadline = Instant::now() + CHANGE_TIMEOUT;
        while Instant::now() < deadline {
            let apps = self.call("app.list", None);
            if predicate(&apps) {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        panic!("timed out waiting for {what}");
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(base) = self.workspace.parent() {
            let _ = std::fs::remove_dir_all(base);
        }
    }
}

fn spawn(home: &Path, workspace: &Path, port: u16) -> Child {
    Command::new(daemon_binary())
        .env("HOME", home)
        .env("KT_WORKSPACE", workspace)
        .env("KT_PORTS", port.to_string())
        // Announcing on the network from a test would publish junk
        // hostnames on whatever LAN CI happens to sit on.
        .env("KT_NO_MDNS", "1")
        // The Keychain is per-user, not per-HOME, so every daemon in this
        // suite would share one session key and defeat the isolation the
        // scratch HOME above exists to provide - and on a developer's machine
        // a test run would rewrite the key their real daemon is using.
        .env("KT_NO_KEYCHAIN", "1")
        .env("RUST_LOG", "warn")
        .spawn()
        .expect("starts the daemon")
}

fn daemon_binary() -> PathBuf {
    // Cargo puts integration test binaries in target/<profile>/deps.
    let mut path = std::env::current_exe().expect("test binary path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    let binary = path.join("kt-daemon");
    assert!(
        binary.exists(),
        "kt-daemon is not built at {}; run `cargo build --workspace` first",
        binary.display()
    );
    binary
}

fn slugs(apps: &serde_json::Value) -> Vec<String> {
    apps.as_array()
        .expect("an array")
        .iter()
        .map(|a| a["slug"].as_str().expect("a slug").to_string())
        .collect()
}

#[test]
fn a_folder_dropped_in_becomes_a_served_app() {
    let daemon = Daemon::start();

    assert!(slugs(&daemon.call("app.list", None)).is_empty());

    daemon.add_app("Trip Planner", "<h1>Portugal</h1>");
    daemon.wait_for("the new app to appear", |apps| {
        slugs(apps) == ["trip-planner"]
    });

    let (status, body) = daemon.get("/trip-planner/");
    assert_eq!(status, 200);
    assert_eq!(body, "<h1>Portugal</h1>");
}

#[test]
fn removing_a_folder_stops_serving_it() {
    let daemon = Daemon::start();
    daemon.add_app("Chores", "<h1>Bins</h1>");
    daemon.wait_for("the app to appear", |apps| !slugs(apps).is_empty());

    std::fs::remove_dir_all(daemon.workspace.join("Chores")).expect("removes folder");
    daemon.wait_for("the app to go", |apps| slugs(apps).is_empty());

    let (status, _) = daemon.get("/chores/");
    assert_eq!(status, 404, "a removed app must stop being reachable");
}

#[test]
fn a_new_app_is_private_and_gets_a_manifest_written_for_it() {
    let daemon = Daemon::start();
    daemon.add_app("Notes", "<h1>Notes</h1>");
    daemon.wait_for("the app to appear", |apps| !slugs(apps).is_empty());

    let app = daemon.call("app.get", Some(serde_json::json!({ "slug": "notes" })));
    assert_eq!(
        app["visibility"], "private",
        "nothing becomes shareable just by existing"
    );

    let manifest = std::fs::read_to_string(daemon.workspace.join("Notes/app.json"))
        .expect("a manifest was written next to the content");
    assert!(manifest.contains("\"slug\": \"notes\""));
}

#[test]
fn two_folders_with_the_same_name_get_distinct_slugs() {
    let daemon = Daemon::start();
    daemon.add_app("My App", "<h1>one</h1>");
    daemon.add_app("my-app", "<h1>two</h1>");

    daemon.wait_for("both apps", |apps| slugs(apps).len() == 2);

    let mut found = slugs(&daemon.call("app.list", None));
    found.sort();
    assert_eq!(found, ["my-app", "my-app-2"]);

    // Both are actually reachable, not just listed.
    for slug in &found {
        let (status, _) = daemon.get(&format!("/{slug}/"));
        assert_eq!(status, 200, "{slug} should serve");
    }
}

#[test]
fn traversal_out_of_an_app_is_refused_over_the_wire() {
    let daemon = Daemon::start();
    daemon.add_app("Trip", "<h1>trip</h1>");
    daemon.wait_for("the app", |apps| !slugs(apps).is_empty());

    // A secret next to the workspace, the classic target.
    std::fs::write(daemon.workspace.join("secret.txt"), "PRIVATE").expect("writes");

    for path in [
        "/trip/../secret.txt",
        "/trip/../../secret.txt",
        "/trip/%2e%2e/secret.txt",
    ] {
        let (status, body) = daemon.get(path);
        assert_eq!(status, 404, "{path} should be refused");
        assert!(!body.contains("PRIVATE"), "{path} leaked a file");
    }
}

#[test]
fn sys_status_reports_the_workspace_and_protocol() {
    let daemon = Daemon::start();
    let status = daemon.call("sys.status", None);

    assert_eq!(status["protocol_version"], kt_types::PROTOCOL_VERSION);
    assert_eq!(
        status["workspace"],
        daemon.workspace.display().to_string(),
        "status should name the folder actually being watched"
    );
}

#[test]
fn an_unknown_method_is_an_error_the_connection_survives() {
    let daemon = Daemon::start();

    assert!(daemon.try_call("app.nonsense", None).is_err());
    // The daemon must still be answering afterwards.
    assert!(daemon.try_call("sys.status", None).is_ok());
}

#[test]
fn the_index_lists_every_app() {
    let daemon = Daemon::start();
    daemon.add_app("Trip Planner", "<h1>a</h1>");
    daemon.add_app("Chores Rota", "<h1>b</h1>");
    daemon.wait_for("both apps", |apps| slugs(apps).len() == 2);

    let (status, body) = daemon.get("/");
    assert_eq!(status, 200);
    assert!(body.contains("Trip Planner"));
    assert!(body.contains("Chores Rota"));
}

#[test]
fn every_spelling_of_an_app_root_serves_it() {
    let daemon = Daemon::start();
    daemon.add_app("Trip", "<h1>trip</h1>");
    daemon.wait_for("the app", |apps| !slugs(apps).is_empty());

    for path in ["/trip", "/trip/", "/trip/index.html"] {
        let (status, body) = daemon.get(path);
        assert_eq!(status, 200, "{path} should serve");
        assert_eq!(body, "<h1>trip</h1>", "{path} should serve the entry point");
    }
}

// ---- the gate, over a real socket ----

#[test]
fn a_private_app_is_refused_to_anyone_but_the_owner() {
    let daemon = Daemon::start();
    daemon.add_app("Diary", "<h1>secret</h1>");
    daemon.wait_for("the app", |apps| !slugs(apps).is_empty());

    // The owner's own browser: always allowed, no pairing with itself.
    let (status, body) = daemon.get("/diary/");
    assert_eq!(status, 200);
    assert_eq!(body, "<h1>secret</h1>");

    // Anyone else on the network: refused, and the content never appears.
    let (status, body) = daemon.get_as_stranger("/diary/");
    if status != 0 {
        assert_eq!(
            status, 403,
            "a private app must not be served to the network"
        );
        assert!(!body.contains("secret"), "private content leaked");
    }
}

#[test]
fn a_household_app_opens_for_the_network() {
    let daemon = Daemon::start();
    daemon.add_app("Chores", "<h1>bins</h1>");
    daemon.wait_for("the app", |apps| !slugs(apps).is_empty());

    daemon.set_visibility("Chores", "network");
    daemon.wait_for("the level to change", |apps| {
        apps[0]["visibility"] == "network"
    });

    let (status, body) = daemon.get_as_stranger("/chores/");
    if status != 0 {
        assert_eq!(
            status, 200,
            "a household app should open on the home network"
        );
        assert_eq!(body, "<h1>bins</h1>");
    }
}

#[test]
fn an_invited_app_shows_the_wait_page_rather_than_refusing() {
    let daemon = Daemon::start();
    daemon.add_app("Trip", "<h1>portugal</h1>");
    daemon.wait_for("the app", |apps| !slugs(apps).is_empty());

    daemon.set_visibility("Trip", "invited");
    daemon.wait_for("the level to change", |apps| {
        apps[0]["visibility"] == "invited"
    });

    let (status, body) = daemon.get_as_stranger("/trip/");
    if status != 0 {
        assert_eq!(
            status, 200,
            "an unknown device waits, it is not turned away"
        );
        assert!(body.contains("Waiting for"), "should be the wait page");
        assert!(
            !body.contains("portugal"),
            "the app must not leak while waiting"
        );
    }
}

#[test]
fn a_nonsense_invite_link_gets_an_explanation_not_a_stack_trace() {
    let daemon = Daemon::start();

    let (status, body) = daemon.get("/i/not-a-real-token");
    assert_eq!(status, 403);
    assert!(
        body.contains("will not open"),
        "should explain, not just refuse"
    );
}

// ---- authoring over the socket ------------------------------------------
//
// The window's "Add an app" runs through these. Every one of them is checked
// against the real binary rather than the dispatcher, because the thing that
// makes them work is a rescan happening before the reply is written - and a
// unit test that calls the handler once cannot tell whether it did.

#[test]
fn app_create_makes_a_folder_and_answers_with_the_app() {
    let daemon = Daemon::start();

    let app = daemon.call(
        "app.create",
        Some(serde_json::json!({ "name": "Packing List" })),
    );

    assert_eq!(app["slug"], "packing-list");
    assert_eq!(app["name"], "Packing List");
    // Private, like every app that has never been shared.
    assert_eq!(app["visibility"], "private");

    // Answered from the library, not from a guess: it is listed already.
    let apps = daemon.call("app.list", None);
    assert!(slugs(&apps).contains(&"packing-list".to_string()));
}

#[test]
fn a_created_app_is_served_immediately() {
    // The reply arriving before the app is reachable would be a lie the user
    // finds out about by tapping the link.
    let daemon = Daemon::start();
    daemon.call("app.create", Some(serde_json::json!({ "name": "Notes" })));

    let (status, body) = daemon.get("/notes/");
    assert_eq!(status, 200);
    assert!(
        body.contains("<title>Notes</title>"),
        "the starter page is there"
    );
}

#[test]
fn creating_two_apps_with_one_name_does_not_clobber_the_first() {
    let daemon = Daemon::start();

    let first = daemon.call("app.create", Some(serde_json::json!({ "name": "Notes" })));
    let second = daemon.call("app.create", Some(serde_json::json!({ "name": "Notes" })));

    assert_eq!(first["slug"], "notes");
    assert_ne!(second["slug"], first["slug"], "the second got its own slug");

    let apps = daemon.call("app.list", None);
    assert_eq!(apps.as_array().expect("array").len(), 2);
}

#[test]
fn a_name_that_would_escape_the_workspace_stays_inside_it() {
    let daemon = Daemon::start();

    let app = daemon.call(
        "app.create",
        Some(serde_json::json!({ "name": "../../../etc/evil" })),
    );

    let path = std::path::Path::new(app["path"].as_str().expect("path"));
    let workspace = daemon
        .workspace
        .canonicalize()
        .expect("workspace canonicalises");
    assert!(
        path.canonicalize()
            .expect("app canonicalises")
            .starts_with(&workspace),
        "an app must never be created outside the workspace"
    );
}

#[test]
fn app_create_refuses_an_empty_name_rather_than_inventing_one() {
    let daemon = Daemon::start();

    // `try_call` hands back the error object as text.
    let error = daemon
        .try_call("app.create", Some(serde_json::json!({ "name": "   " })))
        .expect_err("should be refused");
    assert!(
        error.contains("bad_request"),
        "should be the caller's fault: {error}"
    );
    assert!(
        error.contains("needs a name"),
        "should say what is wrong: {error}"
    );
}

#[test]
fn app_import_copies_a_folder_in_and_leaves_the_original_alone() {
    let daemon = Daemon::start();

    // Somewhere outside the workspace, as a real drop from Finder would be.
    let outside = std::env::temp_dir().join(format!("kt-import-{}", std::process::id()));
    std::fs::create_dir_all(outside.join("Trip Planner/assets")).expect("creates");
    std::fs::write(outside.join("Trip Planner/index.html"), "<h1>lisbon</h1>").expect("writes");
    std::fs::write(outside.join("Trip Planner/assets/a.css"), "body{}").expect("writes");

    let app = daemon.call(
        "app.import",
        Some(serde_json::json!({ "path": outside.join("Trip Planner").display().to_string() })),
    );

    assert_eq!(app["slug"], "trip-planner");

    let (status, body) = daemon.get("/trip-planner/");
    assert_eq!(status, 200);
    assert!(body.contains("lisbon"), "the copied content is served");

    assert!(
        outside.join("Trip Planner/index.html").exists(),
        "importing copies; it does not move someone's folder out from under them"
    );

    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn app_import_refuses_a_folder_already_in_the_workspace() {
    // It is already an app. Copying it would quietly duplicate it.
    let daemon = Daemon::start();
    daemon.add_app("Chores", "<h1>bins</h1>");
    daemon.wait_for("chores to appear", |apps| {
        slugs(apps).contains(&"chores".to_string())
    });

    let error = daemon
        .try_call(
            "app.import",
            Some(serde_json::json!({
                "path": daemon.workspace.join("Chores").display().to_string()
            })),
        )
        .expect_err("should be refused");

    assert!(
        error.contains("bad_request"),
        "should be the caller's fault: {error}"
    );
    assert!(
        error.contains("already in your workspace"),
        "the message should say why: {error}"
    );
}

// ---- approving a device -------------------------------------------------
//
// The flagship flow, and the one HANDOFF says to run by hand before believing
// any auth change: mint a link, open it, open it again, approve, refresh.
// These cover the approving half, which until now had no caller but a human
// with a socket client.

#[test]
fn approving_a_waiting_device_lets_it_in_and_names_it() {
    let daemon = Daemon::start();
    daemon.add_app("Portugal", "<h1>portugal</h1>");
    daemon.wait_for("the app to appear", |apps| {
        slugs(apps).contains(&"portugal".to_string())
    });
    daemon.set_visibility("Portugal", "invited");
    daemon.wait_for("invited to stick", |apps| {
        apps.as_array()
            .is_some_and(|a| a.iter().any(|app| app["visibility"] == "invited"))
    });

    // A stranger opens it: refused politely, and remembered as pending.
    let (status, body) = daemon.get_as_stranger("/portugal/");
    if status == 0 {
        return; // no network on this runner
    }
    assert!(body.contains("Waiting for"), "should be the wait page");

    let devices = daemon.call("device.list", None);
    let waiting: Vec<_> = devices
        .as_array()
        .expect("array")
        .iter()
        .filter(|d| d["status"] == "pending")
        .collect();
    assert_eq!(
        waiting.len(),
        1,
        "the stranger should be waiting: {devices}"
    );
    let id = waiting[0]["id"].as_str().expect("id").to_string();

    // The owner approves, naming the device in the same call.
    let approved = daemon.call(
        "device.approve",
        Some(serde_json::json!({ "id": id, "name": "Kitchen iPad" })),
    );
    assert_eq!(approved["status"], "approved");

    let devices = daemon.call("device.list", None);
    let device = devices
        .as_array()
        .expect("array")
        .iter()
        .find(|d| d["id"] == id.as_str())
        .expect("still listed");
    assert_eq!(device["name"], "Kitchen iPad", "approving also named it");
    assert_eq!(device["status"], "approved");
}

#[test]
fn an_approved_device_stays_approved_across_a_restart() {
    // The session key used to be generated at every start, so an update - or
    // a crash, or a laptop lid - silently invalidated every session in the
    // house and asked everyone to pair again. Unit tests could not see it:
    // it takes a second process lifetime to notice.
    let mut daemon = Daemon::start();
    daemon.add_app("Portugal", "<h1>portugal</h1>");
    daemon.wait_for("the app to appear", |apps| {
        slugs(apps).contains(&"portugal".to_string())
    });
    daemon.set_visibility("Portugal", "invited");
    daemon.wait_for("invited to stick", |apps| {
        apps.as_array()
            .is_some_and(|a| a.iter().any(|app| app["visibility"] == "invited"))
    });

    let (status, _, cookie) = daemon.get_as_stranger_with_cookie("/portugal/", None);
    if status == 0 {
        return; // no network on this runner
    }
    let cookie = cookie.expect("the wait page mints a session");

    let devices = daemon.call("device.list", None);
    let id = devices
        .as_array()
        .expect("array")
        .iter()
        .find(|d| d["status"] == "pending")
        .and_then(|d| d["id"].as_str())
        .expect("the stranger is waiting")
        .to_string();
    daemon.call("device.approve", Some(serde_json::json!({ "id": id })));

    // Note what is *not* asserted here: an invited app answers 200 either way,
    // because the wait page is a courtesy rather than a refusal. The body is
    // the only thing that tells being let in apart from being asked to wait.
    let (status, body, _) = daemon.get_as_stranger_with_cookie("/portugal/", Some(&cookie));
    assert_eq!(status, 200);
    assert!(body.contains("portugal"), "an approved device gets the app");

    daemon.restart();
    daemon.wait_for("the app to come back", |apps| {
        slugs(apps).contains(&"portugal".to_string())
    });

    // The same cookie, against a daemon that has been through a whole process
    // lifetime since minting it.
    let (_, body, _) = daemon.get_as_stranger_with_cookie("/portugal/", Some(&cookie));
    assert!(
        !body.contains("Waiting for"),
        "the restart sent an approved device back to the wait page"
    );
    assert!(
        body.contains("portugal"),
        "the session did not survive the restart"
    );

    // And they are still the same device, rather than a second stranger the
    // owner has to approve all over again.
    let devices = daemon.call("device.list", None);
    let after = devices.as_array().expect("array");
    assert_eq!(after.len(), 1, "no duplicate device was created: {devices}");
    assert_eq!(after[0]["id"], id.as_str());
    assert_eq!(after[0]["status"], "approved");
}

#[test]
fn a_device_decision_is_written_to_the_access_log() {
    // Who gets in is exactly what the Activity view exists to show.
    let daemon = Daemon::start();
    daemon.add_app("Portugal", "<h1>portugal</h1>");
    daemon.wait_for("the app to appear", |apps| {
        slugs(apps).contains(&"portugal".to_string())
    });
    daemon.set_visibility("Portugal", "invited");

    let (status, _) = daemon.get_as_stranger("/portugal/");
    if status == 0 {
        return;
    }

    let devices = daemon.call("device.list", None);
    let Some(id) = devices
        .as_array()
        .expect("array")
        .iter()
        .find(|d| d["status"] == "pending")
        .map(|d| d["id"].as_str().expect("id").to_string())
    else {
        return;
    };

    daemon.call("device.deny", Some(serde_json::json!({ "id": id })));

    let events = daemon.call("log.query", Some(serde_json::json!({ "limit": 20 })));
    assert!(
        events
            .as_array()
            .expect("array")
            .iter()
            .any(|e| e["action"] == "denied" && e["device_id"] == id.as_str()),
        "the refusal should be in the log: {events}"
    );
}

#[test]
fn denying_and_revoking_are_told_apart_in_the_log() {
    // They land on the same stored status, so the log is the only place the
    // difference survives.
    let daemon = Daemon::start();
    daemon.add_app("Portugal", "<h1>portugal</h1>");
    daemon.wait_for("the app to appear", |apps| {
        slugs(apps).contains(&"portugal".to_string())
    });
    daemon.set_visibility("Portugal", "invited");

    let (status, _) = daemon.get_as_stranger("/portugal/");
    if status == 0 {
        return;
    }
    let devices = daemon.call("device.list", None);
    let Some(id) = devices
        .as_array()
        .expect("array")
        .iter()
        .find(|d| d["status"] == "pending")
        .map(|d| d["id"].as_str().expect("id").to_string())
    else {
        return;
    };

    daemon.call("device.approve", Some(serde_json::json!({ "id": id })));
    daemon.call("device.revoke", Some(serde_json::json!({ "id": id })));

    let events = daemon.call("log.query", Some(serde_json::json!({ "limit": 20 })));
    let actions: Vec<&str> = events
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|e| e["action"].as_str())
        .collect();

    assert!(actions.contains(&"paired"), "approval logged: {actions:?}");
    assert!(
        actions.contains(&"revoked"),
        "revocation logged: {actions:?}"
    );
    assert!(
        !actions.contains(&"denied"),
        "this was not a denial: {actions:?}"
    );
}

#[test]
fn approving_an_unknown_device_is_an_error_not_a_silent_success() {
    let daemon = Daemon::start();

    let error = daemon
        .try_call(
            "device.approve",
            Some(serde_json::json!({ "id": "AAAAAAAAAAAAAAAAAAAAAA" })),
        )
        .expect_err("should be refused");
    assert!(
        error.contains("not_found") || error.contains("bad_request"),
        "{error}"
    );
}
