import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { vi } from "vitest";
import type { App } from "../generated";
import type { Device } from "../devices";
import { Shell } from "./Shell";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const call = vi.mocked(invoke);

function device(over: Partial<Device> = {}): Device {
  return {
    id: "dev-1",
    name: "iPhone",
    status: "pending",
    named_by: "guess",
    user_agent: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0) Safari/605.1",
    fingerprint: "f3:9a:21",
    first_seen: 1_760_000_000,
    last_seen: 1_760_000_000,
    ...over,
  };
}

/** Answers `device.list`; every other socket call resolves empty. */
function withDevices(devices: Device[]) {
  call.mockImplementation(((cmd: string, args?: unknown) => {
    const method = (args as { method?: string } | undefined)?.method;
    if (cmd === "kt_call" && method === "device.list") {
      return Promise.resolve(devices);
    }
    return Promise.resolve([]);
  }) as typeof invoke);
}

function app(over: Partial<App> = {}): App {
  return {
    slug: "trip-planner",
    name: "Trip Planner",
    entry: "index.html",
    visibility: "private",
    version: 1,
    paused: false,
    relay: "off",
    public_label: "trip",
    url: "http://trip-planner.local",
    hostname: "trip-planner.local",
    fallback_url: "http://192.168.0.5/trip-planner",
    path: "/ws/Trip Planner",
    size_bytes: 42_000,
    deployed_at: 1_760_000_000,
    entry_exists: true,
    ...over,
  };
}

function show(apps: App[] = [app()]) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Shell apps={apps} />
    </QueryClientProvider>,
  );
}

describe("Shell", () => {
  beforeEach(() => {
    call.mockReset();
    withDevices([]);
  });

  afterEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute("data-theme");
  });

  it("opens on the library", () => {
    show();
    expect(screen.getByRole("heading", { name: "Library" })).toBeDefined();
  });

  it("counts each visibility level in the rail", () => {
    show([
      app({ slug: "a", visibility: "invited" }),
      app({ slug: "b", visibility: "invited" }),
      app({ slug: "c", visibility: "private" }),
    ]);

    const invited = screen.getByRole("button", { name: /Invited/ });
    expect(within(invited).getByText("2")).toBeDefined();
  });

  it("filters the library from the rail, and unfilters on a second click", () => {
    show([
      app({ slug: "a", name: "Alpha", visibility: "private" }),
      app({ slug: "b", name: "Beta", visibility: "invited" }),
    ]);

    const invited = screen.getByRole("button", { name: /Invited/ });
    fireEvent.click(invited);
    expect(screen.queryByRole("heading", { name: "Alpha" })).toBeNull();

    fireEvent.click(invited);
    expect(screen.getByRole("heading", { name: "Alpha" })).toBeDefined();
  });

  it("searches from the title bar", () => {
    show([app({ slug: "a", name: "Alpha" }), app({ slug: "b", name: "Beta" })]);

    fireEvent.change(screen.getByRole("textbox", { name: /Search apps/ }), {
      target: { value: "alph" },
    });

    expect(screen.getByRole("heading", { name: "Alpha" })).toBeDefined();
    expect(screen.queryByRole("heading", { name: "Beta" })).toBeNull();
  });

  it("badges waiting devices, and shows nothing when none are waiting", async () => {
    withDevices([device({ id: "a" }), device({ id: "b" })]);
    const { unmount } = show();

    await waitFor(() => {
      const waiting = screen.getByRole("button", { name: "2 devices waiting for approval" });
      expect(within(waiting).getByText("2")).toBeDefined();
    });
    unmount();

    // A zero badge would be a permanent red dot on a product whose whole point
    // is that nothing is happening unless you said so.
    withDevices([]);
    show();
    const idle = screen.getByRole("button", { name: "0 devices waiting for approval" });
    expect(within(idle).queryByText("0")).toBeNull();
  });

  it("puts the pairing prompt in front of everything when a device asks", async () => {
    // A phone is sitting on the wait page; nothing else on screen matters more.
    withDevices([device()]);
    show();

    await waitFor(() =>
      expect(screen.getByRole("dialog", { name: "A device wants access" })).toBeDefined(),
    );
    expect(screen.getByText(/f3:9a:21 · Safari · iPhone/)).toBeDefined();
  });

  it("dismissing the prompt means decide later, not deny", async () => {
    // The badge keeps the count and the device stays pending.
    withDevices([device()]);
    show();

    await waitFor(() => expect(screen.getByRole("dialog")).toBeDefined());
    fireEvent.keyDown(document, { key: "Escape" });

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(
      screen.getByRole("button", { name: "1 device waiting for approval" }),
    ).toBeDefined();
    expect(call).not.toHaveBeenCalledWith(
      "kt_call",
      expect.objectContaining({ method: "device.deny" }),
    );
  });

  it("toggles appearance and remembers the choice", () => {
    show();
    fireEvent.click(screen.getByRole("button", { name: /Switch to (dark|light) appearance/ }));

    expect(document.documentElement.getAttribute("data-theme")).toMatch(/dark|light/);
    expect(localStorage.getItem("kt.appearance")).toMatch(/dark|light/);
  });

  it("opens the add-an-app dialog from the title bar", () => {
    show();
    fireEvent.click(screen.getByRole("button", { name: /New app/ }));
    expect(screen.getByRole("dialog", { name: "Add an app" })).toBeDefined();
  });

  it("closes the dialog on Escape", () => {
    show();
    fireEvent.click(screen.getByRole("button", { name: /New app/ }));
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("navigates to the other surfaces", () => {
    show();
    fireEvent.click(screen.getByRole("button", { name: /^Activity/ }));
    expect(screen.queryByRole("heading", { name: "Library" })).toBeNull();
  });
});
