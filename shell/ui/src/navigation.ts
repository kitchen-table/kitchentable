import type { Visibility } from "./generated";

/**
 * Where the window is pointed.
 *
 * A tagged union rather than a URL router: the shell is one window with a
 * handful of surfaces, and nothing outside the app ever needs to link into
 * one. The app detail tab lives in the surface so going back to an app returns
 * you to the tab you left.
 */
/**
 * The mockup draws a sixth tab, Versions, and it is deliberately absent.
 *
 * Nothing in the product creates a version: there is no deploy, no
 * `app.write_file` and no `kt deploy`, so the folder simply changes as its
 * owner edits it. A tab is a promise of a place, and one that could only ever
 * be empty is a worse lie than a missing tab - which is the same reasoning the
 * Sharing tab applies to Strict. Rollback would also be the first destructive
 * write Kitchen Table makes to somebody's own folder, which is a promise worth
 * spending on the agent story that needs it rather than on hand edits the
 * owner already controls.
 *
 * It comes back when an agent can write to a folder. Until then the deploys
 * filter in `activity.ts` stays disabled with its reason, and product.md
 * section 5.10 has been amended rather than left claiming a feature.
 */
export type AppTab = "overview" | "sharing" | "devices" | "activity" | "storage";

export const APP_TABS: AppTab[] = [
  "overview",
  "sharing",
  "devices",
  "activity",
  "storage",
];

export const APP_TAB_LABELS: Record<AppTab, string> = {
  overview: "Overview",
  sharing: "Sharing",
  devices: "Devices",
  activity: "Activity",
  storage: "Storage",
};

export type Surface =
  | { kind: "library"; filter: Visibility | null }
  | { kind: "app"; slug: string; tab: AppTab }
  | { kind: "activity" }
  | { kind: "notifications" }
  | { kind: "settings" };

export const LIBRARY: Surface = { kind: "library", filter: null };

export function appSurface(slug: string, tab: AppTab = "overview"): Surface {
  return { kind: "app", slug, tab };
}

/**
 * The rail's Team row is deliberately absent, for the same reason the Versions
 * tab is: it rendered and did nothing, and a row is a promise of a place. Team
 * is a cloud feature whose server half is not in this repo and must not be
 * (CLAUDE.md rule 10), so there was never going to be anything behind it here.
 * Settings' relay card is where the paid tier is described instead - one place
 * that says what the tier is, and nothing anywhere that looks live and is not.
 */

export const NOTIFICATIONS: Surface = { kind: "notifications" };

/** Which sidebar row should read as selected for a given surface. */
export function navSection(
  surface: Surface,
): "library" | "activity" | "notifications" | "settings" {
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
