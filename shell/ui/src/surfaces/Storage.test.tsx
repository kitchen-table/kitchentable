import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { vi } from "vitest";
import type { App } from "../generated";
import { Storage } from "./Storage";

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
    public_label: "trip-planner",
    url: "http://trip-planner.local",
    hostname: "trip-planner.local",
    fallback_url: "http://192.168.0.5/trip-planner",
    loopback_url: "http://localhost/trip-planner",
    path: "/ws/Trip Planner",
    size_bytes: 42_000,
    deployed_at: 0,
    entry_exists: true,
    ...over,
  };
}

function show(over: Partial<App> = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Storage app={app(over)} />
    </QueryClientProvider>,
  );
}

function paramsOf(method: string) {
  const match = call.mock.calls.find(
    ([cmd, args]) => cmd === "kt_call" && (args as { method?: string })?.method === method,
  );
  return match?.[1] as { method: string; params?: Record<string, unknown> } | undefined;
}

beforeEach(() => {
  call.mockReset();
  call.mockResolvedValue({});
});

it("offers both ways an app can remember things, and marks the current one", () => {
  show({ storage: "synced" });

  const chosen = screen.getByRole("radio", { name: /Keep in sync everywhere/ });
  const other = screen.getByRole("radio", { name: /Separate on each device/ });
  expect(chosen.getAttribute("aria-checked")).toBe("true");
  expect(other.getAttribute("aria-checked")).toBe("false");
});

it("changes the mode without touching the backup setting", async () => {
  // The window sends the mode on its own; the daemon keeps whatever backup
  // was already set. Sending `backup` here would let a click on a card turn
  // backups off as a side effect.
  show({ storage: "synced" });

  fireEvent.click(screen.getByRole("radio", { name: /Separate on each device/ }));

  await waitFor(() => expect(paramsOf("storage.set_mode")).toBeDefined());
  expect(paramsOf("storage.set_mode")?.params).toEqual({
    slug: "trip-planner",
    mode: "per_device",
  });
});

it("says the copy is made immediately, not on a schedule", () => {
  // No clock to keep honest, and no question about what happens while the Mac
  // is asleep.
  show({ storage: "per_device", storage_backup: true });

  expect(screen.getByText(/saved here immediately/)).toBeDefined();
  expect(screen.queryByText(/nightly/i)).toBeNull();
});

it("warns plainly when the copy is off", () => {
  show({ storage: "per_device", storage_backup: false });

  expect(screen.getByText(/data stays only on each device/i)).toBeDefined();
  expect(screen.getByRole("switch").getAttribute("aria-checked")).toBe("false");
});

it("turns the copy off without changing where the data lives", async () => {
  show({ storage: "per_device", storage_backup: true });

  fireEvent.click(screen.getByRole("switch"));

  await waitFor(() => expect(paramsOf("storage.set_mode")).toBeDefined());
  expect(paramsOf("storage.set_mode")?.params).toEqual({
    slug: "trip-planner",
    mode: "per_device",
    backup: false,
  });
});

it("will not offer a backup of data that is already here", () => {
  // Under Synced there is no device copy to make, so the switch would be a
  // control that silently does nothing. Drawn with the reason instead.
  show({ storage: "synced" });

  const toggle = screen.getByRole("switch");
  expect(toggle.hasAttribute("disabled")).toBe(true);
  expect(screen.getByText(/already here/)).toBeDefined();
});

describe("the stored data", () => {
  it("does not claim the store is empty while it is still reading", () => {
    // "Nothing stored yet" and "we have not asked" are different claims, and
    // this is the panel an owner checks to see what an app is holding.
    call.mockImplementation(() => new Promise(() => {}));
    show();

    expect(screen.getByText(/Reading the store/)).toBeDefined();
    expect(screen.queryByText(/Nothing stored yet/)).toBeNull();
  });

  it("shows the shared store's keys, values and types", async () => {
    call.mockResolvedValue({
      bytes: 42,
      entries: [
        { key: "items", value: '["passports"]', kind: "json", updated_at: 0 },
      ],
    });
    show();

    await waitFor(() => expect(screen.getByText("items")).toBeDefined());
    expect(screen.getByText('["passports"]')).toBeDefined();
    expect(screen.getByText("json")).toBeDefined();
    expect(screen.getByText(/SQLite · isolated · 42 B/)).toBeDefined();
  });

  it("counts what each device holds without showing what is in it", async () => {
    // The owner's window is not a reason to build a way to read somebody's
    // private notes, and the daemon does not send them.
    call.mockResolvedValue({
      bytes: 10,
      devices: [{ device: "d1", name: "Priya's iPhone", keys: 2 }],
    });
    show({ storage: "per_device" });

    fireEvent.click(screen.getByRole("tab", { name: "Per-viewer" }));

    await waitFor(() => expect(screen.getByText("Priya's iPhone")).toBeDefined());
    expect(screen.getByText("2 keys")).toBeDefined();
    expect(screen.getByText("VIEWER")).toBeDefined();
  });

  it("says the daemon did not answer rather than showing an empty store", async () => {
    call.mockRejectedValue(new Error("gone"));
    show();

    await waitFor(() => expect(screen.getByText(/did not answer/)).toBeDefined());
  });
});
