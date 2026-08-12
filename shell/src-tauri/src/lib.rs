//! Kitchen Table desktop shell.
//!
//! The Rust side here stays deliberately dumb (docs/architecture.md section
//! 13): window and tray management, native dialogs, autostart, the updater,
//! spawning and supervising the daemon, and a bidirectional proxy between
//! Tauri IPC and the daemon's Unix socket. All product logic renders in React
//! from socket state.
//!
//! Scaffold status: window and tray only. Daemon supervision lands in D3.

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Manager,
};

/// Closing the window hides it; the daemon keeps serving. Quitting is an
/// explicit choice from the tray (docs/architecture.md section 3).
fn hide_instead_of_closing(window: &tauri::Window, api: &tauri::WindowEvent) {
    if let tauri::WindowEvent::CloseRequested { api: close, .. } = api {
        close.prevent_close();
        let _ = window.hide();
        tracing::debug!("window hidden; serving continues");
    }
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Kitchen Table", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Pause serving", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Kitchen Table", true, Some("CmdOrCtrl+Q"))?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &pause, &sep, &quit])?;

    TrayIconBuilder::with_id("main")
        .icon(
            app.default_window_icon()
                .cloned()
                .expect("bundled app icon"),
        )
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            // Wired to sys.pause_serving over the socket in D3.
            "pause" => tracing::info!("pause serving requested"),
            "quit" => app.exit(0),
            other => tracing::warn!(menu_item = other, "unhandled tray menu item"),
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    tauri::Builder::default()
        .setup(|app| {
            build_tray(app.handle())?;
            tracing::info!("shell started");
            Ok(())
        })
        .on_window_event(hide_instead_of_closing)
        .run(tauri::generate_context!())
        .expect("failed to start the Kitchen Table shell");
}
