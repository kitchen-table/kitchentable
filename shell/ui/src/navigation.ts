import type { Visibility } from "./generated";

/**
 * Where the window is pointed.
 *
 * A tagged union rather than a URL router: the shell is one window with a
 * handful of surfaces, and nothing outside the app ever needs to link into
 * one. The app detail tab lives in the surface so going back to an app returns
 * you to the tab you left.
 */
export type AppTab =
  | "overview"
  | "sharing"
  | "devices"
  | "activity"
  | "storage"
  | "versions";

export const APP_TABS: AppTab[] = [
  "overview",
  "sharing",
  "devices",
  "activity",
  "storage",
  "versions",
];

export const APP_TAB_LABELS: Record<AppTab, string> = {
  overview: "Overview",
  sharing: "Sharing",
  devices: "Devices",
  activity: "Activity",
  storage: "Storage",
  versions: "Versions",
};

export type Surface =
  | { kind: "library"; filter: Visibility | null }
  | { kind: "app"; slug: string; tab: AppTab }
  | { kind: "activity" }
  | { kind: "settings" }
  | { kind: "team" };

export const LIBRARY: Surface = { kind: "library", filter: null };

export function appSurface(slug: string, tab: AppTab = "overview"): Surface {
  return { kind: "app", slug, tab };
}

/** Which sidebar row should read as selected for a given surface. */
export function navSection(surface: Surface): "library" | "activity" | "settings" | "team" {
  switch (surface.kind) {
    // An app's detail view is still the library as far as the rail is
    // concerned; the app header carries its own "← Library" way out.
    case "library":
    case "app":
      return "library";
    default:
      return surface.kind;
  }
}
