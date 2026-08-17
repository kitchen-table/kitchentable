//! Desktop notifications, and the permission macOS gates them behind.
//!
//! The shell owns this rather than the daemon for the same reason it owns the
//! tray: it is the half with a user session to draw on. The daemon has no
//! desktop, and a headless install has nobody to notify.
//!
//! It stays a proxy all the same (docs/architecture.md section 13). Rust posts
//! whatever wording the window hands it and reports which button came back; it
//! does not decide what an access request is, or what approving one means. That
//! knowledge stays in one place, on the socket, where the CLI and an agent can
//! reach it too.
//!
//! # Why not `tauri-plugin-notification`
//!
//! It was tried first. On desktop it sets a title, body, icon and sound and
//! nothing else, so the Approve and Deny buttons the design puts on an access
//! request cannot exist; and its `permission_state` is a stub that returns
//! `Granted` without asking macOS, so a window built on it would draw a
//! settled permission it had never checked. Both matter here, so this talks to
//! `UNUserNotificationCenter` instead - the framework Apple supports, and the
//! only one with an authorisation prompt.
//!
//! # What macOS requires
//!
//! A bundled, signed app. `pnpm tauri dev` runs a bare binary with no bundle
//! identifier, and there the whole surface reports [`UNBUNDLED`] rather than
//! failing: notifications are the one part of this product that cannot be
//! developed against the dev server, and saying so is better than a permission
//! prompt that never appears and an error nobody can act on.

use serde::Deserialize;

/// The Tauri event carrying a pressed button back to the window.
pub const ACTION_EVENT: &str = "kt://notification-action";

/// Not macOS. Notifications are not built for this platform yet.
#[cfg_attr(target_os = "macos", allow(dead_code))]
const UNSUPPORTED: &str = "unsupported";
/// macOS, but this process has no bundle identifier - a `tauri dev` binary.
const UNBUNDLED: &str = "unbundled";
/// Nobody has been asked yet. The window offers a button that asks.
const NOT_DETERMINED: &str = "not_determined";
/// The owner said no, or Focus/Do Not Disturb settings amount to the same.
/// Only System Settings can undo it; asking again does nothing, by design.
const DENIED: &str = "denied";
const GRANTED: &str = "granted";

/// How long an actionable notification waits for an answer.
///
/// Not optional: `UNUserNotificationCenter` never reports a response for a
/// notification the owner cleared from Notification Center, so a request with
/// no timeout can leave a thread parked for the life of the process. Five
/// minutes is long enough for somebody to walk back to their desk, and when it
/// lapses the banner is withdrawn and the request is still waiting in the
/// window - which is where a decision this old belongs anyway.
#[cfg(target_os = "macos")]
const ANSWER_WITHIN: std::time::Duration = std::time::Duration::from_secs(300);

/// One notification, as the window describes it.
///
/// `key` is opaque here and comes back untouched on the action event: the
/// window sends a device id, and Rust never learns that is what it is.
#[derive(Debug, Deserialize)]
pub struct Spec {
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    /// Buttons, in the order they should appear. Empty means an informational
    /// banner with nothing to press.
    #[serde(default)]
    pub actions: Vec<Button>,
    #[serde(default)]
    pub sound: bool,
}

#[derive(Debug, Deserialize)]
pub struct Button {
    /// Sent back verbatim on the action event.
    pub id: String,
    pub label: String,
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{Spec, ACTION_EVENT, ANSWER_WITHIN, DENIED, GRANTED, NOT_DETERMINED, UNBUNDLED};
    use mac_usernotifications::{
        block_on, blocking, check_bundle, Action, AuthorizationStatus, Notification,
        NotificationSettingStatus,
    };
    use tauri::Emitter;

    /// What macOS will currently do with a notification from this app.
    ///
    /// `alerts` is reported separately from the authorisation because the two
    /// really do come apart: an owner can grant permission and still have
    /// banners switched off, in which case everything we post goes straight to
    /// Notification Center unseen. A window that said "on" there would be
    /// telling somebody their access requests will interrupt them when they
    /// will not.
    pub fn state() -> serde_json::Value {
        if check_bundle().is_err() {
            return serde_json::json!({ "state": UNBUNDLED, "alerts": false });
        }

        match blocking::get_notification_settings() {
            Ok(settings) => {
                let state = match settings.authorization_status {
                    AuthorizationStatus::Authorized
                    | AuthorizationStatus::Provisional
                    | AuthorizationStatus::Ephemeral => GRANTED,
                    AuthorizationStatus::Denied => DENIED,
                    // An unknown status is treated as "not asked". The prompt
                    // is harmless if it turns out we have permission already -
                    // macOS shows it once and answers from its own record
                    // afterwards - and this way a future status we do not know
                    // about cannot silently turn notifications off.
                    AuthorizationStatus::NotDetermined | AuthorizationStatus::Unknown => {
                        NOT_DETERMINED
                    }
                };
                serde_json::json!({
                    "state": state,
                    "alerts": settings.alert_enabled == NotificationSettingStatus::Enabled,
                })
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not read the notification settings");
                serde_json::json!({ "state": NOT_DETERMINED, "alerts": false })
            }
        }
    }

