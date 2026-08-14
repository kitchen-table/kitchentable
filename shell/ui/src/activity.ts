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

/**
 * How each action reads in a list: a glyph, a colour, and two forms of words.
 *
 * `title` stands alone. `phrase` completes a sentence about whoever caused it -
 * "Kitchen iPad opened the app" - which is the form the mockup uses, because a
 * column of the bare word "Opened" answers the least interesting half of the
 * question. Keep the keys in step with what the daemon actually writes; a
 * missing one falls through to [`describe`] and shows up as a raw lowercase
 * verb next to a meaningless dot.
 */
export const ACTION: Record<
  string,
  { glyph: string; colour: string; tint: string; title: string; phrase: string }
> = {
  opened: {
    glyph: "→",
    colour: "var(--accent)",
    tint: "var(--accent-tint)",
    title: "Opened",
    phrase: "opened the app",
  },
  // Someone opened an invite link and is waiting on the owner. Written by
  // redemption, and the row that explains why a prompt appeared.
  requested: {
    glyph: "?",
    colour: "var(--gold)",
    tint: "var(--gold-tint)",
    title: "Asked to be let in",
    phrase: "asked to be let in",
  },
  paired: {
    glyph: "✓",
    colour: "var(--green)",
    tint: "var(--green-tint)",
    title: "Device paired",
    phrase: "was let in",
  },
  // The owner said no to a pairing request. Rarer than `refused`, and a
  // deliberate act rather than a rule being applied.
  denied: {
    glyph: "✕",
    colour: "var(--danger)",
    tint: "var(--danger-tint)",
    title: "Denied",
    phrase: "was denied",
  },
  // The gate turned a request away. This is the common negative event - every
  // blocked open writes one - and it had no entry here at all, so it rendered
  // as the raw word "refused" beside a dot.
  refused: {
    glyph: "✕",
    colour: "var(--danger)",
    tint: "var(--danger-tint)",
    title: "Turned away",
    phrase: "was turned away",
  },
  revoked: {
    glyph: "⊘",
    colour: "var(--gold)",
    tint: "var(--gold-tint)",
    title: "Access revoked",
    phrase: "had access revoked",
  },
  // Nothing writes this yet; it arrives with versioning in D6. Kept because
  // the glyph and wording are settled, and an entry costs nothing until then.
  deployed: {
    glyph: "↑",
    colour: "var(--strict)",
    tint: "var(--strict-tint)",
    title: "Deployed",
    phrase: "deployed a new version",
  },
};

export function describe(action: string) {
  return (
    ACTION[action] ?? {
      glyph: "·",
      colour: "var(--muted)",
      tint: "var(--chip)",
      title: action,
      phrase: action,
    }
  );
}

/**
 * What to call whoever caused an event.
 *
 * The device name when we know it, because that is what the owner recognises -
 * they named it themselves at the pairing prompt. Falling back to the actor
 * rather than the device id: "Someone" is vague but true, and a base64 id in a
 * sentence is neither.
 */
export function who(event: AccessEvent, deviceName?: string): string {
  if (deviceName) return deviceName;
  switch (event.actor) {
    case "owner":
      return "You";
    case "agent":
      return "An agent";
    case "cli":
      return "The CLI";
    case "system":
      return "Kitchen Table";
    default:
      return "Someone";
  }
}

/** "Kitchen iPad opened the app". The row title the mockup draws. */
export function sentence(event: AccessEvent, deviceName?: string): string {
  return `${who(event, deviceName)} ${describe(event.action).phrase}`;
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

/**
 * "expires 21 Aug", not "expires".
 *
 * A chip that says a link expires without saying when is worse than no chip:
 * it raises the question and refuses to answer it.
 */
export function expiry(at: number | null): string {
  if (at === null) return "no expiry";
  return `expires ${new Date(at * 1000).toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
  })}`;
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
