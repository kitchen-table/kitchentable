import type { App } from "./generated";
import { handover, opens } from "./relay";

function app(over: Partial<App> = {}): App {
  return {
    slug: "trip-planner",
    name: "Trip Planner",
    entry: "index.html",
    visibility: "invited",
    version: 1,
    paused: false,
    relay: "off",
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
    expect(opens(app(), shown.url)).toBe("http://trip-planner.local");
  });

  it("goes to the public address once an app is published and the tunnel is up", () => {
    const published = app({
      relay: "standard",
      public_url: "https://chester-adarsh.kitchentable.cloud",
    });
    const shown = handover(published, { state: "connected" });

    expect(shown.url).toBe("https://chester-adarsh.kitchentable.cloud");
    expect(opens(published, shown.url)).toBe("https://chester-adarsh.kitchentable.cloud");
  });

  it("keeps loopback for a Private app, which no other address can open", () => {
    // Not a preference. `.local` and the public name both resolve to this
    // machine's LAN address, which earns no owner exemption in the gate, so
    // either would answer 403 to the person who owns the app.
    const priv = app({ visibility: "private" });
    const shown = handover(priv, { state: "connected" });

    expect(shown.url).toBe("http://trip-planner.local");
    expect(opens(priv, shown.url)).toBe("http://localhost/trip-planner");
  });

  it("does not dodge the gate for an Invited app", () => {
    // Its address can refuse - the owner's own browser is let in only if it is
    // an approved device - but being asked to pair is the product working, and
    // substituting a different URL would put the button back to opening
    // something other than what is on screen.
    const shown = handover(app({ visibility: "invited" }), undefined);
    expect(opens(app({ visibility: "invited" }), shown.url)).toBe(
      "http://trip-planner.local",
    );
  });

  it("falls back to the local address when the tunnel is down", () => {
    // Handing over a name that 502s while a working one existed is worse than
    // not offering the public name at all.
    const published = app({
      relay: "standard",
      public_url: "https://chester-adarsh.kitchentable.cloud",
    });
    const shown = handover(published, { state: "needs_attention", message: "no route" });

    expect(shown.url).toBe("http://trip-planner.local");
    expect(shown.away).toBe(false);
  });
});
