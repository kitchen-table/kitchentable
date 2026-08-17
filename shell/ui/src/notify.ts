/**
 * Desktop notifications: what to post, and whether macOS will show it.
 *
 * The wording lives here rather than in Rust for the same reason the socket
 * types do: this is product copy, it changes with the design, and it is worth
 * being able to test on any machine. Rust posts what it is handed and reports
 * which button came back.
 *
 * Two rules shape everything below.
 *
 * The first is that a notification is a claim that something needs you. Only
 * the access request carries buttons, because it is the only event where the
 * answer is a decision rather than an acknowledgement - and it is the one the
 * marketing page promises you can settle "without opening the app".
 *
 * The second is that we never say permission is granted without having asked
 * macOS. [`NotifyState`] has a state for every honest answer, including the two
 * that are nobody's fault: a development build with no bundle, and a platform
 * this is not built for yet.
 */
import { useCallback, useEffect, useRef } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AccessEvent } from "./activity";
import { call } from "./daemon";
import { type Device, describeAgent } from "./devices";
import { EVENT } from "./events";
import type { App, Event, NotifyPrefs } from "./generated";

/** The Tauri event carrying a pressed button back. Matches notify.rs. */
const ACTION_EVENT = "kt://notification-action";

/**
 * What macOS will do with a notification from this app right now.
 *
 * `unbundled` is the state a developer sees: `pnpm tauri dev` runs a bare
 * binary, which has no bundle identifier for the notification framework to
 * attach to. `denied` cannot be undone from in here - macOS asks once and
 * remembers - so the surface points at System Settings instead of offering a
 * button that would do nothing.
 */
export interface NotifyState {
  state: "unsupported" | "unbundled" | "not_determined" | "denied" | "granted";
  /**
   * Whether banners are switched on, which is a separate setting from
   * permission. Granted with banners off means everything we post lands in
   * Notification Center unseen, and the surface has to say so rather than
   * showing a contented green tick.
   */
  alerts: boolean;
}

/** One notification, as Rust takes it. */
export interface Spec {
  title: string;
  body: string;
  subtitle?: string;
  /** Opaque to Rust; comes back on the action event. Always a device id here. */
  key?: string;
  actions?: Array<{ id: string; label: string }>;
  sound?: boolean;
}

export const APPROVE = "approve";
export const DENY = "deny";

/**
 * The banner for a device at the door, worded as the mockup draws it.
 *
 * The sentence is the title and the fingerprint line is the body, which is the
 * same split the in-app row uses: the sentence says what is happening, the mono
 * line is what you check the physical device against. Somebody comparing the
 * banner with the phone in their hand needs the second one.
 *
 * The app name is optional because it genuinely may not be known: devices are
 * trusted across the workspace and carry no app of their own, so the app comes
 * from the `requested` row in the access log, and a request can arrive before
 * that row is readable. "wants access" is true in both cases; inventing an app
 * name would not be.
 */
export function pairingSpec(device: Device, appName?: string): Spec {
  return {
    title: appName
      ? `${device.name} wants to open ${appName}`
      : `${device.name} wants access`,
    body: `fingerprint · ${device.fingerprint} · ${describeAgent(device.user_agent)}`,
    key: device.id,
    actions: [
      { id: APPROVE, label: "Approve" },
      { id: DENY, label: "Deny" },
    ],
    // The only notification this product makes a sound for. It is the only one
    // where somebody else is waiting on the answer.
    sound: true,
  };
}

/** "Priya's iPhone opened Trip Planner." Off by default; the mockup calls it chatty. */
export function openSpec(deviceName: string, appName: string): Spec {
  return {
    title: `${deviceName} opened ${appName}`,
    body: appName,
    subtitle: undefined,
  };
}

/**
 * Whether an event is worth a banner, given what the owner asked for.
 *
 * `deploys` and `offline` are stored and honoured and can never fire: nothing
 * writes a deploy (product.md 5.10) and nothing detects an app going offline.
 * They are switches over events that do not exist yet rather than switches that
 * are broken, and the surface says which is which.
 */
export function wanted(event: Event, prefs: NotifyPrefs): boolean {
  switch (event.event) {
    case "pairing_request":
      return prefs.requests;
    case "access":
      return prefs.opens && event.action === "opened";
    default:
      return false;
  }
}

/** The most recent app a device asked for, from the access log. */
export function requestedApp(
  log: AccessEvent[],
  deviceId: string,
): string | undefined {
  return log.find(
    (entry) => entry.device_id === deviceId && entry.action === "requested",
  )?.app_slug ?? undefined;
}

