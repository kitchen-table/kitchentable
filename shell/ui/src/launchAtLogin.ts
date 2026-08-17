/**
 * Whether Kitchen Table starts itself when the Mac starts.
 *
 * The one setting in this product that lives entirely outside the daemon. The
 * daemon cannot register itself for login - a LaunchAgent belongs to the app
 * the user installed, not to a binary it happens to spawn - so this is the
 * exception to "new UI capability means a new socket method first" and it is
 * the shell's own business.
 *
 * **Read, never remembered.** `isEnabled()` asks the system on every mount
 * rather than trusting a value cached here, because the LaunchAgent can change
 * without this window: System Settings has its own login-items list, an
 * uninstall removes the plist, and a reinstall may not restore it. A toggle
 * drawing from memory would be confidently wrong, which is worse than slow.
 */
import { useCallback, useEffect, useState } from "react";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { hasTauri } from "./external";

/**
 * What the switch knows.
 *
 * `null` is "not asked yet", and it is deliberately distinct from `false`: a
 * switch drawn off before the answer arrives says the setting is off, which is
 * a claim rather than a delay. `available: false` means there is no host to ask
 * - a plain browser during `pnpm dev`, or vitest - and the row says so instead
 * of offering a control that cannot work.
 */
export interface LaunchAtLogin {
  on: boolean | null;
  available: boolean;
  busy: boolean;
  error: string | null;
  toggle: () => void;
}

export function useLaunchAtLogin(): LaunchAtLogin {
  const available = hasTauri();
  const [on, setOn] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const read = useCallback(async () => {
    if (!available) return;
    try {
      setOn(await isEnabled());
      setError(null);
    } catch (e) {
      // Not knowing is a state. Reporting it beats drawing a switch whose
      // position is a guess.
      setOn(null);
      setError(message(e));
    }
  }, [available]);

  useEffect(() => {
    void read();
  }, [read]);

  const toggle = useCallback(() => {
    if (!available || on === null || busy) return;
    setBusy(true);
    void (async () => {
      try {
        if (on) await disable();
        else await enable();
        setError(null);
      } catch (e) {
        setError(message(e));
      } finally {
        setBusy(false);
        // Re-read rather than assuming the write took. This registers a file
        // with the OS, and "the call returned" is not the same fact as "the
        // agent is registered" - the same lesson the daemon learned writing
        // session keys to the Keychain.
        await read();
      }
    })();
  }, [available, on, busy, read]);

  return { on, available, busy, error, toggle };
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
