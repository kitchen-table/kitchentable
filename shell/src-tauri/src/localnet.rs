//! The macOS Local Network permission: asking for it, and reading it back.
//!
//! macOS gates access to the local network behind a permission it grants per
//! *app*, and it has no API for either half of this. There is nothing to call
//! that raises the prompt, and nothing to call that reports the answer. What
//! exists is a rule: the first time an app touches the local network, macOS
//! asks, and from then on it silently allows or blocks.
//!
//! So the prompt is raised by doing the thing - sending one multicast DNS query,
//! exactly what the daemon does when it announces an app - and the answer is
//! inferred from what comes back.
//!
//! # Why the shell and not the daemon
//!
//! Because the prompt names whoever asked. The daemon is a separate executable
//! that the shell spawns, and a permission dialog naming a background binary,
//! or attributed to a process the owner has never heard of, is the failure mode
//! `checklist-desktop.md` spike S1 was written to avoid. Asking from the app
//! bundle is the fallback that spike names, and it puts "Kitchen Table" in the
//! dialog because Kitchen Table is what asked.
//!
//! # What can honestly be concluded
//!
//! - A reply came back: something on the network answered us, so we are
//!   **allowed**. This is the only positive that is worth anything.
//! - The send was refused outright: **denied**. macOS fails the send rather
//!   than pretending, and this is the only negative worth anything.
//! - Silence: **unknown**, and the screen keeps offering the button. A quiet
//!   network with nothing to answer looks exactly like a network we cannot
//!   reach, and guessing between them would eventually tell somebody their
//!   phone can see their apps when it cannot.
//!
//! Never a false "allowed": the socket here is our own, so a reply means a
//! packet genuinely crossed the network. mDNSResponder's cache cannot answer
//! for it.

use std::time::Duration;

/// Something answered, so the network is reachable.
const ALLOWED: &str = "allowed";
/// The OS refused the send.
const DENIED: &str = "denied";
/// Nothing came back in time, and nothing refused us.
const UNKNOWN: &str = "unknown";
/// Not macOS: no such permission, nothing to ask for.
#[cfg_attr(target_os = "macos", allow(dead_code))]
const NOT_APPLICABLE: &str = "not_applicable";

/// The multicast DNS group and port every Bonjour device listens on.
const MDNS: &str = "224.0.0.251:5353";

/// How long to wait for any device to answer.
///
/// Answers arrive in milliseconds on a network with anything on it. This is
/// long enough to cross a slow access point and short enough that somebody who
/// pressed a button is not left watching a spinner - and it runs *after* the
/// dialog, so it is not competing with the time somebody spends reading it.
const LISTEN_FOR: Duration = Duration::from_millis(1500);

/// Ask every device on the network to name the services it offers.
///
/// The standard `_services._dns-sd._udp.local` meta-query, which is the one
/// question every responder answers, so it maximises the chance of hearing
/// *something* on a quiet network. The unicast-response bit is set so replies
/// come straight back to our port; without it responders answer to the group,
/// which we would have to join to hear.
fn query() -> Vec<u8> {
    let mut packet = vec![
        0, 0, // no transaction id: multicast DNS matches on the question
        0, 0, // no flags: a plain query
        0, 1, // one question
        0, 0, 0, 0, 0, 0, // no answers, authorities or additional records
    ];

    for label in ["_services", "_dns-sd", "_udp", "local"] {
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0); // end of name

    packet.extend_from_slice(&[0, 12]); // PTR
    packet.extend_from_slice(&[0x80, 1]); // IN, answer to me directly
    packet
}

#[cfg(target_os = "macos")]
fn probe() -> &'static str {
    use std::net::UdpSocket;

    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(socket) => socket,
        Err(e) => {
            tracing::warn!(error = %e, "could not open a socket to ask the network");
            return UNKNOWN;
        }
    };

    if let Err(e) = socket.set_read_timeout(Some(LISTEN_FOR)) {
        tracing::warn!(error = %e, "could not set a read timeout");
        return UNKNOWN;
    }

    // The send itself is what raises the dialog, and it blocks until the owner
    // answers it. Everything below happens after they have decided.
    if let Err(e) = socket.send_to(&query(), MDNS) {
        // The refusal macOS gives an app it will not let onto the network.
        // Other errors are real network faults - no route, no interface - and
        // saying "denied" for those would send somebody to a settings pane
        // where nothing is wrong.
        let denied = e.kind() == std::io::ErrorKind::HostUnreachable
            || e.kind() == std::io::ErrorKind::PermissionDenied;
        tracing::info!(error = %e, denied, "the local network query was refused");
        return if denied { DENIED } else { UNKNOWN };
    }

    let mut buffer = [0_u8; 1500];
    match socket.recv_from(&mut buffer) {
        Ok((_, from)) => {
            tracing::info!(%from, "the network answered");
            ALLOWED
        }
        Err(e) => {
            // A timeout here is genuinely inconclusive; see the module docs.
            tracing::info!(error = %e, "nothing answered the local network query");
            UNKNOWN
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn probe() -> &'static str {
    NOT_APPLICABLE
}

/// Touch the local network, raising the macOS prompt if it has never been
/// answered, and report what that told us.
///
/// Off the main thread: the send blocks for as long as the dialog is on screen,
/// and blocking the main thread would freeze the window behind it.
#[tauri::command]
pub async fn kt_local_network() -> serde_json::Value {
    let state = tauri::async_runtime::spawn_blocking(probe)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "the local network probe did not finish");
            UNKNOWN
        });

    serde_json::json!({ "state": state })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_query_asks_the_question_every_responder_answers() {
        let packet = query();

        assert_eq!(&packet[4..6], &[0, 1], "exactly one question");
        assert!(
            packet.windows(9).any(|w| w == b"_services"),
            "the service enumeration meta-query",
        );

        let tail = &packet[packet.len() - 4..];
        assert_eq!(&tail[0..2], &[0, 12], "a PTR question");
        assert_eq!(
            tail[2] & 0x80,
            0x80,
            "the unicast-response bit, so replies reach our own port",
        );
    }

    #[test]
    fn a_probe_answers_with_a_state_the_window_can_switch_on() {
        // Runs the real thing. On a machine with the permission already
        // granted this reaches the network; on CI it times out. Both are
        // states the screen draws, and neither is an error.
        let state = probe();

        assert!(
            [ALLOWED, DENIED, UNKNOWN, NOT_APPLICABLE].contains(&state),
            "unexpected state {state:?}",
        );
    }
}
