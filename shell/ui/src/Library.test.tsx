import { fireEvent, render, screen } from "@testing-library/react";
import { Library } from "./Library";
import type { App } from "./generated";

function app(over: Partial<App> = {}): App {
  return {
    slug: "trip-planner",
    name: "Trip Planner",
    entry: "index.html",
    visibility: "private",
    version: 1,
    paused: false,
    relay: "off",
    storage: "synced",
    storage_backup: true,
    public_label: "trip",
    url: "http://trip-planner.local",
    hostname: "trip-planner.local",
    fallback_url: "http://192.168.0.5/trip-planner",
    loopback_url: "http://localhost/trip-planner",
    path: "/ws/Trip Planner",
    size_bytes: 42_000,
    deployed_at: 1_760_000_000,
    entry_exists: true,
    ...over,
  };
}

/** The props the shell always supplies, so each test states only what it varies. */
function show(over: Partial<Parameters<typeof Library>[0]> = {}) {
  return render(
    <Library
      apps={[app()]}
      filter={null}
      query=""
      workspace="/ws"
      onOpen={() => undefined}
      {...over}
    />,
  );
}

describe("Library", () => {
  it("lists the apps it is given", () => {
    show({ apps: [app(), app({ slug: "chores", name: "Chores Rota" })] });

    expect(screen.getByRole("heading", { name: "Trip Planner" })).toBeDefined();
    expect(screen.getByRole("heading", { name: "Chores Rota" })).toBeDefined();
  });

  it("labels the network level Household, not Network", () => {
    // The wire value stays `network`; only the label changed. Getting this
    // wrong would put an internal name in front of a person.
    show({ apps: [app({ visibility: "network" })] });

    expect(screen.getAllByText("Household").length).toBeGreaterThan(0);
    expect(screen.queryByText("Network")).toBeNull();
  });

  it("shows only the level the shell filtered to", () => {
    show({
      apps: [
        app({ slug: "a", name: "Alpha", visibility: "private" }),
        app({ slug: "b", name: "Beta", visibility: "invited" }),
      ],
      filter: "invited",
    });

    expect(screen.queryByRole("heading", { name: "Alpha" })).toBeNull();
    expect(screen.getByRole("heading", { name: "Beta" })).toBeDefined();
  });

  it("searches name, slug and URL", () => {
    const apps = [
      app({ slug: "alpha", name: "Alpha", url: "http://alpha.local" }),
      app({ slug: "beta", name: "Beta", url: "http://beta.local" }),
    ];

    show({ apps, query: "bet" });
    expect(screen.queryByRole("heading", { name: "Alpha" })).toBeNull();
    expect(screen.getByRole("heading", { name: "Beta" })).toBeDefined();
  });

  it("says so when a search matches nothing", () => {
    show({ query: "nothing-like-this" });
    expect(screen.getByText(/Nothing matches/)).toBeDefined();
  });

  it("opens an app when its card is clicked", () => {
    let opened: string | undefined;
    show({ onOpen: (chosen) => (opened = chosen.slug) });

    fireEvent.click(screen.getByRole("button", { name: "Trip Planner" }));
    expect(opened).toBe("trip-planner");
  });

  it("tells someone what to do when the workspace is empty", () => {
    show({ apps: [] });
    expect(screen.getByText("Set the table")).toBeDefined();
    expect(screen.getByRole("button", { name: "Add your first app" })).toBeDefined();
  });

  it("flags an app the network made us rename", () => {
    // Otherwise the URL on the card looks like a bug.
    show({
      apps: [app({ slug: "notes", hostname: "notes-2.local", url: "http://notes-2.local" })],
    });
    expect(screen.getByText("name taken")).toBeDefined();
  });

  it("does not flag an app that got the name it asked for", () => {
    show({
      apps: [app({ slug: "notes", hostname: "notes.local", url: "http://notes.local" })],
    });
    expect(screen.queryByText("name taken")).toBeNull();
  });

  it("flags an app with no .local name at all", () => {
    // mDNS is down or was denied; the IP fallback is the only thing that works.
    show({ apps: [app({ hostname: undefined })] });
    expect(screen.getByText("IP fallback")).toBeDefined();
  });

  it("flags plain HTTP as not encrypted", () => {
    show({ apps: [app({ url: "http://trip-planner.local" })] });
    expect(screen.getByText("Not encrypted")).toBeDefined();
  });

  it("does not flag HTTPS as not encrypted", () => {
    show({ apps: [app({ url: "https://trip-planner.local" })] });
    expect(screen.queryByText("Not encrypted")).toBeNull();
  });

  it("flags an app whose entry file is missing", () => {
    // The app is registered, announced and gated exactly like a working one.
    // Nothing else on the card would reveal that its URL is a 404.
    show({ apps: [app({ entry_exists: false })] });
    expect(screen.getByText(/won.t open/)).toBeDefined();
  });

  it("does not flag an app whose entry file is there", () => {
    show({ apps: [app({ entry_exists: true })] });
    expect(screen.queryByText(/won.t open/)).toBeNull();
  });

  it("renders a scannable QR code per app", () => {
    // The aha moment is a phone camera, so a card without one is a card that
    // cannot do its job.
    show();
    expect(
      screen.getByRole("img", { name: "Scan to open Trip Planner" }),
    ).toBeDefined();
  });
});
