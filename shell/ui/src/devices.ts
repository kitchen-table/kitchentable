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

  return { approve, deny, revoke, rename };
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