    /// Show the system prompt, then report where that left us.
    ///
    /// macOS only ever asks once per install; after that this returns the
    /// stored answer without showing anything. That is why the window must not
    /// offer it as a retry - a second press would look broken.
    pub fn request() -> serde_json::Value {
        if check_bundle().is_err() {
            return serde_json::json!({ "state": UNBUNDLED, "alerts": false });
        }

        match blocking::request_auth() {
            Ok(granted) => tracing::info!(granted, "notification permission answered"),
            Err(e) => tracing::warn!(error = %e, "could not ask for notification permission"),
        }

        // Deliberately re-read rather than trusting the boolean: the prompt
        // answers authorisation only, and the window also needs to know whether
        // banners are switched on.
        state()
    }

    /// Post one notification and, if it has buttons, wait for the answer.
    ///
    /// Off the calling thread, because waiting is the normal case: the future
    /// resolves when somebody presses a button, which may be minutes away. The
    /// delegate callback arrives on the main run loop, which Tauri is already
    /// pumping.
    pub fn show(app: tauri::AppHandle, spec: Spec) {
        if check_bundle().is_err() {
            tracing::debug!("no bundle identifier; skipping the notification");
            return;
        }

        std::thread::spawn(move || {
            let mut notification = Notification::new().title(&spec.title).message(&spec.body);

            if let Some(subtitle) = &spec.subtitle {
                notification = notification.subtitle(subtitle);
            }
            if spec.sound {
                notification = notification.default_sound();
            }

            let actionable = !spec.actions.is_empty();
            for button in &spec.actions {
                notification = notification.action(Action::button(&button.id, &button.label));
            }
            if actionable {
                notification = notification.timeout(ANSWER_WITHIN);
            }

            let handle = match notification.send_blocking() {
                Ok(handle) => handle,
                Err(e) => {
                    // Includes the case where the owner has denied permission:
                    // there is nothing to recover here, and the window already
                    // shows the same request in a list.
                    tracing::warn!(error = %e, "could not post a notification");
                    return;
                }
            };

            // Nothing to wait for, and nothing the window could do with a
            // dismissal of a banner it only meant as information.
            if !actionable {
                return;
            }

            match block_on(handle.response()) {
                Ok(response) => {
                    // The body was clicked rather than a button. Window
                    // management is the shell's own job, so this is the one
                    // response it acts on itself - and it still forwards the
                    // event, because the window may want to show the request.
                    if response.is_default_action() {
                        crate::show_window(&app);
                    }

                    let action = if response.is_default_action() {
                        "default".to_string()
                    } else if response.is_timed_out() {
                        "timed_out".to_string()
                    } else if response.is_dismiss_action() {
                        "dismissed".to_string()
                    } else {
                        response.action_identifier.clone()
                    };

                    if let Err(e) = app.emit(
                        ACTION_EVENT,
                        serde_json::json!({ "key": spec.key, "action": action }),
                    ) {
                        tracing::warn!(error = %e, "could not deliver a notification action");
                    }
                }
                Err(e) => tracing::warn!(error = %e, "no answer from the notification"),
            }
        });
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::{Spec, UNSUPPORTED};

    pub fn state() -> serde_json::Value {
        serde_json::json!({ "state": UNSUPPORTED, "alerts": false })
    }

    pub fn request() -> serde_json::Value {
        state()
    }

    /// Deliberately silent. Windows and Linux get their own path when this
    /// product ships for them; until then the window reads [`UNSUPPORTED`] and
    /// says so, rather than offering a switch that does nothing.
    pub fn show(_app: tauri::AppHandle, _spec: Spec) {}
}

/// Whether macOS will show a notification from this app, without asking.
#[tauri::command]
pub fn kt_notify_state() -> serde_json::Value {
    imp::state()
}

/// Ask macOS for permission, showing the system prompt if it has never asked.
#[tauri::command]
pub fn kt_notify_request() -> serde_json::Value {
    imp::request()
}

/// Post a notification. Answers to its buttons arrive on [`ACTION_EVENT`].
#[tauri::command]
pub fn kt_notify(app: tauri::AppHandle, spec: Spec) {
    imp::show(app, spec);
}

/// Kept honest by the fact that both halves of `imp` are compiled from the same
/// signatures; the useful assertions about wording and timing live in the UI's
/// own tests, where they can run on every platform.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_needs_only_a_title_and_a_body() {
        let spec: Spec = serde_json::from_value(serde_json::json!({
            "title": "Priya's iPad wants access",
            "body": "Trip Planner",
        }))
        .expect("deserialises");

        assert!(
            spec.actions.is_empty(),
            "an informational banner by default"
        );
        assert!(!spec.sound);
        assert_eq!(spec.key, None);
    }

    #[test]
    fn buttons_keep_the_order_the_window_sent() {
        let spec: Spec = serde_json::from_value(serde_json::json!({
            "title": "t",
            "body": "b",
            "key": "device-1",
            "actions": [
                { "id": "approve", "label": "Approve" },
                { "id": "deny", "label": "Deny" },
            ],
        }))
        .expect("deserialises");

        let ids: Vec<_> = spec.actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["approve", "deny"]);
        assert_eq!(spec.key.as_deref(), Some("device-1"));
    }

    #[test]
    fn the_state_is_always_a_string_the_window_can_switch_on() {
        let state = kt_notify_state();
        let named = state["state"].as_str().expect("a state string");

        assert!(
            [UNSUPPORTED, UNBUNDLED, NOT_DETERMINED, DENIED, GRANTED].contains(&named),
            "unexpected state {named:?}",
        );
    }
}
