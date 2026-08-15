/**
 * Opening a URL outside the window.
 *
 * `target="_blank"` does nothing in the desktop app. A webview has no tabs and
 * no window to open a second one into, so the click is simply swallowed - no
 * error, no navigation, nothing. That is what made the app header's **Open**
 * link look broken: it is a perfectly good anchor pointing at a URL that serves
 * 200, and in the one place it matters most it did not go anywhere.
 *
 * Tauri's answer is the opener plugin, which hands the URL to the OS. It is
 * already a dependency, already registered in `lib.rs`, and `opener:default` in
 * the capability file already permits `http:` and `https:` - only the call from
 * this side was missing.
 *
 * Invoked by command name rather than through `@tauri-apps/plugin-opener`,
 * which would be a second package for one call whose shape is fixed by the
 * plugin's own `open_url(url, with)`.
 */
import { invoke } from "@tauri-apps/api/core";

/**
 * Whether there is a Tauri host to ask.
 *
 * The same UI runs in a plain browser - `pnpm -C shell/ui dev` - and under
 * vitest, and in neither is there an IPC. That is not a degraded mode to
 * apologise for: in a browser, an anchor already opens a tab correctly, so the
 * right thing there is to leave the click alone.
 */
export function hasTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Hand a URL to whatever the OS opens it with. */
export async function openExternal(url: string): Promise<void> {
  await invoke("plugin:opener|open_url", { url });
}

/**
 * An `onClick` for an anchor that must open outside the window.
 *
 * Kept as an anchor rather than turned into a button: the href is what makes
 * the address copyable, hoverable and readable in the status bar, and it is
 * what makes the browser case work with no special path at all. This only takes
 * the click when there is a host that needs it taken.
 */
export function openInstead(url: string, onError?: (error: unknown) => void) {
  return (event: { preventDefault: () => void }) => {
    if (!hasTauri()) return;
    event.preventDefault();
    // One handler covers both ways this can go wrong. `invoke` does reach for
    // the IPC synchronously - which is what took the window down in the events
    // module - but it does so inside an `async` function here, so a synchronous
    // throw arrives as a rejection like any other. A `try` around this would
    // read as a second hazard that does not exist.
    void openExternal(url).catch((error) => onError?.(error));
  };
}
