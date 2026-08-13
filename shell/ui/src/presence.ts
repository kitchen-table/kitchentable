/**
 * Who has an app open right now.
 *
 * Kept current by `event.presence` rather than by asking repeatedly: the query
 * is here for the first paint and for anything a lost subscription missed.
 */
import { useQuery } from "@tanstack/react-query";
import type { Viewer } from "./generated";
import { call } from "./daemon";

/**
 * The id the daemon uses for the owner's own browser.
 *
 * It has no device record because it never had to pair with itself, so it
 * would otherwise be a viewer with no name.
 */
export const OWNER = "owner";

export function usePresence(slug: string, enabled = true) {
  return useQuery({
    queryKey: ["presence", slug],
    queryFn: () => call<Viewer[]>("presence.list", { slug }),
    retry: false,
    enabled,
  });
}

/**
 * What to call whoever is reading it.
 *
 * The device name where we have one, because that is what the owner named at
 * the pairing prompt and what they will recognise.
 */
export function viewerName(viewer: Viewer, deviceNames: Map<string, string>): string {
  if (viewer.device_id === OWNER) return "This machine";
  return deviceNames.get(viewer.device_id) ?? "Someone";
}

/**
 * The page someone is on, as a person would say it.
 *
 * The root is "the app" rather than "/", because "viewing /" reads like a
 * filesystem and this is a sentence about a person.
 */
export function viewerWhere(viewer: Viewer): string {
  // Tolerant of a missing path rather than trusting the shape. This is one
  // cosmetic line in a panel, and throwing here would blank the whole app
  // detail view - a bad trade for a field that is only ever decoration.
  const path = (viewer.path ?? "").replace(/\/+$/, "");
  if (path === "" || path === "/index.html") return "the app";
  return path.replace(/^\//, "").replace(/\.html?$/, "");
}

/**
 * How recently they checked in.
 *
 * Anything inside a beat is "just now": the page checks in on a timer, so a
 * number that ticked 1s, 2s, 3s would be counting the timer rather than saying
 * anything about the person.
 */
export function viewerWhen(viewer: Viewer): string {
  return (viewer.seconds_ago ?? 0) < 15 ? "just now" : `${viewer.seconds_ago}s ago`;
}
