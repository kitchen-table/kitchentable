import type { Visibility } from "./generated";

/**
 * How each visibility level is shown.
 *
 * `network` is labelled **Household** because that is what it means: networks
 * the owner marked as home, not whatever network the laptop is on right now
 * (docs/product.md section 5.4). The wire value stays `network`.
 */
export const VISIBILITY: Record<
  Visibility,
  { label: string; colour: string; tint: string; blurb: string }
> = {
  private: {
    label: "Private",
    colour: "var(--vis-private)",
    tint: "var(--vis-private-tint)",
    blurb: "Your own devices only",
  },
  network: {
    label: "Household",
    colour: "var(--vis-household)",
    tint: "var(--vis-household-tint)",
    blurb: "Anyone on a network you've marked as home",
  },
  invited: {
    label: "Invited",
    colour: "var(--vis-invited)",
    tint: "var(--vis-invited-tint)",
    blurb: "Link holders, on approved devices",
  },
  public: {
    label: "Public",
    colour: "var(--vis-public)",
    tint: "var(--vis-public-tint)",
    blurb: "Anyone with the URL, anywhere",
  },
};

export const VISIBILITY_ORDER: Visibility[] = [
  "private",
  "network",
  "invited",
  "public",
];

/**
 * A stable colour for an app's tile, derived from its slug.
 *
 * Deterministic so an app does not change colour between launches, and drawn
 * from the brand palette rather than random hues.
 */
const TILE_COLOURS = [
  "#2f6df0",
  "#2f9e6f",
  "#b8862f",
  "#8b5cf6",
  "#e0663b",
  "#6b6b6b",
];

export function tileColour(slug: string): string {
  let hash = 0;
  for (const ch of slug) {
    hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  }
  return TILE_COLOURS[hash % TILE_COLOURS.length] ?? "#2f6df0";
}
