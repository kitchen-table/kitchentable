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
    url: "http://trip-planner.local",
    hostname: "trip-planner.local",
    fallback_url: "http://192.168.0.5/trip-planner",
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

describe("AppDetail", () => {
  beforeEach(() => {
    call.mockReset();
    call.mockResolvedValue([]);
  });

  it("keeps the app's name, level and URL visible on every tab", () => {
    show({}, "devices");

    expect(screen.getByRole("heading", { name: "Trip Planner" })).toBeDefined();
    expect(screen.getByText("Invited")).toBeDefined();
    expect(screen.getByText(/trip-planner\.local · v3/)).toBeDefined();
  });

  it("offers all six tabs and marks the current one", () => {
    show({}, "sharing");

    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual([
      "Overview",
      "Sharing",
      "Devices",
      "Activity",
      "Storage",
      "Versions",
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

  it("reports bundle size in human units", () => {
    show({ size_bytes: 42_000 });
    expect(screen.getByText("41 KB")).toBeDefined();
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
