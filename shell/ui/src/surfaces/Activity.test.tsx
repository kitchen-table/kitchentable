import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { vi } from "vitest";
import type { AccessEvent } from "../activity";
import type { Device } from "../devices";
import type { App } from "../generated";
import { AppActivity, WorkspaceActivity } from "./Activity";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const call = vi.mocked(invoke);

const NOW = Math.floor(Date.now() / 1000);

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
    deployed_at: NOW - 7200,
    entry_exists: true,
    ...over,
  };
}

function event(over: Partial<AccessEvent> = {}): AccessEvent {
  return { at: NOW - 120, actor: "viewer", action: "opened", ...over };
}

function device(over: Partial<Device> = {}): Device {
  return {
    id: "dev-1",
    name: "Kitchen iPad",
    status: "approved",
    named_by: "owner",
    user_agent: "Mozilla/5.0 (iPad; CPU OS 18_0) Safari/605.1",
    fingerprint: "a7:2f:9c",
    first_seen: NOW - 8000,
    last_seen: NOW - 100,
    ...over,
  };
}

/** Answer each socket method separately: these views ask two at once. */
function answers(byMethod: Record<string, unknown>) {
  call.mockImplementation(((cmd: string, args?: unknown) => {
    const method = (args as { method?: string } | undefined)?.method;
    if (cmd !== "kt_call" || !method) return Promise.resolve({});
    return Promise.resolve(byMethod[method] ?? []);
  }) as typeof invoke);
}

function paramsOf(method: string) {
  const match = call.mock.calls.find(
    ([cmd, args]) => cmd === "kt_call" && (args as { method?: string })?.method === method,
  );
  return match?.[1] as { method: string; params?: Record<string, unknown> } | undefined;
}

function show(node: React.ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

describe("AppActivity", () => {
  beforeEach(() => {
    call.mockReset();
    answers({});
  });

  it("asks only for this app's history", async () => {
    show(<AppActivity app={app()} />);
    await waitFor(() => expect(paramsOf("log.query")).toBeDefined());
    expect(paramsOf("log.query")?.params?.slug).toBe("trip-planner");
  });

  it("reads a row as a sentence, with the device's fingerprint under it", async () => {
    answers({
      "log.query": [event({ device_id: "dev-1" })],
      "device.list": [device()],
    });
    show(<AppActivity app={app()} />);

    await waitFor(() =>
      expect(screen.getByText("Kitchen iPad opened the app")).toBeDefined(),
    );
    expect(screen.getByText(/a7:2f:9c/)).toBeDefined();
    expect(screen.getByText("2m ago")).toBeDefined();
  });

  it("does not claim an empty history while it is still asking", async () => {
    // "Nothing has happened" and "we have not asked yet" are different claims,
    // and this is the list an owner checks to be sure nobody opened something.
    // Every call is held, not just the log: this view asks for the devices to
    // name the rows with as well, and releasing one of the two would leave the
    // query under test still pending.
    const waiting: Array<() => void> = [];
    call.mockImplementation(
      (() =>
        new Promise<unknown[]>((resolve) =>
          waiting.push(() => resolve([])),
        )) as unknown as typeof invoke,
    );
    show(<AppActivity app={app()} />);

    expect(screen.getByText(/Reading the log/)).toBeDefined();
    expect(screen.queryByText(/Nothing yet/)).toBeNull();
    waiting.forEach((release) => release());
    await waitFor(() => expect(screen.getByText(/Nothing yet/)).toBeDefined());
  });

  it("says the daemon did not answer rather than showing an empty list", async () => {
    call.mockImplementation((() => Promise.reject(new Error("gone"))) as typeof invoke);
    show(<AppActivity app={app()} />);

    await waitFor(() => expect(screen.getByText(/did not answer/)).toBeDefined());
  });

  it("narrows to opens, and says so when a filter finds nothing", async () => {
    answers({
      "log.query": [event(), event({ action: "paired", actor: "owner", at: NOW - 300 })],
    });
    show(<AppActivity app={app()} />);

    await waitFor(() => expect(screen.getByText(/opened the app/)).toBeDefined());
    expect(screen.getByText(/was let in/)).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "Opens" }));
    expect(screen.queryByText(/was let in/)).toBeNull();
    expect(screen.getByText(/opened the app/)).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "Devices" }));
    expect(screen.queryByText(/opened the app/)).toBeNull();
    expect(screen.getByText(/was let in/)).toBeDefined();
  });

  it("offers Deploys but will not let it empty the list", async () => {
    // Shown disabled with the reason rather than dropped, which is what the
    // Sharing tab does with Strict: the mockup keeps its four chips and
    // nothing pretends to work.
    answers({ "log.query": [event()] });
    show(<AppActivity app={app()} />);

    await waitFor(() => expect(screen.getByText(/opened the app/)).toBeDefined());
    const deploys = screen.getByRole("button", { name: "Deploys" });
    expect(deploys.hasAttribute("disabled")).toBe(true);

    fireEvent.click(deploys);
    expect(screen.getByText(/opened the app/)).toBeDefined();
  });
});

