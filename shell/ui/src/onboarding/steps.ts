/**
 * The onboarding flow.
 *
 * Nine steps, seven required and two optional, matching the macOS mockup. The
 * order is the point: the aha moment is a QR code on step 4 and a phone opening
 * the app on step 6, so everything before them is kept to what is genuinely
 * needed first.
 *
 * Notifications sits between them deliberately. It is the permission that makes
 * the *next* step work without the window open - the banner is how an access
 * request is answered - so it is asked for immediately before the step that
 * produces one, and after the app it will be about exists.
 */
export const STEPS = [
  "welcome",
  "workspace",
  "network",
  "first-app",
  "notifications",
  "pair",
  "ready",
  "agent",
  "relay",
] as const;

export type Step = (typeof STEPS)[number];

/** The rail labels, split the way the mockup splits them. */
export const REQUIRED: Step[] = [
  "welcome",
  "workspace",
  "network",
  "first-app",
  "notifications",
  "pair",
  "ready",
];
export const OPTIONAL: Step[] = ["agent", "relay"];

export const LABELS: Record<Step, string> = {
  welcome: "Welcome",
  workspace: "Workspace",
  network: "Network",
  "first-app": "Your first app",
  notifications: "Notifications",
  pair: "Pair your phone",
  ready: "You're all set",
  agent: "Connect your agent",
  relay: "Away from home",
};

export function stepIndex(step: Step): number {
  return STEPS.indexOf(step);
}

export function next(step: Step): Step {
  const i = stepIndex(step);
  return STEPS[Math.min(i + 1, STEPS.length - 1)] ?? step;
}

export function previous(step: Step): Step {
  const i = stepIndex(step);
  return STEPS[Math.max(i - 1, 0)] ?? step;
}

/** Optional steps can be skipped; required ones cannot. */
export function isSkippable(step: Step): boolean {
  return OPTIONAL.includes(step);
}

/**
 * Whether onboarding has been completed on this machine.
 *
 * Stored locally rather than in the daemon: it is a fact about this person
 * having seen these screens, not about the workspace, and a fresh install
 * pointed at an existing workspace should still get the tour.
 */
const DONE_KEY = "kt.onboarding.done";

export function isComplete(): boolean {
  try {
    return localStorage.getItem(DONE_KEY) === "1";
  } catch {
    // Private browsing, or storage disabled. Showing onboarding again is a
    // better failure than crashing the window.
    return false;
  }
}

export function markComplete(): void {
  try {
    localStorage.setItem(DONE_KEY, "1");
  } catch {
    // Nothing to do; they will see the tour again next launch.
  }
}

export function reset(): void {
  try {
    localStorage.removeItem(DONE_KEY);
  } catch {
    // Same.
  }
}
