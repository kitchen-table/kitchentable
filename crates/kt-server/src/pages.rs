//! The pages a viewer sees before they see the app.
//!
//! Served by the daemon on the app's own hostname, so someone opening a shared
//! link never bounces to a different domain (docs/architecture.md section 5).
//! Deliberately self-contained: a viewer is on a phone, on someone else's
//! network, and possibly on a captive portal, so nothing here loads.

/// Shared chrome. The palette matches the desktop app's tokens, and follows
/// the phone's own light or dark setting.
const STYLE: &str = r#"
:root{--paper:#fbfaf8;--raised:#f0ece4;--ink:#1a1a1a;--ink3:#6b6b6b;
--muted:#8a8378;--faint:#a8a091;--accent:#2f6df0;--border:rgba(0,0,0,.08)}
@media (prefers-color-scheme:dark){:root{--paper:#211f19;--raised:#2b2822;
--ink:#f2ede3;--ink3:#b2ab9d;--muted:#918a7d;--faint:#6f6a5e;--accent:#5b8dff;
--border:rgba(255,255,255,.11)}}
*{box-sizing:border-box}
body{margin:0;min-height:100vh;display:flex;align-items:center;
justify-content:center;padding:32px 24px;background:var(--paper);color:var(--ink);
font:400 16px/1.5 system-ui,-apple-system,sans-serif;-webkit-font-smoothing:antialiased}
main{width:100%;max-width:400px;text-align:center}
h1{margin:0 0 10px;font-size:24px;letter-spacing:-.02em}
p{margin:0 0 8px;color:var(--ink3);font-size:15px;line-height:1.6}
.small{font-size:12.5px;color:var(--faint)}
.mark{margin:0 auto 22px;width:56px;height:56px;display:block}
.spin{margin:22px auto 14px;width:26px;height:26px;border:3px solid var(--raised);
border-top-color:var(--accent);border-radius:50%;animation:s .9s linear infinite}
@keyframes s{to{transform:rotate(360deg)}}
.card{background:var(--raised);border:1px solid var(--border);border-radius:14px;
padding:16px;margin-top:20px;text-align:left}
"#;

/// The Kitchen Table mark, inline so it needs no request of its own.
const MARK: &str = r#"<svg class="mark" viewBox="0 0 32 32" aria-hidden="true">
<rect x="4" y="11" width="24" height="5" rx="2.5" fill="var(--accent)"></rect>
<rect x="8" y="16" width="3.4" height="11" rx="1.7" fill="var(--ink)"></rect>
<rect x="20.6" y="16" width="3.4" height="11" rx="1.7" fill="var(--ink)"></rect></svg>"#;

/// Wrap a body in the page chrome.
///
/// `title` is escaped here rather than at each call site: it comes from a
/// folder name, and the one place every page funnels through is the only place
/// that can guarantee it happens.
fn page(title: &str, body: &str, refresh: Option<u32>) -> String {
    let meta_refresh = match refresh {
        // A meta refresh rather than JavaScript: it keeps working if the page
        // is restored from the back/forward cache, and needs no script at all.
        Some(seconds) => format!(r#"<meta http-equiv="refresh" content="{seconds}">"#),
        None => String::new(),
    };

    let title = escape(title);

    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="robots" content="noindex,nofollow">
{meta_refresh}<title>{title}</title><style>{STYLE}</style></head>
<body><main>{MARK}{body}</main></body></html>"#
    )
}

/// Shown while the owner is being asked. Refreshes itself until the answer
/// arrives, which is why the viewer never has to do anything.
pub fn waiting(app_name: &str, owner_hint: &str) -> String {
    page(
        app_name,
        &format!(
            r#"<p class="small">{owner_hint} shared</p>
<h1>{app}</h1>
<div class="spin"></div>
<p>Waiting for {owner_hint} to let this device in.</p>
<p class="small">You do not need to do anything. This page updates by itself.</p>"#,
            app = escape(app_name),
            owner_hint = escape(owner_hint),
        ),
        Some(3),
    )
}

/// Shown once, in the moment between opening the link and the app appearing.
pub fn interstitial(app_name: &str, owner_hint: &str) -> String {
    page(
        app_name,
        &format!(
            r#"<p class="small">{owner_hint} shared</p>
<h1>{app}</h1>
<div class="spin"></div>
<p>Getting it ready…</p>
<p class="small">No account. No install. Ever.</p>"#,
            app = escape(app_name),
            owner_hint = escape(owner_hint),
        ),
        Some(1),
    )
}

/// The owner said no, or the device was revoked. A dead end, said kindly and
/// without implying the viewer did something wrong.
/// The owner took it offline.
///
/// Says so plainly and says it is temporary. A viewer who gets "not available"
/// assumes they are unwelcome and asks the owner about it; the truth is that
/// nothing is wrong with them and the app is coming back.
pub fn paused(app_name: &str) -> String {
    page(
        app_name,
        &format!(
            r#"<h1>Paused</h1>
<p>{app} has been paused by whoever shares it.</p>
<p class="small">Your link still works. Try again once it is back.</p>"#,
            app = escape(app_name)
        ),
        None,
    )
}

pub fn denied(app_name: &str) -> String {
    page(
        app_name,
        &format!(
            r#"<h1>Not available</h1>
<p>{app} is not shared with this device.</p>
<p class="small">If you think that is a mistake, ask whoever sent you the link
for a new one.</p>"#,
            app = escape(app_name)
        ),
        None,
    )
}

/// The link itself is no good: expired, revoked, or already claimed by another
/// device. Each reason gets its own sentence, because "invalid link" leaves
/// someone with nothing to do.
pub fn bad_invite(reason: &str) -> String {
    page(
        "Link not valid",
        &format!(
            r#"<h1>This link will not open</h1>
<p>{reason}</p>
<p class="small">Ask whoever shared it with you for a fresh link.</p>"#,
            reason = escape(reason)
        ),
        None,
    )
}

/// Household visibility, wrong network. The one refusal with a fix the viewer
/// can act on themselves.
pub fn wrong_network(app_name: &str) -> String {
    page(
        app_name,
        &format!(
            r#"<h1>{app} is a home app</h1>
<p>It only opens on its owner's home network. You are on a different one right
now.</p>
<p class="small">Connect to their wifi, or ask them for a share link that works
anywhere.</p>"#,
            app = escape(app_name)
        ),
        None,
    )
}

/// App names come from folder names on disk, so they are untrusted here.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_is_self_contained() {
        // A viewer may be on a captive portal or a network that cannot reach
        // anything but this machine. A page that needs a font or a script
        // would render broken exactly when it matters most.
        let pages = [
            waiting("Trip", "Someone"),
            interstitial("Trip", "Someone"),
            denied("Trip"),
            bad_invite("expired"),
            wrong_network("Trip"),
        ];

        for html in pages {
            assert!(!html.contains("http://"), "external reference: {html}");
            assert!(!html.contains("https://"), "external reference: {html}");
            assert!(!html.contains("<script"), "needs script: {html}");
        }
    }

    #[test]
    fn the_waiting_page_refreshes_itself() {
        // The viewer's whole job is to do nothing, so the page has to poll.
        assert!(waiting("Trip", "Someone").contains("http-equiv=\"refresh\""));
    }

    #[test]
    fn dead_ends_do_not_refresh() {
        for html in [denied("Trip"), bad_invite("expired"), wrong_network("Trip")] {
            assert!(!html.contains("refresh"), "a dead end should not reload");
        }
    }

    #[test]
    fn app_names_are_escaped_everywhere_including_the_title() {
        // A folder can be named anything, and the title is as much an
        // injection point as the body.
        let evil = "<script>alert(1)</script>";
        for html in [
            waiting(evil, "Someone"),
            interstitial(evil, "Someone"),
            denied(evil),
            wrong_network(evil),
        ] {
            assert!(!html.contains("<script>alert"), "unescaped: {html}");
            assert!(html.contains("&lt;script&gt;"));
        }
    }

    #[test]
    fn the_owner_hint_is_escaped_too() {
        let html = waiting("Trip", "<img onerror=alert(1)>");
        assert!(!html.contains("<img onerror"));
    }

    #[test]
    fn no_page_is_indexable() {
        // Shared links end up in chat apps that prefetch them.
        for html in [waiting("a", "b"), interstitial("a", "b"), denied("a")] {
            assert!(html.contains("noindex"));
        }
    }

    #[test]
    fn the_wrong_network_page_says_what_to_do_about_it() {
        let html = wrong_network("Chores");
        assert!(html.contains("home network"));
        assert!(html.contains("wifi") || html.contains("share link"));
    }
}
