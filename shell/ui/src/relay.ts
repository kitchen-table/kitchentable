import type { App, RelayMode, RelayState, Visibility } from "./generated";

/**
 * Which address to put in front of somebody holding a phone.
 *
 * The QR code is the "take this away with you" affordance, so for a published
 * app the public address is the one worth handing over: it works in the
 * kitchen *and* on the train, where the `.local` name stops resolving the
 * moment they leave.
 *
 * Gated on the tunnel actually being up, which is the whole point of knowing.
 * Handing somebody a code that 502s while a working `.local` one was available
 * would be a worse answer than not offering the public name at all.
 */
export function handover(
  app: App,
  tunnel: RelayState | undefined,
): { url: string; away: boolean; alternative?: string } {
  if (app.public_url && tunnel?.state === "connected") {
    return { url: app.public_url, away: true, alternative: app.url };
  }
  return {
    url: app.url,
    away: false,
    // Only when there is no `.local` to fall back from. Android is the usual
    // reason a hostname never resolves.
    alternative: app.hostname === undefined ? app.fallback_url : undefined,
  };
}

/**
 * Whether an app is reachable away from home, and on what terms.
 *
 * The two live modes are not a performance setting. Serving a snapshot while
 * the owner's machine sleeps requires the edge to hold servable plaintext, and
 * end-to-end encryption requires that it cannot. Both cannot be true of the
 * same app, so the owner chooses - at the moment they switch the relay on,
 * never by a silent default for the sensitive case.
 */
export const RELAY: Record<
  Exclude<RelayMode, "off">,
  {
    label: string;
    badge: string;
    colour: string;
    tint: string;
    border: string;
    /** The one-line promise, for the modal. */
    promise: string;
    pitch: string;
    /**
     * Shown on the mode cards. `warn` is the cost of choosing this one; `soon`
     * is a promise of the mode that has no implementation behind it yet, and is
     * marked rather than ticked so the card does not claim it works.
     */
    points: { text: string; bold?: string; warn?: boolean; soon?: boolean }[];
    note: { head: string; body: string };
  }
> = {
  standard: {
    label: "Stay reachable",
    badge: "STANDARD",
    colour: "var(--accent)",
    tint: "var(--accent-tint)",
    border: "var(--accent-bd)",
    promise: "Standard",
    pitch:
      "Encrypted in transit, and Kitchen Table can technically read it. Meant to be served from an encrypted-at-rest snapshot while you sleep — that part is not built yet.",
    points: [
      {
        bold: "Snapshot failover",
        text: "encrypted-at-rest copy is served while your Mac sleeps",
        soon: true,
      },
      { text: "Encrypted in transit" },
      {
        text: "TLS terminates at the edge — it can technically read this app",
        warn: true,
      },
    ],
    note: {
      head: "Encrypted in transit — not end-to-end.",
      body: "TLS terminates at the edge and forwards to your daemon over the tunnel, so the edge can technically read this app's content. Snapshot failover — the part that keeps links alive while your Mac sleeps — is not built yet, so today this app is reachable only while the daemon is running.",
    },
  },
  strict: {
    label: "Full privacy",
    badge: "STRICT · E2E",
    colour: "var(--strict)",
    tint: "var(--strict-tint)",
    border: "var(--strict-bd)",
    promise: "Strict",
    pitch:
      "End-to-end via SNI passthrough — the operator cannot read it. The link goes dark when this machine sleeps.",
    points: [
      {
        bold: "SNI passthrough",
        text: "the edge routes ciphertext by hostname only",
      },
      { text: "TLS terminates inside your daemon — the edge can't read anything" },
      { text: "No snapshot failover — offline means offline", warn: true },
    ],
    note: {
      head: "End-to-end. The edge cannot read this app.",
      body: "The relay routes ciphertext by hostname (SNI) only; the TLS session terminates inside your daemon, which holds the certificate. The cost: no snapshot failover — when your Mac sleeps, the app is offline.",
    },
  },
};

/**
 * What the badge on a published app should say.
 *
 * **Publishing is an intention; being reachable is a fact.** The badge used to
 * render from the manifest, so an app whose tunnel had been down for an hour
 * still showed a confident green ON and the owner found out from somebody
 * telling them the link was broken. `published` is the owner's setting and
 * `tunnel` is what is actually true; this is where they meet.
 */
