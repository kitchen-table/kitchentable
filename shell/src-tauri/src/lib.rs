//! Kitchen Table desktop shell.
//!
//! The Rust side stays deliberately dumb (docs/architecture.md section 13):
//! window and tray management, spawning and supervising the daemon, and a
//! proxy between Tauri IPC and the daemon's Unix socket. All product logic
//! renders in React from socket state.

use std::sync::Arc;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager, State,
};

mod daemon;
mod socket;

use daemon::Supervisor;

struct AppState {
    supervisor: Arc<Supervisor>,
}

/// The one command the UI needs: forward a JSON-RPC call to the daemon.
///
/// Deliberately generic. A command per method would mean editing Rust every
/// time the socket API grows, and would tempt the shell into interpreting
/// payloads it has no business understanding.
#[tauri::command]
fn kt_call(
    method: String,
    params: Option<serde_json::Value>,
) -> Result<serde_json::Value, serde_json::Value> {
    socket::call(&socket::socket_path(), &method, params).map_err(|e| {
        // Not an error worth a stack trace: the daemon being down is a state
        // the UI is designed to render.
        tracing::debug!(method, error = %e, "socket call failed");
        e.to_payload()
    })
}

/// Whether the daemon is up, for the window's connection state.
///
/// Reports the socket's view first, since a daemon we adopted rather than
/// spawned is healthy even though we have no child process for it.
#[tauri::command]
fn kt_health(state: State<'_, AppState>) -> serde_json::Value {
    let ours = state.supervisor.is_alive();
    let mut health = match socket::handshake(&socket::socket_path()) {
        socket::Handshake::Ready => serde_json::json!({ "state": "ready" }),
        socket::Handshake::NotRunning => serde_json::json!({ "state": "not_running" }),
        socket::Handshake::WrongVersion { theirs, ours } => serde_json::json!({
            "state": "wrong_version", "theirs": theirs, "ours": ours,
        }),
        socket::Handshake::Unhealthy(reason) => {
            serde_json::json!({ "state": "unhealthy", "reason": reason })
        }
    };

    if let Some(object) = health.as_object_mut() {
        object.insert("we_started_it".into(), ours.into());
    }
    health
}

/// Restart the daemon, for the "try again" button on the disconnected state.
#[tauri::command]
fn kt_restart_daemon(state: State<'_, AppState>) -> Result<(), String> {
    state.supervisor.stop();
    state
        .supervisor
        .ensure_running()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Stop the daemon and exit.
///
/// The same path as the tray's Quit item, so there is one way to actually stop
/// serving no matter which surface asked.
#[tauri::command]
fn kt_quit(app: tauri::AppHandle, state: State<'_, AppState>) {
    state.supervisor.stop();
    app.exit(0);
}

/// Closing the window hides it; the daemon keeps serving. Quitting is an
/// explicit choice from the tray (docs/architecture.md section 3).
fn hide_instead_of_closing(window: &tauri::Window, event: &tauri::WindowEvent) {
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        let _ = window.hide();
        tracing::debug!("window hidden; serving continues");
    }
}

/// How long to wait before trying the event stream again.
///
/// The daemon going away is ordinary - an update restarts it - so this is a
/// retry interval rather than an error path. Short enough that the window is
/// not stale for long, long enough that a daemon which refuses to start does
/// not turn into a busy loop.
const RESUBSCRIBE_AFTER: std::time::Duration = std::time::Duration::from_secs(2);

/// Pump daemon events into the window, forever.
///
/// The Rust side stays a dumb proxy here too: it does not read the events, it
/// re-emits them under one name and lets the UI decide what any of it means.
///
/// Reconnecting is the interesting part. A daemon that restarts has a new
/// library, new devices and possibly new URLs, and the window has been looking
/// at the old ones - so reconnecting emits a resync rather than quietly
/// resuming, and the UI refetches instead of trusting what it already has.
fn forward_events(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let socket = socket::socket_path();
        loop {
            let emitter = app.clone();
            match socket::subscribe(&socket, move |event| {
                if let Err(e) = emitter.emit("kt://event", event) {
                    tracing::debug!(error = %e, "could not emit an event");
                }
            }) {
                Ok(()) => tracing::debug!("the daemon closed the event stream"),
                Err(e) => tracing::debug!(error = %e, "could not subscribe"),
            }

            std::thread::sleep(RESUBSCRIBE_AFTER);
            // Whatever happened while we were not listening, we missed.
            let _ = app.emit("kt://resync", ());
        }
    });
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Kitchen Table", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Kitchen Table", true, Some("CmdOrCtrl+Q"))?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &sep, &quit])?;

    TrayIconBuilder::with_id("main")
        .icon(
            app.default_window_icon()
                .cloned()
                .expect("bundled app icon"),
        )
        // A template image adopts the menu bar's own colour, so the icon works
        // in both light and dark menu bars instead of being a coloured square.
        .icon_as_template(true)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_window(app),
            "quit" => {
                if let Some(state) = app.try_state::<AppState>() {
                    state.supervisor.stop();
                }
                app.exit(0);
            }
            other => tracing::warn!(menu_item = other, "unhandled tray menu item"),
        })
        .build(app)?;

    Ok(())
}

fn show_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kitchentable_shell_lib=info".into()),
        )
        .init();

    let supervisor = Arc::new(Supervisor::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // The folder picker behind "Add an app". Picking a folder is the one
        // thing the daemon cannot do for itself: it has no window to put a
        // native panel on.
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            supervisor: Arc::clone(&supervisor),
        })
        .invoke_handler(tauri::generate_handler![
            kt_call,
            kt_health,
            kt_restart_daemon,
            kt_quit
        ])
        .setup(move |app| {
            build_tray(app.handle())?;

            // Start the daemon off the main thread: the window should appear
            // immediately and render its connecting state, not block on a
            // process launch.
            let supervisor = Arc::clone(&supervisor);
            std::thread::spawn(move || match supervisor.ensure_running() {
                Ok(started) => tracing::info!(started, "daemon ready"),
                Err(e) => tracing::error!(error = %e, "could not start the daemon"),
            });

            forward_events(app.handle().clone());

            tracing::info!("shell started");
            Ok(())
        })
        .on_window_event(hide_instead_of_closing)
        .run(tauri::generate_context!())
        .expect("failed to start the Kitchen Table shell");
}