/** Ask Rust what macOS will do, without prompting anybody. */
export function useNotifyState() {
  return useQuery({
    queryKey: ["notify-state"],
    queryFn: () => invoke<NotifyState>("kt_notify_state"),
    // Someone can change this in System Settings while the window is open, and
    // the window would otherwise keep claiming notifications work.
    refetchInterval: 30_000,
    refetchOnWindowFocus: true,
    retry: false,
  });
}

/**
 * Show the system prompt.
 *
 * macOS asks once per install and answers from its own record afterwards, so
 * this is not a retry: a surface that offers it twice is offering a button that
 * does nothing the second time.
 */
export function useAskPermission() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: () => invoke<NotifyState>("kt_notify_request"),
    onSuccess: (state) => client.setQueryData(["notify-state"], state),
  });
}

export function useNotifyPrefs() {
  return useQuery({
    queryKey: ["notify-prefs"],
    queryFn: () => call<NotifyPrefs>("notify.prefs"),
    retry: false,
  });
}

export function useSetNotifyPrefs() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (patch: Partial<NotifyPrefs>) =>
      call<NotifyPrefs>("notify.set_prefs", patch),
    // The daemon answers with all four, so the cache takes its word rather
    // than the patch we sent.
    onSuccess: (prefs) => client.setQueryData(["notify-prefs"], prefs),
  });
}

/** Post one notification. Silently does nothing where they are unsupported. */
export function show(spec: Spec): void {
  void invoke("kt_notify", { spec }).catch(() => {
    // No IPC - the UI also runs in a plain browser and under vitest. A missing
    // banner is not worth taking the window down for.
  });
}

/**
 * Post banners for daemon events, and act on the buttons.
 *
 * Mounted once, beside [`useDaemonEvents`], and for the same reason: an event
 * that arrives while the Notifications surface is closed still has to reach the
 * desktop. The window being hidden is the normal case here - that is what the
 * whole feature is for.
 *
 * Approving from a banner goes back over the socket exactly as the in-app
 * button does. There is no second path into that decision, which is what keeps
 * the access log honest about how a device got in.
 */
export function useDesktopNotifications(prefs?: NotifyPrefs): void {
  const client = useQueryClient();

  // Read through a ref so the subscription is not torn down and rebuilt every
  // time a switch moves; a re-subscribe races with the events it exists to
  // catch.
  const current = useRef(prefs);
  current.current = prefs;

  const post = useCallback(async (event: Event) => {
    const prefs = current.current;
    if (!prefs || !wanted(event, prefs)) return;

    if (event.event === "pairing_request") {
      const devices = await call<Device[]>("device.list");
      const device = devices.find((candidate) => candidate.id === event.device_id);
      // Gone already: approved from another window, or denied between the
      // event and this call. Nothing to ask about.
      if (!device || device.status !== "pending") return;

      const log = await call<AccessEvent[]>("log.query", { limit: 50 });
      const slug = requestedApp(log, device.id);
      const apps = slug ? await call<App[]>("app.list") : [];
      show(pairingSpec(device, apps.find((app) => app.slug === slug)?.name ?? slug));
      return;
    }

    if (event.event === "access" && event.app_slug) {
      const apps = await call<App[]>("app.list");
      const app = apps.find((candidate) => candidate.slug === event.app_slug);
      if (app) show(openSpec("Someone", app.name));
    }
  }, []);

  useEffect(() => {
    let stops: Array<() => void> = [];
    let gone = false;

    try {
      void Promise.all([
        listen<Event>(EVENT, ({ payload }) => void post(payload)),
        listen<{ key?: string; action: string }>(ACTION_EVENT, ({ payload }) => {
          if (!payload.key) return;
          const method =
            payload.action === APPROVE
              ? "device.approve"
              : payload.action === DENY
                ? "device.deny"
                : null;
          // Dismissed, timed out, or the body was clicked. All of those leave
          // the request standing, which is the safe direction: a device is let
          // in only by somebody saying so.
          if (!method) return;

          void call(method, { id: payload.key })
            .catch(() => {
              // Answered elsewhere in the meantime. The list below is right
              // either way, so the refetch covers it.
            })
            .finally(() => {
              void client.invalidateQueries({ queryKey: ["devices"] });
              void client.invalidateQueries({ queryKey: ["log"] });
            });
        }),
      ])
        .then((listeners) => {
          if (gone) listeners.forEach((stop) => stop());
          else stops = listeners;
        })
        .catch(() => {
          // No IPC. Same as everywhere else: the window still works.
        });
    } catch {
      // Same.
    }

    return () => {
      gone = true;
      stops.forEach((stop) => stop());
    };
  }, [client, post]);
}