export function reachability(
  published: boolean,
  tunnel: RelayState | undefined,
): {
  label: string;
  colour: string;
  tint: string;
  /** Solid for a settled state, pulsing while it is still trying. */
  busy: boolean;
  detail?: string;
} {
  if (!published) {
    return { label: "OFF", colour: "var(--muted)", tint: "var(--chip)", busy: false };
  }

  // Undefined means the window has not heard back yet - which is not the same
  // as off, and must not be drawn as though the answer were known.
  switch (tunnel?.state) {
    case "connected":
      return {
        label: "ON",
        colour: "var(--green)",
        tint: "var(--green-tint)",
        busy: false,
      };
    case "connecting":
      return {
        label: "RECONNECTING",
        colour: "var(--gold)",
        tint: "var(--gold-tint)",
        busy: true,
        detail: "Published, but not reachable from outside until this finishes.",
      };
    case "needs_attention":
      return {
        label: "OFFLINE",
        colour: "var(--danger)",
        tint: "var(--danger-tint)",
        busy: false,
        detail: tunnel.message,
      };
    case "off":
      // Published, but this daemon has no relay configured at all, so nothing
      // is dialling and nothing will start.
      return {
        label: "NOT DIALLING",
        colour: "var(--danger)",
        tint: "var(--danger-tint)",
        busy: false,
        detail:
          "This app is published but this machine has no relay configured, so nothing is reachable from outside.",
      };
    default:
      return {
        label: "CHECKING",
        colour: "var(--muted)",
        tint: "var(--chip)",
        busy: true,
      };
  }
}

/**
 * Strict mode has no transport yet.
 *
 * The edge terminates TLS today, so there is nothing behind this button but
 * Standard. Offering it and quietly serving Standard would be the worst lie
 * this product could tell: the owner would have chosen the mode whose entire
 * point is that the operator cannot read the app, and been given the one where
 * it can. Shown and disabled rather than hidden, so the promise is visible and
 * its absence is dated.
 */
export const STRICT_AVAILABLE = false;

/**
 * Which mode switching the relay on should start on.
 *
 * Mirrors `RelayMode::suggested_for` in `kt-types`. Duplicated rather than sent
 * over the wire because it decides which radio button starts selected, which is
 * a question the window can answer before it has asked the daemon anything.
 *
 * Standard, at every visibility, and this used to suggest Strict for everything
 * but Public. Strict has no snapshot — no plaintext at the edge means there is
 * nothing to store — so a Strict app goes dark with the laptop. Suggesting it
 * for Invited pointed the *common* case, the rota shared with one person,
 * straight at the mode that turns off the thing the paid tier is bought for.
 *
 * Strict stays a deliberate choice for the app somebody means it for, rather
 * than the answer they accept because it was already selected. The argument is
 * about defaults, not about whether Strict is good; if you are restoring the
 * old shape, read this paragraph and `RelayMode::suggested_for` first.
 */
export function suggestedFor(_visibility: Visibility): Exclude<RelayMode, "off"> {
  return "standard";
}

/**
 * Whether the relay can do anything at all for an app at this level.
 *
 * Private is satisfiable only on loopback and Household means being on the home
 * network. A relayed request can satisfy neither, so a relay switch on either
 * could only ever produce refusals - and an inert switch that can only refuse
 * is the same class of lie as a blurb promising reach it does not have.
 *
 * Mirrors `Visibility::reachable_over_relay` in `kt-types`, which is where the
 * daemon decides the same thing when it works out whether an app has a public
 * address at all.
 *
 * **This governs whether the relay is *offered*, never whether it is shown.**
 * Hiding the block outright was a trap: publish an app as Invited, set it to
 * Private, and the whole block disappeared with the relay still switched on
 * and no way left to switch it off. See `strandedRelay`.
 */
export function relayApplies(visibility: Visibility): boolean {
  return visibility === "invited" || visibility === "public";
}

/**
 * An app that is published at a visibility the relay can never serve.
 *
 * Reachable in one click - publish as Invited, then change your mind about who
 * may open it - and it leaves the app registered as published while every
 * relayed request is refused. Nothing leaks, because the gate holds. But the
 * owner is left believing one of two wrong things depending on which control
 * they look at, so it has to be said out loud and be fixable from here.
 *
 * Changing the visibility deliberately does *not* switch the relay off by
 * itself: publishing and deciding who may open something are separate
 * decisions, and a side effect in either direction is how an app ends up
 * somewhere its owner did not put it.
 */
export function strandedRelay(
  visibility: Visibility,
  mode: RelayMode,
): boolean {
  return mode !== "off" && !relayApplies(visibility);
}
