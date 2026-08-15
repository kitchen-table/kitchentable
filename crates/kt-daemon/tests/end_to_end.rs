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
        Self::start_with(false)
    }

    /// `own_address` on means the gate treats this machine's LAN address as
    /// the owner, which is what a real daemon does and what makes a stranger
    /// unplayable from one machine. Off for every test that needs a stranger.
    fn start_with(own_address: bool) -> Self {
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
            child: spawn(&home, &workspace, port, own_address),
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
        self.child = spawn(&self.home, &self.workspace, self.port, false);
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

    /// Connect from the machine's real LAN address rather than 127.0.0.1.
    ///
    /// This is the only way to exercise the gate over a real socket, and it is
    /// a stranger **only because `Daemon::start` sets `KT_NO_OWN_ADDRESS`**. A
    /// real daemon counts its own address as the owner - that is how the
    /// owner's browser arrives when it follows a `.local` name - so without
    /// that switch this connects as the owner and every refusal below stops
    /// meaning what its name says. On a daemon started with the exemption on,
    /// this is the owner.
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

    /// Subscribe on a connection of its own and hand back the reader.
    ///
    /// Its own connection because that is how the shell uses it: one long-lived
    /// subscriber alongside ordinary request/response traffic. Sharing a
    /// connection would let an event land between a request and its answer,
    /// which is exactly the interleaving this needs to prove does not happen.
    fn subscribe(&self) -> BufReader<UnixStream> {
        let stream = UnixStream::connect(self.socket()).expect("connects");
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .expect("sets timeout");

        let mut writer = &stream;
        writer
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"event.subscribe\"}\n")
            .expect("subscribes");

        let mut reader = BufReader::new(stream);
        let mut ack = String::new();
        reader
            .read_line(&mut ack)
            .expect("reads the acknowledgement");
        assert!(ack.contains("subscribed"), "expected an ack, got {ack}");
        reader
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

fn spawn(home: &Path, workspace: &Path, port: u16, own_address: bool) -> Child {
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
        // The gate treats an address this machine holds as the owner's own
        // machine, because that is how the owner's browser arrives when it
        // follows a `.local` name. This suite's only way of playing a stranger
        // is to connect to that same address from this same machine, so with
        // the exemption on there is no stranger left to play. Turning it off
        // narrows the gate to loopback - it cannot grant anything - and keeps
        // every refusal below meaning what its name says. The owner's side of
        // it is covered by `the_owner_opens_a_private_app_at_its_local_name`,
        // which starts a daemon without this.
        .envs((!own_address).then_some(("KT_NO_OWN_ADDRESS", "1")))
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

/// Whether the response is `html`, as served.
///
/// Not equality: Kitchen Table adds one script tag to HTML documents on the way
/// out, so the page can say it is still open. That is the only edit it makes,
/// so the test is that the app's own markup arrived intact and the tag came
/// with it - which is also the assertion that would fail if anything else ever
/// started rewriting people's pages.
fn served(body: &str, html: &str) -> bool {
    body.starts_with(html) && body.contains("/__kt/live.js") && body.len() < html.len() + 200
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
    assert!(served(&body, "<h1>Portugal</h1>"), "got {body}");
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
        assert!(
            served(&body, "<h1>trip</h1>"),
            "{path} should serve the entry point, got {body}"
        );
    }
}

#[test]
fn a_single_file_becomes_an_app_that_serves_it() {
    // The window offered to host "HTML, CSS, JS, PDFs, images" while every
    // import path refused anything but a directory. This is the whole way
    // through: a file arrives over the socket, the registry picks it as the
    // entry, and the server hands it back at the app's root.
    let daemon = Daemon::start();

    let outside = daemon.workspace.parent().expect("parent").join("elsewhere");
    std::fs::create_dir_all(&outside).expect("creates");
    let file = outside.join("Sunday Menu.pdf");
    std::fs::write(&file, "%PDF-1.4 pretend").expect("writes");

    let made = daemon.call(
        "app.import",
        Some(serde_json::json!({ "path": file.display().to_string() })),
    );

    // Named after the file without its extension, and opening on the file.
    assert_eq!(made["name"], "Sunday Menu");
    assert_eq!(made["entry"], "Sunday Menu.pdf");
    assert_eq!(made["entry_exists"], true, "not a broken app: {made}");

    daemon.wait_for("the app to appear", |apps| {
        slugs(apps).contains(&"sunday-menu".to_string())
    });

    let (status, body) = daemon.get("/sunday-menu/");
    assert_eq!(status, 200, "the file should be served at the app's root");
    assert!(body.contains("%PDF-1.4"), "got {body}");

    // The original is where they left it. This is a copy, and the window says so.
    assert!(file.exists(), "the source file is untouched");
}

// ---- the gate, over a real socket ----

#[test]
fn the_owner_opens_a_private_app_at_its_local_name() {
    // Found by an owner pressing Open on their own Private app and being told
    // "Not available" on their own Mac. `localhost` counted as this machine and
    // the `.local` name did not, because Bonjour resolves it to this machine's
    // LAN address - so the same browser, two inches from the files, was
    // refused.
    //
    // This is the only test in the file that runs with the own-address
    // exemption on, and it is the reason every other one turns it off: with it
    // on there is no stranger left for this suite to play, because its only way
    // of being one is to connect to this machine's address from this machine.
    // The refusal side lives next door, and in kt-server's unit tests where a
    // neighbour's address can actually be spelled out.
    let daemon = Daemon::start_with(true);
    daemon.add_app("Diary", "<h1>secret</h1>");
    daemon.wait_for("the app", |apps| !slugs(apps).is_empty());

    // Loopback has always worked and still must.
    let (loopback, _) = daemon.get("/diary/");
    assert_eq!(loopback, 200, "a private app must open on loopback");

    // And now so does the address the window prints and the QR encodes.
    let (own, body) = daemon.get_as_stranger("/diary/");
    if own == 0 {
        return; // no network on this runner
    }
    assert_eq!(
        own, 200,
        "a private app must open for the machine it lives on: {body}"
    );
}

#[test]
fn a_private_app_is_refused_to_anyone_but_the_owner() {
    let daemon = Daemon::start();
    daemon.add_app("Diary", "<h1>secret</h1>");
    daemon.wait_for("the app", |apps| !slugs(apps).is_empty());

    // The owner's own browser: always allowed, no pairing with itself.
    let (status, body) = daemon.get("/diary/");
    assert_eq!(status, 200);
    assert!(served(&body, "<h1>secret</h1>"), "got {body}");

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
        assert!(served(&body, "<h1>bins</h1>"), "got {body}");
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

/// Read events until `wanted` shows up, or give up.
///
/// By name rather than by position: the daemon is free to say other true things
/// first, and a test that breaks when it does would be testing the order rather
/// than the event.
fn wait_for_event(reader: &mut BufReader<UnixStream>, wanted: &str) -> serde_json::Value {
    let deadline = Instant::now() + CHANGE_TIMEOUT;
    let mut seen = Vec::new();

    while Instant::now() < deadline {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let value: serde_json::Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let params = value["params"].clone();
        if params["event"] == wanted {
            return params;
        }
        seen.push(params["event"].as_str().unwrap_or("?").to_string());
    }
    panic!("never saw {wanted}; saw {seen:?}");
}

#[test]
fn a_subscriber_is_told_when_an_app_appears_and_when_it_leaves() {
    // The window polled every two seconds before this existed. The point is
    // not only that the event arrives but that it arrives without anybody
    // asking, so this never calls app.list.
    let daemon = Daemon::start();
    let mut events = daemon.subscribe();

    daemon.add_app("Portugal", "<h1>portugal</h1>");
    let appeared = wait_for_event(&mut events, "app_changed");
    assert_eq!(appeared["app"]["slug"], "portugal");
    assert_eq!(
        appeared["app"]["url"],
        format!("http://localhost:{}/portugal", daemon.port),
        "the event carries the app as the window would show it, URL and all: {appeared}"
    );

    std::fs::remove_dir_all(daemon.workspace.join("Portugal")).expect("removes the folder");
    let gone = wait_for_event(&mut events, "app_removed");
    assert_eq!(gone["slug"], "portugal");
}

#[test]
fn only_the_app_that_changed_is_announced() {
    // A scan sees the whole workspace. Restating all of it on every change
    // would have the window refetching twenty apps because one of them moved,
    // so the events describe the difference rather than the contents.
    let daemon = Daemon::start();
    daemon.add_app("Portugal", "<h1>portugal</h1>");
    daemon.wait_for("the first app", |apps| {
        slugs(apps).contains(&"portugal".to_string())
    });

    let mut events = daemon.subscribe();
    daemon.add_app("Iceland", "<h1>iceland</h1>");
    daemon.wait_for("the second app", |apps| {
        slugs(apps).contains(&"iceland".to_string())
    });

    // The next app_changed must be the new one. Portugal has not been touched,
    // so hearing about it here would mean the whole library was restated.
    let announced = wait_for_event(&mut events, "app_changed");
    assert_eq!(
        announced["app"]["slug"], "iceland",
        "expected only the new app to be announced: {announced}"
    );
}

#[test]
fn a_waiting_device_announces_itself_rather_than_waiting_to_be_noticed() {
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

    let mut events = daemon.subscribe();
    let (status, _) = daemon.get_as_stranger("/portugal/");
    if status == 0 {
        return; // no network on this runner
    }

    let asked = wait_for_event(&mut events, "pairing_request");
    let id = asked["device_id"].as_str().expect("carries the device");

    let devices = daemon.call("device.list", None);
    assert!(
        devices
            .as_array()
            .expect("array")
            .iter()
            .any(|d| d["id"] == id && d["status"] == "pending"),
        "the event named a device that is really waiting: {devices}"
    );
}

#[test]
fn a_page_that_checks_in_shows_up_as_reading_the_app() {
    // The whole reason the script exists. Serving static files leaves nothing
    // to observe, so this is the only path by which the owner's window can
    // know somebody has an app open - and it has to work over real HTTP,
    // through the real gate, with the real routes.
    let daemon = Daemon::start();
    daemon.add_app("Portugal", "<html><body><h1>portugal</h1></body></html>");
    daemon.wait_for("the app to appear", |apps| {
        slugs(apps).contains(&"portugal".to_string())
    });

    // Nobody has said anything yet, so nobody is reading it.
    let empty = daemon.call(
        "presence.list",
        Some(serde_json::json!({ "slug": "portugal" })),
    );
    assert_eq!(empty.as_array().expect("array").len(), 0);

    // The page arrives with the tag that does the checking in.
    let (status, body) = daemon.get("/portugal/");
    assert_eq!(status, 200);
    assert!(body.contains("/__kt/live.js?app=portugal"), "got {body}");

    // And the script itself is really served, rather than 404ing into a page
    // that then silently never reports anything.
    let (status, script) = daemon.get("/__kt/live.js?app=portugal");
    assert_eq!(status, 200, "the live-view client must be reachable");
    assert!(script.contains("__kt/beat"), "got {script}");

    // Now do what the script does.
    let (status, _) = daemon.get("/__kt/beat?app=portugal&path=/budget");
    assert_eq!(status, 204);

    let viewers = daemon.call(
        "presence.list",
        Some(serde_json::json!({ "slug": "portugal" })),
    );
    let viewers = viewers.as_array().expect("array");
    assert_eq!(viewers.len(), 1, "somebody is reading it: {viewers:?}");
    assert_eq!(viewers[0]["path"], "/budget", "and the page they are on");

    // Closing the tab empties the list rather than leaving a name up until it
    // ages out.
    let (status, _) = daemon.get("/__kt/gone?app=portugal");
    assert_eq!(status, 204);
    let after = daemon.call(
        "presence.list",
        Some(serde_json::json!({ "slug": "portugal" })),
    );
    assert_eq!(after.as_array().expect("array").len(), 0, "{after}");
}

#[test]
fn checking_in_reaches_subscribers_without_being_asked() {
    let daemon = Daemon::start();
    daemon.add_app("Portugal", "<html><body><h1>portugal</h1></body></html>");
    daemon.wait_for("the app to appear", |apps| {
        slugs(apps).contains(&"portugal".to_string())
    });

    let mut events = daemon.subscribe();
    daemon.get("/__kt/beat?app=portugal&path=/");

    let live = wait_for_event(&mut events, "presence");
    assert_eq!(live["app_slug"], "portugal");
    assert_eq!(live["viewers"].as_array().expect("array").len(), 1);
}

#[test]
fn an_asset_is_served_exactly_as_it_is_on_disk() {
    // The script goes into HTML documents and nothing else. A stylesheet or a
    // script file that came back with markup appended would be corrupt.
    let daemon = Daemon::start();
    daemon.add_app("Portugal", "<html><body><h1>portugal</h1></body></html>");
    std::fs::write(
        daemon.workspace.join("Portugal").join("app.css"),
        "body{color:red}",
    )
    .expect("writes a stylesheet");
    daemon.wait_for("the app to appear", |apps| {
        slugs(apps).contains(&"portugal".to_string())
    });

    let (status, body) = daemon.get("/portugal/app.css");
    assert_eq!(status, 200);
    assert_eq!(body, "body{color:red}", "assets go out untouched");
}

#[test]
fn pausing_takes_an_app_offline_for_everyone_and_resuming_brings_it_back() {
    let daemon = Daemon::start();
    daemon.add_app("Portugal", "<html><body><h1>portugal</h1></body></html>");
    daemon.wait_for("the app to appear", |apps| {
        slugs(apps).contains(&"portugal".to_string())
    });

    let (status, body) = daemon.get("/portugal/");
    assert_eq!(status, 200);
    assert!(served(&body, "<html><body><h1>portugal</h1>"), "got {body}");

    daemon.call("app.pause", Some(serde_json::json!({ "slug": "portugal" })));

    // Including the owner, on this machine. "Take it offline" is not a
    // visibility level with an exception for the person who pressed it.
    let (status, body) = daemon.get("/portugal/");
    assert_eq!(status, 503, "a paused app is unavailable, not forbidden");
    assert!(body.contains("Paused"), "got {body}");
    assert!(
        !body.contains("<h1>portugal</h1>"),
        "content leaked: {body}"
    );

    // It is still in the library, with its links and devices intact - that is
    // the whole difference between pausing and forgetting.
    let apps = daemon.call("app.list", None);
    assert!(slugs(&apps).contains(&"portugal".to_string()));
    assert!(
        apps.as_array()
            .expect("array")
            .iter()
            .any(|a| a["slug"] == "portugal" && a["paused"] == true),
        "the window needs to be able to see it is paused: {apps}"
    );

    daemon.call(
        "app.resume",
        Some(serde_json::json!({ "slug": "portugal" })),
    );
    let (status, body) = daemon.get("/portugal/");
    assert_eq!(status, 200, "resuming brings it back");
    assert!(served(&body, "<html><body><h1>portugal</h1>"), "got {body}");
}

#[test]
fn pausing_survives_a_restart() {
    // It is written to the manifest, not just held in memory, so an app taken
    // offline stays offline through an update.
    let mut daemon = Daemon::start();
    daemon.add_app("Portugal", "<html><body><h1>portugal</h1></body></html>");
    daemon.wait_for("the app to appear", |apps| {
        slugs(apps).contains(&"portugal".to_string())
    });
    daemon.call("app.pause", Some(serde_json::json!({ "slug": "portugal" })));

    daemon.restart();
    daemon.wait_for("the app to come back", |apps| {
        slugs(apps).contains(&"portugal".to_string())
    });

    let (status, _) = daemon.get("/portugal/");
    assert_eq!(status, 503, "still paused after a restart");
}

#[test]
fn forgetting_an_app_leaves_the_folder_alone_and_the_watcher_does_not_re_add_it() {
    // The trap this exists for: the workspace watcher's whole job is to notice
    // folders and register them, so simply dropping the row would have the app
    // back within a second.
    let daemon = Daemon::start();
    daemon.add_app("Portugal", "<html><body><h1>portugal</h1></body></html>");
    daemon.wait_for("the app to appear", |apps| {
        slugs(apps).contains(&"portugal".to_string())
    });

    daemon.call(
        "app.forget",
        Some(serde_json::json!({ "slug": "portugal" })),
    );

    daemon.wait_for("the app to leave the library", |apps| {
        !slugs(apps).contains(&"portugal".to_string())
    });

    // The promise the window makes.
    let folder = daemon.workspace.join("Portugal");
    assert!(
        folder.join("index.html").exists(),
        "the folder is untouched"
    );

    // Poke the workspace so the watcher runs again, and confirm it stays gone.
    std::fs::write(folder.join("notes.txt"), "still here").expect("writes");
    std::thread::sleep(Duration::from_secs(3));
    let apps = daemon.call("app.list", None);
    assert!(
        !slugs(&apps).contains(&"portugal".to_string()),
        "a forgotten app came back on the next scan: {apps}"
    );

    let (status, _) = daemon.get("/portugal/");
    assert_eq!(status, 404, "and it is not served either");
}

#[test]
fn forgetting_survives_a_restart() {
    let mut daemon = Daemon::start();
    daemon.add_app("Portugal", "<html><body><h1>portugal</h1></body></html>");
    daemon.wait_for("the app to appear", |apps| {
        slugs(apps).contains(&"portugal".to_string())
    });
    daemon.call(
        "app.forget",
        Some(serde_json::json!({ "slug": "portugal" })),
    );
    daemon.wait_for("the app to leave", |apps| {
        !slugs(apps).contains(&"portugal".to_string())
    });

    daemon.restart();
    std::thread::sleep(Duration::from_secs(2));

    let apps = daemon.call("app.list", None);
    assert!(
        !slugs(&apps).contains(&"portugal".to_string()),
        "a forgotten app came back after a restart: {apps}"
    );
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
fn forgetting_a_device_shortens_the_list_and_lets_that_browser_ask_again() {
    // The device list only ever grew: every lost cookie, private window and
    // rotated session key left a row, and this machine reached thirty-eight
    // rows for four physical devices. This is the way back out, over the same
    // socket the window uses.
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
    let before = devices.as_array().expect("array").len();
    let Some(id) = devices
        .as_array()
        .expect("array")
        .iter()
        .find(|d| d["status"] == "pending")
        .map(|d| d["id"].as_str().expect("id").to_string())
    else {
        return;
    };

    daemon.call("device.forget", Some(serde_json::json!({ "id": id })));

    let after = daemon.call("device.list", None);
    let rows = after.as_array().expect("array");
    assert_eq!(rows.len(), before - 1, "the row should be gone: {after}");
    assert!(
        !rows.iter().any(|d| d["id"] == id.as_str()),
        "and it should be that row: {after}"
    );

    // The row is gone; why it went is not. An owner looking back at a list
    // that used to have a device on it has to be able to find this.
    let events = daemon.call("log.query", Some(serde_json::json!({ "limit": 20 })));
    assert!(
        events
            .as_array()
            .expect("array")
            .iter()
            .any(|e| e["action"] == "forgotten" && e["device_id"] == id.as_str()),
        "the removal should be in the log: {events}"
    );

    // And the browser is a stranger again rather than being locked out: it
    // asks, which is the whole reason this is not a way to refuse anybody.
    let (again, _) = daemon.get_as_stranger("/portugal/");
    assert!(again == 200 || again == 403, "unexpected status {again}");
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
