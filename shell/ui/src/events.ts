/**
 * Daemon events, as the window hears them.
 *
 * The shell subscribes on the socket and re-emits everything under one Tauri
 * event; this decides what each one invalidates. Deliberately a cache
 * invalidation layer rather than a second copy of the state: an event says
 * something changed, and the query that already knows how to fetch it goes and
 * fetches it. Two paths writing the same state is how they come to disagree.
 */
import { useEffect } from "react";
import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import type { Event } from "./generated";

/** The Tauri event the shell re-emits daemon events under. */
export const EVENT = "kt://event";

/**
 * Emitted when the shell reconnects after losing the daemon.
 *
 * Whatever happened while nobody was listening was missed, so everything is
 * suspect - a restarted daemon may have a different library, different URLs and
 * a different port.
 */
export const RESYNC = "kt://resync";

/** What each event makes stale. */
export function invalidate(client: QueryClient, event: Event): void {
  switch (event.event) {
    case "app_changed":
    case "app_removed":
      void client.invalidateQueries({ queryKey: ["apps"] });
      break;
    case "pairing_request":
      // Somebody is on a wait page right now. This is the event the whole
      // subscription exists for.
      void client.invalidateQueries({ queryKey: ["devices"] });
      break;
    case "access":
      void client.invalidateQueries({ queryKey: ["log"] });
      break;
    case "presence":
      // Someone arrived, left, or moved to another page. The event carries the
      // whole list, but refetching keeps one path into this state rather than
      // two that can drift.
      void client.invalidateQueries({ queryKey: ["presence", event.app_slug] });
      break;
    case "serving_changed":
      void client.invalidateQueries({ queryKey: ["status"] });
      break;
    case "error":
      // Nothing to refetch: the daemon is telling the owner something, and
      // dropping it silently is better than guessing which view it belongs to.
      console.warn("daemon:", event.message);
      break;
  }
}

/**
 * Subscribe for as long as the window is open.
 *
 * Mounted once, at the top of the tree. Listening per-surface would mean an
 * event arriving while a surface is closed is simply missed.
 *
 * Survives having no Tauri IPC to listen on. The same UI runs in a plain
 * browser - `pnpm -C shell/ui dev`, which CLAUDE.md documents - and under
 * vitest, and in neither is there an IPC for `listen` to attach to. Without
 * live updates the window still works: every query has a fallback poll, which
 * is exactly what it had before events existed. Throwing on mount instead
 * would take the whole window down for the sake of an improvement.
 */
export function useDaemonEvents(): void {
  const client = useQueryClient();

  useEffect(() => {
    let stops: Array<() => void> = [];
    let gone = false;

    // `listen` reaches for the IPC synchronously, so this needs both a
    // try/catch and a rejection handler.
    try {
      void Promise.all([
        listen<Event>(EVENT, ({ payload }) => invalidate(client, payload)),
        listen(RESYNC, () => void client.invalidateQueries()),
      ])
        .then((listeners) => {
          // Unmounted before the listeners attached: stop them immediately
          // rather than leaking a subscription onto a dead component.
          if (gone) listeners.forEach((stop) => stop());
          else stops = listeners;
        })
        .catch(() => {
          // No IPC. The polls carry it.
        });
    } catch {
      // Same.
    }

    return () => {
      gone = true;
      stops.forEach((stop) => stop());
    };
  }, [client]);
}
