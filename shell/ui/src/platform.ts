/**
 * Which OS the window is on, for the handful of places the design differs.
 *
 * Read from the webview rather than the Tauri OS plugin: this is only ever used
 * for chrome layout (where the window buttons sit) and copy ("this Mac"), and
 * a wrong answer degrades to a slightly wide title bar rather than anything
 * that matters. Adding a plugin and an async call for that is not worth it.
 */
export type Platform = "macos" | "windows" | "linux";

export function detect(): Platform {
  const ua = navigator.userAgent;
  if (/Mac OS X|Macintosh/.test(ua)) return "macos";
  if (/Windows/.test(ua)) return "windows";
  return "linux";
}

/** Stamps the root element so tokens.css can key off the platform. */
export function apply(platform: Platform = detect()): Platform {
  document.documentElement.setAttribute("data-platform", platform);
  return platform;
}

/** "this Mac" / "this PC" / "this computer", for onboarding and Settings copy. */
export function thisMachine(platform: Platform = detect()): string {
  switch (platform) {
    case "macos":
      return "this Mac";
    case "windows":
      return "this PC";
    default:
      return "this computer";
  }
}
