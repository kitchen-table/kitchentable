import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { beforeEach, expect, it, vi } from "vitest";
import type { Device } from "../devices";
import type { NotifyState } from "../notify";
import { Notifications } from "./Notifications";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

const call = vi.mocked(invoke);

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

/** Answers every call this surface makes; `over` replaces one of them. */
function answering({
  devices = [] as Device[],
  permission = { state: "granted", alerts: true } as NotifyState,
}) {
  call.mockImplementation(((cmd: string, args?: unknown) => {
    if (cmd === "kt_notify_state" || cmd === "kt_notify_request") {
      return Promise.resolve(permission);
    }
    if (cmd === "kt_notify") return Promise.resolve(null);

    const method = (args as { method?: string } | undefined)?.method;
    switch (method) {
      case "device.list":
        return Promise.resolve(devices);
      case "notify.prefs":
        return Promise.resolve({
          requests: true,
          opens: false,
          deploys: true,
          offline: true,
        });
      case "notify.seen":
      case "notify.mark_seen":
        return Promise.resolve({ at: 0 });
      default:
        return Promise.resolve([]);
    }
  }) as typeof invoke);
}

function show() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={client}>
      <Notifications />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  call.mockReset();
});

/** One preference switch, as an element whose `disabled` can be read. */
function loaded(name: string): HTMLButtonElement {
  return screen.getByRole("switch", { name }) as HTMLButtonElement;
}

it("shows a waiting device with both answers on it", async () => {
  answering({ devices: [device()] });
  show();

  await waitFor(() => expect(screen.getByText(/Priya's iPad wants access/)).toBeDefined());
  expect(screen.getByRole("button", { name: "Approve" })).toBeDefined();
  expect(screen.getByRole("button", { name: "Deny" })).toBeDefined();
  expect(screen.getByText(/e2:77:41/)).toBeDefined();
});

it("approves over the socket, the same way the banner does", async () => {
  answering({ devices: [device()] });
  show();

  await waitFor(() => expect(screen.getByRole("button", { name: "Approve" })).toBeDefined());
  fireEvent.click(screen.getByRole("button", { name: "Approve" }));

  await waitFor(() =>
    expect(call).toHaveBeenCalledWith(
      "kt_call",
      expect.objectContaining({ method: "device.approve" }),
    ),
  );
});

it("says nobody is waiting rather than showing an empty panel", async () => {
  answering({ devices: [] });
  show();

  await waitFor(() => expect(screen.getByText("You’re all caught up")).toBeDefined());
});

it("offers to ask macOS when macOS has not been asked", async () => {
  answering({ permission: { state: "not_determined", alerts: false } });
  show();

  await waitFor(() => expect(screen.getByText(/has not been asked yet/)).toBeDefined());
  fireEvent.click(screen.getByRole("button", { name: "Turn on" }));

  await waitFor(() => expect(call).toHaveBeenCalledWith("kt_notify_request"));
});

it("points at System Settings when macOS is blocking, and offers no button", async () => {
  // macOS asks once and remembers. A "try again" here would do nothing.
  answering({ permission: { state: "denied", alerts: false } });
  show();

  await waitFor(() => expect(screen.getByText(/blocking notifications/)).toBeDefined());
  expect(screen.queryByRole("button", { name: "Turn on" })).toBeNull();
});

it("does not claim banners are showing when banners are switched off", async () => {
  answering({ permission: { state: "granted", alerts: false } });
  show();

  await waitFor(() => expect(screen.getByText(/banners are switched off/)).toBeDefined());
});

it("says a development build cannot show these, instead of failing quietly", async () => {
  answering({ permission: { state: "unbundled", alerts: false } });
  show();

  await waitFor(() => expect(screen.getByText(/need the built app/)).toBeDefined());
});

it("saves a switch to the daemon rather than to the window", async () => {
  answering({});
  show();

  // Every switch is disabled until the daemon has answered - a control drawn
  // from a guess would flip back under the owner's hand.
  await waitFor(() => expect(loaded("Someone opens an app").disabled).toBe(false));
  fireEvent.click(loaded("Someone opens an app"));

  await waitFor(() =>
    expect(call).toHaveBeenCalledWith(
      "kt_call",
      expect.objectContaining({ method: "notify.set_prefs", params: { opens: true } }),
    ),
  );
});

it("disables the two switches nothing can fire, and says why", async () => {
  answering({});
  show();

  // Waiting for a switch that does work, so this cannot pass merely because
  // the card is still loading and everything is disabled.
  await waitFor(() => expect(loaded("Someone opens an app").disabled).toBe(false));

  // A switch that looks live and never fires is worse than one that explains
  // itself: the owner would conclude the feature is broken.
  expect(loaded("A deploy finishes").disabled).toBe(true);
  expect(screen.getByText(/Nothing records a deploy/)).toBeDefined();
  expect(loaded("An app goes offline").disabled).toBe(true);
});

it("will not offer a preview macOS would swallow", async () => {
  answering({ permission: { state: "denied", alerts: false } });
  show();

  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Preview desktop banner" }).hasAttribute("disabled"),
    ).toBe(true),
  );
});

it("posts a real notification for the preview, not a drawing of one", async () => {
  answering({ devices: [device()] });
  show();

  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Preview desktop banner" }).hasAttribute("disabled"),
    ).toBe(false),
  );
  fireEvent.click(screen.getByRole("button", { name: "Preview desktop banner" }));

  await waitFor(() =>
    expect(call).toHaveBeenCalledWith("kt_notify", expect.objectContaining({ spec: expect.any(Object) })),
  );
});
