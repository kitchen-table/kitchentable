import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { call } from "./daemon";

/**
 * A device that has asked to open something.
 *
 * Hand-written rather than generated: `device.list` serialises kt-auth's
 * `Device`, which is not a `TS` type. Keep in step with kt-auth/src/device.rs.
 */
export interface Device {
  id: string;
  /** Suggested from the user agent at first sight, editable afterwards. */
  name: string;
  status: "pending" | "approved" | "revoked" | "owner";
  /** Where `name` came from. Nothing ever overwrites `owner`. */
  named_by: "guess" | "network" | "owner";
  user_agent: string;
  /** Short and stable, so two similar devices can be told apart. */
  fingerprint: string;
  first_seen: number;
  last_seen: number;
}

/**
 * How quickly a waiting phone gets an answer, if nobody is watching events.
 *
 * `event.pairing_request` is what makes the prompt appear now, and it arrives
 * the moment the device is recorded rather than up to two seconds later. This
 * is the fallback for a window that has lost its subscription and not yet got
 * it back.
 */
const POLL_MS = 30_000;

export function useDevices(enabled = true) {
  return useQuery({
    queryKey: ["devices"],
    queryFn: () => call<Device[]>("device.list"),
    refetchInterval: POLL_MS,
    retry: false,
    enabled,
  });
}

/** Devices waiting on a decision, oldest first: whoever asked first is answered first. */
export function pending(devices: Device[] = []): Device[] {
  return devices
    .filter((device) => device.status === "pending")
    .sort((a, b) => a.first_seen - b.first_seen);
}

export function approved(devices: Device[] = []): Device[] {
  return devices
    .filter((device) => device.status === "approved" || device.status === "owner")
    .sort((a, b) => b.last_seen - a.last_seen);
}

export function revoked(devices: Device[] = []): Device[] {
  return devices.filter((device) => device.status === "revoked");
}

/**
 * Several rows that are almost certainly the same physical device.
 *
 * `sessions` is every row, newest first; `head` is the one the group is drawn
 * as. The group has no id of its own because it is not a thing the daemon knows
 * about - see [`group`].
 */
export interface DeviceGroup {
  /** The row the group is named and dated by: the most recently seen. */
  head: Device;
  sessions: Device[];
}

/**
 * Group rows that share a name and a browser.
 *
 * **For reading and for revoking. Never for granting.** A device id is minted
 * fresh whenever a request arrives without a usable cookie, so one phone that
 * has cleared its cookies a few times is several rows, and the list becomes
 * archaeology - which is what makes it useless for the one job it has, letting
 * an owner recognise what is in their house.
 *
 * What is matched on is a name and a user agent, and both are guessable and
 * forgeable. So this must never be the reason anybody is let *in*: approving
 * stays per-row, in the pairing prompt, where the owner is answering a specific
 * device that is asking right now. Revoking and forgetting apply to the whole
 * group, because doing less than the owner asked is the failure that matters
 * there - revoking one row of nine left the other eight approved.
 *
 * The owner's own machine is never grouped with anything. It is one row by
 * definition and merging it into a pile would put "This machine" on a group
 * that also contains strangers.
 */
export function group(devices: Device[] = []): DeviceGroup[] {
  const groups = new Map<string, Device[]>();
  for (const device of devices) {
    // A separator that cannot occur in either half. A space would merge a
    // device called "iPhone Safari" with one called "iPhone" on Safari, and
    // owners name their devices in whatever words they like.
    const key =
      device.status === "owner"
        ? `owner:${device.id}`
        : `${device.name}\u0000${device.user_agent}`;
    const existing = groups.get(key);
    if (existing) existing.push(device);
    else groups.set(key, [device]);
  }

  return [...groups.values()]
    .map((sessions) => {
      const ordered = [...sessions].sort((a, b) => b.last_seen - a.last_seen);
      return { head: ordered[0] as Device, sessions: ordered };
    })
    .sort((a, b) => b.head.last_seen - a.head.last_seen);
}

/** How many rows in a group, said the way the list shows it. */
export function sessionCount(group: DeviceGroup): string | null {
  const n = group.sessions.length;
  return n > 1 ? `${n} sessions` : null;
}

