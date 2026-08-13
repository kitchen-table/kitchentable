/**
 * The access log, as the window reads it.
 *
 * Hand-written rather than generated: `log.query` returns a JSON object built
 * in rpc.rs rather than a `TS`-derived struct, so there is nothing for ts-rs to
 * export yet. Keep this in step with that handler.
 */
export interface AccessEvent {
  /** Unix seconds. */
  at: number;
  app_slug?: string | null;
  device_id?: string | null;
  /** `owner`, `viewer`, `agent`, `cli`, `system`. */
  actor: string;
  /** `opened`, `paired`, `denied`, `revoked`, `deployed`. */
  action: string;
  detail?: string | null;
}

/** How each action reads in a list: a glyph, a colour, and a sentence. */
export const ACTION: Record<
  string,
  { glyph: string; colour: string; tint: string; title: string }
> = {
  opened: {
    glyph: "→",
    colour: "var(--accent)",
    tint: "var(--accent-tint)",
    title: "Opened",
  },
  paired: {
    glyph: "✓",
    colour: "var(--green)",
    tint: "var(--green-tint)",
    title: "Device paired",
  },
  denied: {
    glyph: "✕",
    colour: "var(--danger)",
    tint: "var(--danger-tint)",
    title: "Refused",
  },
  revoked: {
    glyph: "⊘",
    colour: "var(--gold)",
    tint: "var(--gold-tint)",
    title: "Access revoked",
  },
  deployed: {
    glyph: "↑",
    colour: "var(--strict)",
    tint: "var(--strict-tint)",
    title: "Deployed",
  },
};

export function describe(action: string) {
  return (
    ACTION[action] ?? {
      glyph: "·",
      colour: "var(--muted)",
      tint: "var(--chip)",
      title: action,
    }
  );
}

/** The activity filters the mockup puts above the per-app list. */
export const ACTIVITY_FILTERS = ["all", "opens", "deploys", "devices"] as const;
export type ActivityFilter = (typeof ACTIVITY_FILTERS)[number];

export function matches(event: AccessEvent, filter: ActivityFilter): boolean {
  switch (filter) {
    case "all":
      return true;
    case "opens":
      return event.action === "opened";
    case "deploys":
      return event.action === "deployed";
    case "devices":
      return ["paired", "denied", "revoked"].includes(event.action);
  }
}

/**
 * "just now", "4m", "2h", "3d".
 *
 * Short because these sit at the right edge of dense rows. Anything older than
 * a week gets a date, since "31d" stops meaning anything.
 */
export function ago(at: number, now: number = Date.now() / 1000): string {
  const seconds = Math.max(0, now - at);
  if (seconds < 45) return "just now";
  if (seconds < 3600) return `${Math.round(seconds / 60)}m`;
  if (seconds < 86_400) return `${Math.round(seconds / 3600)}h`;
  if (seconds < 604_800) return `${Math.round(seconds / 86_400)}d`;
  return new Date(at * 1000).toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
  });
}

/** "12.4 MB". Binary units, because that is what a file manager shows. */
export function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}
