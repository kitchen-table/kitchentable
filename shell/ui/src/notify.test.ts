import { describe, expect, it } from "vitest";
import type { AccessEvent } from "./activity";
import type { Device } from "./devices";
import type { Event, NotifyPrefs } from "./generated";
import { APPROVE, DENY, openSpec, pairingSpec, requestedApp, wanted } from "./notify";

function device(over: Partial<Device> = {}): Device {
  return {
    id: "dev-1",
    name: "Priya's iPad",
    status: "pending",
    named_by: "guess",
    user_agent: "Mozilla/5.0 (iPad; CPU OS 19_0) Safari/605.1",
    fingerprint: "e2:77:41",
    first_seen: 1_760_000_000,
    last_seen: 1_760_000_000,
    ...over,
  };
}

function prefs(over: Partial<NotifyPrefs> = {}): NotifyPrefs {
  return { requests: true, opens: false, deploys: true, offline: true, ...over };
}

describe("the access request banner", () => {
  it("names the device and the app, and offers the decision", () => {
    const spec = pairingSpec(device(), "Trip Planner");

    expect(spec.title).toBe("Priya's iPad wants to open Trip Planner");
    expect(spec.actions?.map((action) => action.id)).toEqual([APPROVE, DENY]);
    // Approve first, because approving is the common answer and the first
    // button is the one macOS puts under the cursor.
    expect(spec.actions?.[0]?.label).toBe("Approve");
  });

  it("carries the fingerprint, which is what you check the real device against", () => {
    const spec = pairingSpec(device(), "Trip Planner");

    expect(spec.body).toContain("e2:77:41");
    expect(spec.body).toContain("Safari");
  });

  it("says only what it knows when the app is not established yet", () => {
    // The app comes from the access log, and the request can land first. A
    // banner that guessed an app name would be pointing at the wrong door.
    const spec = pairingSpec(device());

    expect(spec.title).toBe("Priya's iPad wants access");
    expect(spec.title).not.toContain("undefined");
  });

  it("sends the device id back as the key, so a pressed button can act", () => {
    expect(pairingSpec(device({ id: "abc" })).key).toBe("abc");
  });
});

describe("what earns a banner", () => {
  it("posts an access request, because somebody is waiting on the answer", () => {
    const event: Event = { event: "pairing_request", device_id: "dev-1" };
    expect(wanted(event, prefs())).toBe(true);
  });

  it("respects the switch being off", () => {
    const event: Event = { event: "pairing_request", device_id: "dev-1" };
    expect(wanted(event, prefs({ requests: false }))).toBe(false);
  });

  it("stays quiet about opens unless asked", () => {
    const event: Event = { event: "access", app_slug: "trip-planner", action: "opened" };

    expect(wanted(event, prefs())).toBe(false);
    expect(wanted(event, prefs({ opens: true }))).toBe(true);
  });

  it("never interrupts for a refusal or a revocation", () => {
    // These are log entries, not decisions waiting on anybody. The
    // Notifications surface lists them; the desktop does not shout them.
    const refused: Event = { event: "access", app_slug: "trip-planner", action: "refused" };
    expect(wanted(refused, prefs({ opens: true }))).toBe(false);
  });

  it("has nothing to say about events the daemon only reports to the window", () => {
    const presence: Event = { event: "presence", app_slug: "trip-planner", viewers: [] };
    expect(wanted(presence, prefs())).toBe(false);
  });
});

describe("finding which app a device asked for", () => {
  const log: AccessEvent[] = [
    { at: 30, device_id: "dev-1", action: "requested", app_slug: "chores-rota", actor: "viewer" },
    { at: 20, device_id: "dev-2", action: "requested", app_slug: "sprint-board", actor: "viewer" },
    { at: 10, device_id: "dev-1", action: "opened", app_slug: "trip-planner", actor: "viewer" },
  ];

  it("takes the newest request from that device", () => {
    expect(requestedApp(log, "dev-1")).toBe("chores-rota");
  });

  it("returns nothing rather than another device's app", () => {
    expect(requestedApp(log, "dev-9")).toBeUndefined();
  });
});

describe("the open banner", () => {
  it("reads as a sentence about what happened", () => {
    expect(openSpec("Priya's iPhone", "Trip Planner").title).toBe(
      "Priya's iPhone opened Trip Planner",
    );
  });

  it("carries no buttons, because there is nothing to decide", () => {
    expect(openSpec("Priya's iPhone", "Trip Planner").actions).toBeUndefined();
  });
});
