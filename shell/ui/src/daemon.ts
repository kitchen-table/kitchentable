//! The UI's view of the daemon.

import { invoke } from "@tauri-apps/api/core";
import { useQuery } from "@tanstack/react-query";
import type { App, SysStatus } from "./generated";

/** A typed error from the daemon, or a transport failure dressed as one. */
export interface DaemonError {
  code: string;
  message: string;
}

export type Health = (
  | { state: "ready" }
  | { state: "not_running" }
  | { state: "wrong_version"; theirs: number; ours: number }
  | { state: "unhealthy"; reason: string }
) & {
  /** Whether this shell spawned the daemon, or adopted one already running. */
  we_started_it?: boolean;
};

/**
 * Call a socket method through the shell.
 *
 * Everything the window shows comes through here, which is the rule that keeps
 * the socket honest: if the UI can do it, the CLI and an agent can too.
 */
export async function call<T>(method: string, params?: unknown): Promise<T> {
  try {
    return await invoke<T>("kt_call", { method, params: params ?? null });
  } catch (raw) {
    throw asDaemonError(raw);
  }
}

function asDaemonError(raw: unknown): DaemonError {
  if (typeof raw === "object" && raw !== null && "code" in raw) {
    return raw as DaemonError;
  }
  return { code: "unavailable", message: String(raw) };
}

/**
 * A safety net, not the mechanism.
 *
 * `event.subscribe` is what keeps these current now, so this is only here for
 * what a subscription cannot cover: the seconds between the daemon going away
 * and the shell noticing, and anything a lagging subscriber dropped. Minutes
 * rather than seconds, because a poll this slow costs nothing and a poll fast
 * enough to matter would mean the events were not working.
 */
const POLL_MS = 60_000;

/**
 * How often to ask again while the daemon is *not* ready.
 *
 * First run is the case this exists for. A daemon starting for the first time
 * has a database to create, migrations to run and keys to mint, and it may be
 * sitting behind the Local Network dialog while it does - so the window's first
 * health check can easily lose the race. At the minute-long poll that left
 * somebody's very first sight of Kitchen Table as an error screen saying the
 * background service was not running, for a full minute, when it was starting
 * normally and would be up in seconds.
 */
const STARTING_POLL_MS = 700;

export function useHealth() {
  return useQuery({
    queryKey: ["health"],
    queryFn: () => invoke<Health>("kt_health"),
    // Slow once it is up, because then this is only a safety net; quick while
    // it is not, because then it is the thing the window is waiting on.
    refetchInterval: (query) =>
      query.state.data?.state === "ready" ? POLL_MS : STARTING_POLL_MS,
    // A daemon that is down is a normal state, not an error to retry-storm.
    retry: false,
  });
}

export function useApps(enabled: boolean) {
  return useQuery({
    queryKey: ["apps"],
    queryFn: () => call<App[]>("app.list"),
    refetchInterval: POLL_MS,
    retry: false,
    enabled,
  });
}

export function useStatus(enabled: boolean) {
  return useQuery({
    queryKey: ["status"],
    queryFn: () => call<SysStatus>("sys.status"),
    refetchInterval: POLL_MS,
    retry: false,
    enabled,
  });
}

export function restartDaemon(): Promise<void> {
  return invoke("kt_restart_daemon");
}

/**
 * Quit the whole product, daemon included.
 *
 * Distinct from closing the window, which only hides it. This is the one place
 * in the UI that stops serving, so it is deliberately not on a toolbar.
 */
export function quit(): Promise<void> {
  return invoke("kt_quit");
}
