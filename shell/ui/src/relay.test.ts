import type { App } from "./generated";
import { handover } from "./relay";

function app(over: Partial<App> = {}): App {
  return {
    slug: "trip-planner",
    name: "Trip Planner",
    entry: "index.html",
    visibility: "invited",
    version: 1,
    paused: false,
    relay: "off",
    storage: "synced",
    storage_backup: true,
    public_label: "chester",
    url: "http://trip-planner.local",
    hostname: "trip-planner.local",
    fallback_url: "http://192.168.0.5/trip-planner",
    loopback_url: "http://localhost/trip-planner",
    path: "/ws/Trip Planner",
    size_bytes: 1000,
    deployed_at: 1_760_000_000,
    entry_exists: true,
    ...over,
  };
}

describe("where Open goes", () => {
  it("follows the address the window is showing", () => {
    // A button that went somewhere other than the address printed two lines
    // above it is the thing this fixes.
    const shown = handover(app(), { state: "connected" });
    expect(shown.url).toBe("http://trip-planner.local");
  });

  it("goes to the public address once an app is published and the tunnel is up", () => {
    const published = app({
      relay: "standard",
      storage: "synced",
      storage_backup: true,
      public_url: "https://chester-adarsh.kitchentable.cloud",
    });
    const shown = handover(published, { state: "connected" });

    expect(shown.url).toBe("https://chester-adarsh.kitchentable.cloud");
  });

  it("shows the same address on every visibility level", () => {
    // No substitution anywhere. A Private app is satisfiable only on loopback
    // and so answers 403 at this address - that is the gate's answer to show,
    // not one to route around by opening a different URL than the one on
    // screen. Same for Invited, which may ask the owner to pair.
    for (const visibility of ["private", "network", "invited", "public"] as const) {
      const shown = handover(app({ visibility }), { state: "connected" });
      expect(shown.url, visibility).toBe("http://trip-planner.local");
    }
  });

  it("falls back to the local address when the tunnel is down", () => {
    // Handing over a name that 502s while a working one existed is worse than
    // not offering the public name at all.
    const published = app({
      relay: "standard",
      storage: "synced",
      storage_backup: true,
      public_url: "https://chester-adarsh.kitchentable.cloud",
    });
    const shown = handover(published, { state: "needs_attention", message: "no route" });

    expect(shown.url).toBe("http://trip-planner.local");
    expect(shown.away).toBe(false);
  });
});
