//! The one thing Kitchen Table adds to an app it serves.
//!
//! A page has to keep saying it is open or nobody can know that it is, so a
//! single script tag goes into served HTML and checks in every few seconds.
//! This is the only modification made to anyone's files on the way out, it is
//! never written to disk, and it is confined to HTML documents - assets go out
//! byte for byte.
//!
//! Deliberately an external same-origin script rather than an inline one. An
//! app with `Content-Security-Policy: script-src 'self'` - the common strict
//! setting - blocks inline scripts and allows this, so the usual way of being
//! careful about scripts does not break the app instead.

use crate::presence::BEAT_EVERY;

/// Where the script and its check-ins live.
///
/// Under one reserved prefix so an app that happens to have a `live.js` of its
/// own is unaffected, and so the whole thing is greppable by anyone wondering
/// what that request in their log is.
pub const PREFIX: &str = "/__kt";

/// The script tag, for a given app.
///
/// Carries the slug because the same page is reachable two ways - on the app's
/// own `.local` hostname and under a `/slug/` prefix on this machine - and the
/// check-in must say which app it belongs to either way.
pub fn tag(slug: &str) -> String {
    format!(r#"<script src="{PREFIX}/live.js?app={slug}" defer></script>"#)
}

/// The client. Small on purpose: it is somebody else's page.
pub fn script() -> String {
    let seconds = BEAT_EVERY.as_secs();
    format!(
        r#"// Kitchen Table: tells the owner's window that this page is open.
// Sends no page content - only which app, and which path you are on.
(function () {{
  var app = new URL(document.currentScript.src).searchParams.get("app");
  if (!app) return;
  var stopped = false;
  function beat() {{
    if (stopped || document.visibilityState === "hidden") return;
    fetch("{PREFIX}/beat?app=" + encodeURIComponent(app) +
          "&path=" + encodeURIComponent(location.pathname),
          {{ credentials: "same-origin", cache: "no-store" }})
      .catch(function () {{}});
  }}
  beat();
  var timer = setInterval(beat, {seconds}000);
  // Say goodbye when the tab goes, so the owner's list empties promptly
  // instead of waiting for the entry to age out.
  addEventListener("pagehide", function () {{
    stopped = true;
    clearInterval(timer);
    navigator.sendBeacon("{PREFIX}/gone?app=" + encodeURIComponent(app));
  }});
  addEventListener("visibilitychange", function () {{
    if (document.visibilityState === "visible") beat();
  }});
}})();
"#
    )
}

/// Put the tag into an HTML document.
///
/// Before `</body>` where there is one, so the app's own scripts run first and
/// nothing of ours is in the way of the page rendering. A fragment with no
/// body - which is a normal thing to serve - just gets it appended.
///
/// Case-insensitive, because HTML is, and matching only `</body>` would silently
/// skip a page written with `</BODY>`.
pub fn inject(html: &str, slug: &str) -> String {
    let tag = tag(slug);
    let lower = html.to_ascii_lowercase();
    match lower.rfind("</body>") {
        Some(at) => format!("{}{tag}{}", &html[..at], &html[at..]),
        None => format!("{html}{tag}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tag_goes_before_the_closing_body() {
        let out = inject("<html><body><h1>Hi</h1></body></html>", "trip");
        assert!(out.starts_with("<html><body><h1>Hi</h1>"));
        assert!(out.contains(r#"src="/__kt/live.js?app=trip""#));
        assert!(out.ends_with("</body></html>"), "got {out}");
    }

    #[test]
    fn an_uppercase_body_tag_is_still_found() {
        // HTML is case-insensitive and people write it by hand.
        let out = inject("<HTML><BODY>Hi</BODY></HTML>", "trip");
        assert!(out.contains("</BODY></HTML>"));
        assert!(out.contains("/__kt/live.js"));
    }

    #[test]
    fn a_fragment_with_no_body_still_gets_it() {
        let out = inject("<h1>Just this</h1>", "trip");
        assert!(out.starts_with("<h1>Just this</h1>"));
        assert!(out.contains("/__kt/live.js"));
    }

    #[test]
    fn only_the_last_body_close_is_used() {
        // A page that mentions </body> inside a string or a comment should not
        // have the script wedged into the middle of it.
        let out = inject("<body><script>var s='</body>';</script></body>", "trip");
        assert!(out.ends_with("</body>"));
        assert_eq!(out.matches("/__kt/live.js").count(), 1);
    }

    #[test]
    fn the_script_names_no_content() {
        // It reports which app and which path, and nothing about the page
        // itself. Anything more would make this a tracker.
        let js = script();
        assert!(js.contains("location.pathname"));
        assert!(!js.contains("document.title"));
        assert!(!js.contains("innerHTML"));
        assert!(!js.contains("cookie"));
    }
}