describe("WorkspaceActivity", () => {
  beforeEach(() => {
    call.mockReset();
    answers({});
  });

  it("asks for every app's history at once", async () => {
    show(<WorkspaceActivity apps={[app()]} onNavigate={vi.fn()} />);
    await waitFor(() => expect(paramsOf("log.query")).toBeDefined());
    expect(paramsOf("log.query")?.params?.slug).toBeUndefined();
  });

  it("names the app on every row and opens it", async () => {
    const onNavigate = vi.fn();
    answers({ "log.query": [event({ app_slug: "trip-planner" })] });
    show(<WorkspaceActivity apps={[app()]} onNavigate={onNavigate} />);

    await waitFor(() => expect(screen.getByRole("button", { name: "Trip Planner" })).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Trip Planner" }));
    expect(onNavigate).toHaveBeenCalledWith({
      kind: "app",
      slug: "trip-planner",
      tab: "activity",
    });
  });

  it("keeps a forgotten app's rows, and does not offer to open one", async () => {
    // The log is append-only and forgetting an app leaves its folder alone, so
    // a row can name an app the library no longer lists. Dropping those rows
    // would quietly edit the history the log exists to keep.
    answers({ "log.query": [event({ app_slug: "old-notes" })] });
    show(<WorkspaceActivity apps={[app()]} onNavigate={vi.fn()} />);

    await waitFor(() => expect(screen.getByText("old-notes")).toBeDefined());
    expect(screen.queryByRole("button", { name: "old-notes" })).toBeNull();
  });

  it("says nothing has happened only once it knows", async () => {
    show(<WorkspaceActivity apps={[app()]} onNavigate={vi.fn()} />);
    await waitFor(() => expect(screen.getByText(/Nothing has happened yet/)).toBeDefined());
  });

  it("does not say the app at both ends of the same row", async () => {
    answers({ "log.query": [event({ app_slug: "trip-planner" })] });
    show(<WorkspaceActivity apps={[app()]} onNavigate={vi.fn()} />);

    await waitFor(() => expect(screen.getByText(/opened it/)).toBeDefined());
    expect(screen.queryByText(/opened the app/)).toBeNull();
  });

  it("draws every tile alike, including a row that names no app", async () => {
    // Some workspace events have no app - pairing a device is the common one.
    // Deciding a row's shape from its own slug gave those the single-app
    // treatment, so a smaller tile in a different colour sat in the middle of
    // the column and bent it.
    answers({
      "log.query": [
        event({ app_slug: "trip-planner" }),
        event({ action: "paired", actor: "owner", at: NOW - 300 }),
      ],
    });
    const { container } = show(
      <WorkspaceActivity apps={[app()]} onNavigate={vi.fn()} />,
    );

    await waitFor(() => expect(screen.getByText(/was let in/)).toBeDefined());
    const tiles = Array.from(
      container.querySelectorAll<HTMLElement>('span[aria-hidden="true"]'),
    );
    expect(tiles).toHaveLength(2);
    expect(new Set(tiles.map((tile) => tile.style.width))).toEqual(
      new Set(["32px"]),
    );

    // The one with an app says which app, in colour and nothing else. The one
    // without falls back to saying what happened, because a blank tile on a
    // row that names no app answers nothing at all.
    const [withApp, withoutApp] = tiles;
    expect(withApp?.textContent).toBe("");
    expect(withoutApp?.textContent).toBe("✓");
  });
});
