import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { vi } from "vitest";
import type { Device } from "../devices";
import { Devices } from "./Devices";

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
    first_seen: Math.floor(Date.now() / 1000) - 60,
    last_seen: Math.floor(Date.now() / 1000) - 60,
    ...over,
  };
}

function withDevices(devices: Device[]) {
  call.mockImplementation(((cmd: string, args?: unknown) => {
    const method = (args as { method?: string } | undefined)?.method;
    if (cmd === "kt_call" && method === "device.list") return Promise.resolve(devices);
    return Promise.resolve({});
  }) as typeof invoke);
}

function show() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <Devices />
    </QueryClientProvider>,
  );
}

/**
 * The params of a socket call to `method`, if one was made.
 *
 * Matched across every call rather than taken from the last one: `device.list`
 * polls every two seconds, so "the most recent call" is a race.
 */
function paramsOf(method: string) {
  const match = call.mock.calls.find(
    ([cmd, args]) => cmd === "kt_call" && (args as { method?: string })?.method === method,
  );
  return match?.[1] as { method: string; params?: Record<string, unknown> } | undefined;
}

describe("Devices", () => {
  beforeEach(() => {
    call.mockReset();
    withDevices([]);
  });

  it("says so when nothing has ever paired", async () => {
    show();
    await waitFor(() => expect(screen.getByText(/Nothing has been paired yet/)).toBeDefined());
  });

  it("shows a waiting device with enough to identify it", async () => {
    withDevices([device()]);
    show();

    await waitFor(() => expect(screen.getByText("iPhone wants access")).toBeDefined());
    expect(screen.getByText(/f3:9a:21 · Safari · iPhone/)).toBeDefined();
  });

  it("approves with the name in the field, in one call", async () => {
    // Two round trips could leave a device approved but unnamed, and a list of
    // four devices called "iPhone" is not a list anyone can revoke from.
    withDevices([device()]);
    show();

    await waitFor(() => expect(screen.getByText("iPhone wants access")).toBeDefined());
    fireEvent.change(screen.getByLabelText("Name this device"), {
      target: { value: "Kitchen iPad" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));

    await waitFor(() => expect(paramsOf("device.approve")).toBeDefined());
    expect(paramsOf("device.approve")?.params).toEqual({
      id: "dev-1",
      name: "Kitchen iPad",
    });
  });

  it("denies a waiting device", async () => {
    withDevices([device()]);
    show();

    await waitFor(() => expect(screen.getByText("iPhone wants access")).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Deny" }));

    await waitFor(() => expect(paramsOf("device.deny")).toBeDefined());
    expect(paramsOf("device.deny")?.params).toEqual({ id: "dev-1" });
  });

  it("answers whoever asked first", async () => {
    withDevices([
      device({ id: "second", name: "iPad", first_seen: 200 }),
      device({ id: "first", name: "Android", first_seen: 100 }),
    ]);
    show();

    await waitFor(() => expect(screen.getByText("Android wants access")).toBeDefined());
    const asking = screen.getAllByText(/wants access/);
    expect(asking[0]?.textContent).toBe("Android wants access");
  });

  it("lists approved devices and lets them be revoked", async () => {
    withDevices([device({ status: "approved", name: "Kitchen iPad" })]);
    show();

    await waitFor(() => expect(screen.getByText("Approved devices · 1")).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Revoke" }));

    await waitFor(() => expect(paramsOf("device.revoke")).toBeDefined());
  });

  it("renames an approved device in place", async () => {
    withDevices([device({ status: "approved", name: "iPhone" })]);
    show();

    await waitFor(() => expect(screen.getByText("Approved devices · 1")).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "iPhone" }));

    const field = screen.getByLabelText("Rename iPhone");
    fireEvent.change(field, { target: { value: "Kitchen iPad" } });
    fireEvent.keyDown(field, { key: "Enter" });

    await waitFor(() => expect(paramsOf("device.rename")).toBeDefined());
    expect(paramsOf("device.rename")?.params).toEqual({ id: "dev-1", name: "Kitchen iPad" });
  });

  it("will not offer to revoke this machine", async () => {
    // Revoking the owner would lock someone out of their own library.
    withDevices([device({ status: "owner", name: "This Mac" })]);
    show();

    await waitFor(() => expect(screen.getByText("This machine")).toBeDefined());
    expect(screen.queryByRole("button", { name: "Revoke" })).toBeNull();
  });

  it("keeps refused devices visible rather than forgetting them", async () => {
    withDevices([device({ status: "revoked", name: "Unknown iPad" })]);
    show();

    await waitFor(() => expect(screen.getByText("Refused · 1")).toBeDefined());
    const row = screen.getByText("Unknown iPad").closest("div");
    expect(row).not.toBeNull();
    expect(within(row!.parentElement!).getByText("Refused")).toBeDefined();
  });
});