export function useDeviceActions() {
  const queryClient = useQueryClient();
  const settle = () => {
    void queryClient.invalidateQueries({ queryKey: ["devices"] });
    // Approving is the thing the Activity view most wants to show at once.
    void queryClient.invalidateQueries({ queryKey: ["log"] });
  };

  const approve = useMutation({
    mutationFn: ({ id, name }: { id: string; name?: string }) =>
      call("device.approve", { id, name }),
    onSuccess: settle,
  });

  const deny = useMutation({
    mutationFn: (id: string) => call("device.deny", { id }),
    onSuccess: settle,
  });

  const revoke = useMutation({
    mutationFn: (id: string) => call("device.revoke", { id }),
    onSuccess: settle,
  });

  const rename = useMutation({
    mutationFn: ({ id, name }: { id: string; name: string }) =>
      call("device.rename", { id, name }),
    onSuccess: settle,
  });

  /**
   * Remove rows from the list, one call each.
   *
   * Sequential rather than concurrent: the daemon holds one connection and
   * answers in order anyway, and a partial failure is easier to reason about
   * when the calls that already went through are the ones at the front.
   */
  const forget = useMutation({
    mutationFn: async (ids: string[]) => {
      for (const id of ids) await call("device.forget", { id });
    },
    onSuccess: settle,
  });

  /** Revoke every session in a group. Revoking one of nine left eight in. */
  const revokeAll = useMutation({
    mutationFn: async (ids: string[]) => {
      for (const id of ids) await call("device.revoke", { id });
    },
    onSuccess: settle,
  });

  return { approve, deny, revoke, revokeAll, rename, forget };
}

/**
 * Where a device's name came from, said plainly.
 *
 * Worth saying out loud rather than silently pre-filling a better name: for a
 * product whose pitch is that nothing leaves your desk, a name appearing from
 * nowhere is unsettling. It also tells the owner how much to trust it.
 */
export function nameSource(device: Device): string | null {
  switch (device.named_by) {
    case "network":
      return "the name this device gives itself on your network";
    case "guess":
      return "guessed from the browser — worth correcting";
    case "owner":
      return null;
  }
}

/**
 * "Safari · iOS", from a user agent.
 *
 * Only ever shown next to a fingerprint, so it is a hint rather than an
 * identification - the point is to help someone match the row to the phone in
 * their hand, not to be authoritative about what that phone is.
 */
export function describeAgent(userAgent: string): string {
  const ua = userAgent.toLowerCase();

  const browser = ua.includes("firefox")
    ? "Firefox"
    : ua.includes("edg/")
      ? "Edge"
      : ua.includes("chrome") || ua.includes("crios")
        ? "Chrome"
        : ua.includes("safari")
          ? "Safari"
          : ua.includes("curl")
            ? "curl"
            : "Browser";

  const system = ua.includes("iphone")
    ? "iPhone"
    : ua.includes("ipad")
      ? "iPad"
      : ua.includes("android")
        ? "Android"
        : ua.includes("mac os")
          ? "macOS"
          : ua.includes("windows")
            ? "Windows"
            : ua.includes("linux")
              ? "Linux"
              : null;

  return system ? `${browser} · ${system}` : browser;
}

/**
 * Which of the two device drawings a user agent has earned.
 *
 * The mockup draws a handset or a laptop and has no third shape, so this
 * chooses between them rather than describing anything. The handset is the
 * specific claim - a phone drawn beside "Chrome · Windows" is simply wrong, and
 * every row drew one - so only a string that actually puts the device in a hand
 * gets it. Everything else is a laptop, which covers the desktops and covers
 * `curl`, since that is somebody at a keyboard.
 *
 * Reads the same string as [`describeAgent`] and is wrong in the same places: a
 * modern iPad calls itself a Macintosh unless it says otherwise. Better a
 * laptop drawn for a tablet than a handset drawn for every Mac in the house.
 */
export function deviceShape(userAgent: string): "phone" | "laptop" {
  const ua = userAgent.toLowerCase();
  const handheld =
    ua.includes("iphone") ||
    ua.includes("ipad") ||
    ua.includes("android") ||
    ua.includes("mobile");
  return handheld ? "phone" : "laptop";
}
