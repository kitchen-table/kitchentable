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
  // The owner tidying the list rather than refusing anybody. Worth its own
  // wording: "removed from the list" is what happened, and reading it as a
  // revocation would be wrong - a forgotten device may ask again.
  forgotten: {
    glyph: "−",
    colour: "var(--muted)",
    tint: "var(--chip)",
    title: "Removed from the list",
    phrase: "was removed from the device list",
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

/**
 * What to call whoever caused an event, in the mono sub-line.
 *
 * Lower case and terse, because it sits at the head of a row of fragments
 * rather than starting a sentence - the sentence is the line above it.
 */
const ACTOR: Record<string, string> = {
  owner: "you",
  viewer: "device",
  agent: "agent",
  cli: "cli",
  system: "kitchen table",
};

/**
 * The mono sub-line under an activity row: who, which device, and any detail.
 *
 * The device's fingerprint rather than its id, because the fingerprint is what
 * the Devices tab and the pairing prompt both show - somebody comparing a row
 * here against a row there needs the same string in both places. The id is
 * neither short nor meaningful to a person.
 *
 * Takes the device's bits already looked up rather than a `Device`, so this
 * module stays free of the query layer and can be tested on plain values.
 */
export function meta(
  event: AccessEvent,
  device?: { fingerprint?: string; agent?: string },
): string {
  const parts = [ACTOR[event.actor] ?? event.actor];
  if (device?.fingerprint) parts.push(device.fingerprint);
  if (device?.agent) parts.push(device.agent);
  if (event.detail) parts.push(event.detail);
  return parts.join(" · ");
}

/** The activity filters the mockup puts above the per-app list. */
export const ACTIVITY_FILTERS = ["all", "opens", "deploys", "devices"] as const;
export type ActivityFilter = (typeof ACTIVITY_FILTERS)[number];

export const FILTER_LABELS: Record<ActivityFilter, string> = {
  all: "All",
  opens: "Opens",
  deploys: "Deploys",
  devices: "Devices",
};

/**
 * Filters that would answer "nothing here" whatever happened, and say so.
 *
 * The daemon never writes a `deployed` row - that action arrives with
 * versioning - so this chip can only ever empty the list. Shown disabled with
 * the reason rather than dropped, which is what the Sharing tab does with
 * Strict: the mockup's shape is kept, and nothing pretends to work. Delete this
 * list the moment something writes a deploy.
 */
export const UNAVAILABLE_FILTERS: ActivityFilter[] = ["deploys"];

export function filterAvailable(filter: ActivityFilter): boolean {
  return !UNAVAILABLE_FILTERS.includes(filter);
}

export function matches(event: AccessEvent, filter: ActivityFilter): boolean {
  switch (filter) {
    case "all":
      return true;
    case "opens":
      return event.action === "opened";
    case "deploys":
      return event.action === "deployed";
    case "devices":
      // Everything that changed who may open something - including the request
      // that started it, and the refusals that follow a revocation. Filtering
      // to the owner's own decisions would hide the events that explain them.
      return [
        "requested",
        "paired",
        "denied",
        "revoked",
        "refused",
        "forgotten",
      ].includes(event.action);
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
 * "2m ago", "just now", "12 Aug" - the form the activity rows use.
 *
 * [`ago`] is written for the right edge of a dense row, where every character
 * costs; an activity list has room for the word and reads better with it. Only
 * the elapsed forms take it: "just now ago" and "12 Aug ago" are both wrong, so
 * the suffix goes on when the string starts with a number and not otherwise.
 */
export function when(at: number, now?: number): string {
  const short = ago(at, now);
  return /^\d/.test(short) ? `${short} ago` : short;
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
