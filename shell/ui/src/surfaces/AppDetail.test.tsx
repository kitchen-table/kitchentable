import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { vi } from "vitest";
import type { App } from "../generated";
import type { AppTab } from "../navigation";
import { AppDetail } from "./AppDetail";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const call = vi.mocked(invoke);

function app(over: Partial<App> = {}): App {
  return {
    slug: "trip-planner",
    name: "Trip Planner",
    entry: "index.html",
    visibility: "invited",
    version: 3,
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
    deployed_at: Math.floor(Date.now() / 1000) - 7200,
    entry_exists: true,
    ...over,
  };
}

function show(over: Partial<App> = {}, tab: AppTab = "overview") {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const onTab = vi.fn();
  const onNavigate = vi.fn();
  const view = render(
    <QueryClientProvider client={client}>
      <AppDetail app={app(over)} tab={tab} onTab={onTab} onNavigate={onNavigate} />
    </QueryClientProvider>,
  );
  return { ...view, onTab, onNavigate };
}

/**
 * Answer each socket method separately.
 *
 * The Overview asks three questions at once - the log, the devices to name it
 * with, and the invites - so a single blanket answer cannot say anything
 * useful about any of them.
 */
function answers(byMethod: Record<string, unknown>) {
  call.mockImplementation((_command, args) => {
    const method = (args as { method: string }).method;
    return Promise.resolve(byMethod[method] ?? []);
  });
}

