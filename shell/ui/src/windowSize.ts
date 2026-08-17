/**
 * The window's two shapes.
 *
 * Onboarding is a panel, not an application: eight screens of one column, laid
 * out in `Onboarding - macOS.dc.html` at 960 × 640. Left in the library's own
 * 1240 × 820 it is a small amount of text stranded in a large empty window,
 * which is what it looked like before this existed.
 *
 * So the window is resized for the flow and put back afterwards. Resized rather
 * than a second window: one window means the app that appears when onboarding
 * finishes is the same object that was on screen a moment earlier, in the same
 * place, rather than something that vanishes and is replaced.
 *
 * It is also fixed while onboarding runs. There is nothing to reflow to - the
 * column has one width - and a resizable panel invites somebody to drag it into
 * a shape the flow was never drawn at.
 */
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

/** From the onboarding mockup's frame. */
export const ONBOARDING = { width: 960, height: 640 };

/** From `tauri.conf.json`, which is where the app's own size is declared. */
export const APP = { width: 1240, height: 820 };
export const APP_MINIMUM = { width: 880, height: 600 };

/**
 * Apply a shape, centring the window on it.
 *
 * Every call is best-effort. The same UI runs in a browser under `pnpm dev` and
 * under vitest, where there is no window to resize and no IPC to ask - and a
 * window that cannot be resized is not a reason to fail to render one.
 */
async function apply(
  size: { width: number; height: number },
  options: { fixed: boolean },
): Promise<void> {
  try {
    const window = getCurrentWindow();
    // The minimum first, and never above the size we are about to set: macOS
    // clamps to the minimum, so shrinking to 960 under an 880-wide floor is
    // fine, but a larger floor would silently win.
    await window.setMinSize(
      new LogicalSize(
        Math.min(APP_MINIMUM.width, size.width),
        Math.min(APP_MINIMUM.height, size.height),
      ),
    );
    await window.setSize(new LogicalSize(size.width, size.height));
    await window.setResizable(!options.fixed);
    await window.center();
  } catch {
    // No window to size. See above.
  }
}

/** The onboarding panel: fixed, and the size it was drawn at. */
export function sizeForOnboarding(): Promise<void> {
  return apply(ONBOARDING, { fixed: true });
}

/** The application proper: resizable, with its floor restored. */
export async function sizeForApp(): Promise<void> {
  await apply(APP, { fixed: false });
  try {
    await getCurrentWindow().setMinSize(
      new LogicalSize(APP_MINIMUM.width, APP_MINIMUM.height),
    );
  } catch {
    // Same.
  }
}