describe("AppDetail", () => {
  beforeEach(() => {
    call.mockReset();
    call.mockResolvedValue([]);
  });

  it("keeps the app's name, level and URL visible on every tab", () => {
    show({}, "devices");

    expect(screen.getByRole("heading", { name: "Trip Planner" })).toBeDefined();
    expect(screen.getByText("Invited")).toBeDefined();
    expect(screen.getByText(/trip-planner\.local · index\.html/)).toBeDefined();
  });

  it("offers all five built tabs and marks the current one", () => {
    show({}, "sharing");

    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual([
      "Overview",
      "Sharing",
      "Devices",
      "Activity",
      "Storage",
    ]);
    expect(screen.getByRole("tab", { name: "Sharing" }).getAttribute("aria-selected")).toBe("true");
  });

  it("switches tabs", () => {
    const { onTab } = show();
    fireEvent.click(screen.getByRole("tab", { name: "Devices" }));
    expect(onTab).toHaveBeenCalledWith("devices");
  });

  it("goes back to the library", () => {
    const { onNavigate } = show();
    fireEvent.click(screen.getByRole("button", { name: "← Library" }));
    expect(onNavigate).toHaveBeenCalledWith({ kind: "library", filter: null });
  });

  it("counts opens from the access log", async () => {
    call.mockResolvedValue([
      { at: 1, actor: "viewer", action: "opened" },
      { at: 2, actor: "viewer", action: "opened" },
      { at: 3, actor: "owner", action: "paired" },
    ]);

    show();
    await waitFor(() => expect(screen.getByText("2")).toBeDefined());
    expect(screen.getByText("total opens")).toBeDefined();
  });

  it("shows a published app's public address under its name, on every tab", async () => {
    // The most prominent URL in the window, and the first place somebody looks
    // to check a rename took. It showed the `.local` name whatever the app's
    // public address was.
    answers({ "sys.status": { relay: { state: "connected" } } });
    show({ public_url: "https://chester-adarsh.kitchentable.cloud" }, "devices");

    await waitFor(() =>
      expect(screen.getByText(/chester-adarsh\.kitchentable\.cloud · index\.html/)).toBeDefined(),
    );
  });

  it("opens the address on screen, on every visibility level", async () => {
    // This has been both ways round, so the history is worth keeping.
    //
    // It asserted `.local`, then loopback after an owner clicked Open on a
    // Private app and was refused on their own machine - `.local` resolves to
    // this machine's LAN address, so it is not loopback and earns no exemption
    // in the gate: 403 for Private, a pairing prompt for Invited.
    //
    // It is back to the shown address by an explicit decision of the owner's,
    // who found a button that went somewhere other than the address printed
    // two lines above it the worse of the two faults. The 403 is real and is
    // the gate's answer to show. **Do not quietly restore the substitution** -
    // if this needs revisiting, the fix is to make Private openable for the
    // owner from a non-loopback address, which is what the dead `is_owner`
    // branch in the gate is waiting for.
    answers({ "sys.status": { relay: { state: "connected" } } });
    show({
      visibility: "private",
      public_url: "https://chester-adarsh.kitchentable.cloud",
    });

    await waitFor(() => screen.getByRole("link", { name: /Open/ }));
    expect(screen.getByRole("link", { name: /Open/ }).getAttribute("href")).toBe(
      "http://trip-planner.local",
    );
  });

  it("hands a phone the address that still works after they leave", async () => {
    // The QR is the take-it-with-you affordance, and it was encoding the
    // `.local` name even for a published app - so it worked at the kitchen
    // table and nowhere else.
    answers({
      "sys.status": { relay: { state: "connected" } },
    });
    show({ public_url: "https://trip-adarsh.kitchentable.cloud" });

    await waitFor(() =>
      expect(screen.getByText("trip-adarsh.kitchentable.cloud")).toBeDefined(),
    );
    expect(screen.getByText("Works away from your network")).toBeDefined();
    // The local name is still offered, for a phone that is in the house.
    expect(screen.getByText(/or trip-planner\.local/)).toBeDefined();
  });

  it("keeps the local address when the tunnel is not up", async () => {
    // Handing somebody a code that 502s, while a working one was available,
    // would be worse than not offering the public name at all.
    answers({ "sys.status": { relay: { state: "connecting" } } });
    show({ public_url: "https://trip-adarsh.kitchentable.cloud" });

    await waitFor(() => expect(screen.getByText("trip-planner.local")).toBeDefined());
    expect(screen.queryByText("trip-adarsh.kitchentable.cloud")).toBeNull();
  });

  it("reports bundle size in human units", () => {
    show({ size_bytes: 42_000 });
    expect(screen.getByText("41 KB")).toBeDefined();
  });

  it("names who has the app open, and the page they are on", async () => {
    answers({
      "presence.list": [
        { device_id: "d1", path: "/budget", seconds_ago: 2 },
        { device_id: "owner", path: "/", seconds_ago: 1 },
      ],
      "device.list": [{ id: "d1", name: "Kitchen iPad", status: "approved" }],
    });

    show();
    await waitFor(() => expect(screen.getByText("Kitchen iPad")).toBeDefined());
    expect(screen.getByText(/viewing budget/)).toBeDefined();
    // The owner's own browser has no device record, so it is named for what
    // it is rather than left as an unrecognised id.
    expect(screen.getByText("This machine")).toBeDefined();
    expect(screen.getByText(/viewing the app/)).toBeDefined();
  });

  it("tells two unpaired readers apart by where they are", async () => {
    // Household apps pair nobody, so their readers have no device to name.
    // One row saying "Someone" for a houseful of phones would be worse than
    // saying where each of them is.
    answers({
      "presence.list": [
        { device_id: "at:192.168.0.51", path: "/", seconds_ago: 1 },
        { device_id: "at:192.168.0.77", path: "/", seconds_ago: 2 },
      ],
    });

    show();
    await waitFor(() => expect(screen.getByText("Someone at 192.168.0.51")).toBeDefined());
    expect(screen.getByText("Someone at 192.168.0.77")).toBeDefined();
  });

  it("counts the people reading it, once it has asked", async () => {
    answers({ "presence.list": [{ device_id: "d1", path: "/", seconds_ago: 1 }] });

    show();
    await waitFor(() => expect(screen.getByText("online now")).toBeDefined());
    await waitFor(() => expect(screen.getByText("1")).toBeDefined());
  });

  it("says nobody rather than pretending, when nobody has it open", async () => {
    answers({ "presence.list": [] });

    show();
    await waitFor(() =>
      expect(screen.getByText(/Nobody has this open right now/)).toBeDefined(),
    );
  });

  it("puts Live now above the history, as the mockup does", () => {
    // Who is reading it now is the question an owner asks first.
    const { container } = show();
    const text = container.textContent ?? "";

    expect(text.indexOf("Live now")).toBeGreaterThan(-1);
    expect(text.indexOf("Live now")).toBeLessThan(text.indexOf("Recent activity"));
  });

  it("says when the folder last changed, and does not call it a deploy", () => {
    // The mockup reads "v7 / deployed 12m ago" and neither half was true here:
    // `version` is stamped 1 by the registry and never moves, and `deployed_at`
    // is the folder's mtime. Nothing deploys - the workspace is a watched
    // folder - so the tile reports the change and calls it one.
    const { container } = show({
      version: 7,
      deployed_at: Math.floor(Date.now() / 1000) - 720,
    });

    expect(screen.getByText("12m ago")).toBeDefined();
    expect(screen.getByText("last changed")).toBeDefined();
    expect(container.textContent).not.toMatch(/deployed/i);
    expect(container.textContent).not.toMatch(/\bv7\b/);
  });

  it("shows a dash rather than a guess when the filesystem will not say", () => {
    // Some network mounts, and Linux before 4.11. "just now" would be a
    // fabrication about somebody's folder.
    show({ deployed_at: undefined });

    // Scoped to the tile: a dash also stands for "presence has not answered
    // yet" two tiles along, and asserting on the character alone would pass
    // whichever of them rendered.
    const tile = screen.getByText("last changed").parentElement;
    expect(tile?.textContent).toBe("—last changed");
  });

  it("has no Versions tab, because nothing creates a version", () => {
    // The mockup draws six. The sixth could only ever be empty: there is no
    // deploy, no app.write_file and no kt deploy, so the tab is absent rather
    // than present-and-lying. Delete this test when something writes a version.
    show();

    expect(screen.queryByRole("tab", { name: "Versions" })).toBeNull();
  });

  it("says who did each thing in the activity list, not just what happened", () => {
    // The mockup writes whole sentences - "Priya's iPhone opened the app" -
    // because a column of the bare word "Opened" answers the least
    // interesting half of the question.
    answers({
      "log.query": [{ at: 1, actor: "viewer", action: "opened", device_id: "d1" }],
      "device.list": [{ id: "d1", name: "Kitchen iPad", status: "approved" }],
    });

    show();
    return waitFor(() =>
      expect(screen.getByText(/Kitchen iPad opened the app/)).toBeDefined(),
    );
  });

  it("keeps the recent-activity panel to three rows, as the mockup does", async () => {
    // The panel is a glance with "View all" under it, not a short copy of the
    // Activity tab, and the mockup draws three.
    answers({
      "log.query": Array.from({ length: 6 }, (_, index) => ({
        at: 1000 - index,
        actor: "viewer",
        action: "opened",
      })),
    });

    show();
    await waitFor(() =>
      expect(screen.getAllByText(/opened the app/).length).toBeGreaterThan(0),
    );
    expect(screen.getAllByText(/opened the app/)).toHaveLength(3);
  });

  it("falls back to the actor when the device is not one we know", () => {
    answers({ "log.query": [{ at: 1, actor: "owner", action: "opened" }] });

    show();
    return waitFor(() => expect(screen.getByText(/You opened the app/)).toBeDefined());
  });

  it("has words for a request the gate turned away", async () => {
    // `refused` is what kt-server writes on every blocked open - the most
    // common negative event - and it had no entry at all, so it rendered as
    // the raw lowercase word beside a meaningless dot.
    answers({ "log.query": [{ at: 1, actor: "viewer", action: "refused" }] });

    show();
    await waitFor(() =>
      expect(screen.getByText(/Someone was turned away/)).toBeDefined(),
    );
    expect(screen.queryByText(/^refused$/)).toBeNull();
  });

  it("offers the live invite link, which is the thing you hand over", async () => {
    answers({
      "share.list_invites": [
        { token: "9fK2", url: "http://trip-planner.local/i/9fK2-mZq8vL", active: true },
        { token: "old", url: "http://trip-planner.local/i/old", active: false },
      ],
    });

    show();
    await waitFor(() => expect(screen.getByText("Invite link · active")).toBeDefined());
    expect(screen.getByText("trip-planner.local/i/9fK2-mZq8vL")).toBeDefined();
    expect(screen.queryByText(/\/i\/old/)).toBeNull();
  });

  it("explains who can open the app when there is no link to show", async () => {
    answers({ "share.list_invites": [] });

    show();
    await waitFor(() => expect(screen.getByText(/Invited ·/)).toBeDefined());
    expect(screen.queryByText("Invite link · active")).toBeNull();
  });

  it("shows a dash for online now rather than a zero it cannot know", () => {
    // Until event.* subscriptions land there is no live view of who is
    // connected. Printing 0 would be a claim; a dash is an admission.
    show();
    expect(screen.getByText("—")).toBeDefined();
    expect(screen.getByText("online now")).toBeDefined();
  });

  it("renders a QR big enough to scan off the screen", () => {
    show();
    expect(screen.getAllByRole("img", { name: "Scan to open Trip Planner" }).length).toBe(1);
  });

  it("offers the IP fallback only when there is no .local name", () => {
    const { unmount } = show({ hostname: undefined });
    expect(screen.getByText(/^or 192\.168\.0\.5/)).toBeDefined();
    unmount();

    show({ hostname: "trip-planner.local" });
    expect(screen.queryByText(/^or 192\.168\.0\.5/)).toBeNull();
  });

  it("explains a missing entry file, on every tab", () => {
    // The only failure with no visible symptom until someone taps the link.
    const { unmount } = show({ entry: "index.html", entry_exists: false }, "devices");

    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("no page to open");
    expect(alert.textContent).toContain("index.html");
    expect(alert.textContent).toContain("/ws/Trip Planner");
    unmount();

    show({ entry_exists: true });
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("says so when nothing has happened yet", async () => {
    show();
    await waitFor(() => expect(screen.getByText(/Nothing yet/)).toBeDefined());
  });
});
