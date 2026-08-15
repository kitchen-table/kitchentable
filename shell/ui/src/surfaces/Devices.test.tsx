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

  it("shows one row for a phone that has lost its cookie three times", async () => {
    // The bug this fixes: thirty-six rows for four physical devices, because a
    // device id is minted fresh whenever a request arrives without a usable
    // cookie. The list is the security surface of the product and it was
    // mostly archaeology.
    withDevices([
      device({ id: "a", status: "approved", name: "iPhone", last_seen: 300 }),
      device({ id: "b", status: "approved", name: "iPhone", last_seen: 200 }),
      device({ id: "c", status: "approved", name: "iPhone", last_seen: 100 }),
    ]);
    show();

    await waitFor(() =>
      expect(screen.getByText("Approved devices · 1 (3 sessions)")).toBeDefined(),
    );
    expect(screen.getAllByRole("button", { name: "iPhone" })).toHaveLength(1);
    expect(screen.getByRole("button", { name: /3 sessions/ })).toBeDefined();
  });

  it("keeps two different devices apart even when both are iPhones", async () => {
    withDevices([
      device({ id: "a", status: "approved", name: "Priya's iPhone" }),
      device({ id: "b", status: "approved", name: "Sam's iPhone" }),
    ]);
    show();

    await waitFor(() => expect(screen.getByText("Approved devices · 2")).toBeDefined());
  });

  it("never folds this machine into a group with anything else", async () => {
    // "This machine" on a row that also stands for two strangers would be a
    // straightforward lie about who is trusted.
    withDevices([
      device({ id: "owner", status: "owner", name: "Mac", user_agent: "Safari" }),
      device({ id: "other", status: "approved", name: "Mac", user_agent: "Safari" }),
    ]);
    show();

    await waitFor(() => expect(screen.getByText("This machine")).toBeDefined());
    expect(screen.getByText("Approved devices · 2")).toBeDefined();
  });

  it("revokes every session in a group, not just the newest", async () => {
    // Revoking one row of three left the other two approved, so the most
    // consequential control in this tab did less than it said.
    withDevices([
      device({ id: "a", status: "approved", name: "iPhone", last_seen: 300 }),
      device({ id: "b", status: "approved", name: "iPhone", last_seen: 200 }),
    ]);
    show();

    await waitFor(() =>
      expect(screen.getByRole("button", { name: /2 sessions/ })).toBeDefined(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Revoke" }));

    await waitFor(() => {
      const revocations = call.mock.calls.filter(
        ([cmd, args]) =>
          cmd === "kt_call" && (args as { method?: string })?.method === "device.revoke",
      );
      expect(revocations).toHaveLength(2);
    });
  });

  it("asks before removing a row, and says what removing means", async () => {
    withDevices([device({ status: "approved", name: "Old iPad" })]);
    show();

    await waitFor(() => expect(screen.getByText("Approved devices · 1")).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));

    expect(screen.getByText(/it will ask to be let in, as a new device/)).toBeDefined();
    expect(paramsOf("device.forget")).toBeUndefined();

    fireEvent.click(screen.getAllByRole("button", { name: "Remove" }).at(-1)!);
    await waitFor(() => expect(paramsOf("device.forget")).toBeDefined());
    expect(paramsOf("device.forget")?.params).toEqual({ id: "dev-1" });
  });

  it("warns that removing a refused device drops the refusal", async () => {
    // Removing a revoked row is a step backwards - the refusal becomes another
    // prompt - so an owner tidying the list must not do it believing the
    // refusal still holds.
    withDevices([device({ status: "revoked", name: "Unknown iPad" })]);
    show();

    await waitFor(() => expect(screen.getByText("Refused · 1")).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));

    expect(screen.getByText(/removing the row drops that refusal/)).toBeDefined();
  });

  it("will not offer to remove this machine", async () => {
    withDevices([device({ status: "owner", name: "This Mac" })]);
    show();

    await waitFor(() => expect(screen.getByText("This machine")).toBeDefined());
    expect(screen.queryByRole("button", { name: "Remove" })).toBeNull();
  });

  it("lists the sessions behind a group so one can be removed on its own", async () => {
    withDevices([
      device({ id: "a", status: "approved", name: "iPhone", fingerprint: "aa:11:22", last_seen: 300 }),
      device({ id: "b", status: "approved", name: "iPhone", fingerprint: "bb:33:44", last_seen: 200 }),
    ]);
    show();

    await waitFor(() =>
      expect(screen.getByRole("button", { name: /2 sessions/ })).toBeDefined(),
    );
    fireEvent.click(screen.getByRole("button", { name: /2 sessions/ }));

    expect(screen.getByText(/bb:33:44/)).toBeDefined();
    const removals = screen.getAllByRole("button", { name: "Remove" });
    fireEvent.click(removals.at(-1)!);

    await waitFor(() => expect(paramsOf("device.forget")).toBeDefined());
    expect(paramsOf("device.forget")?.params).toEqual({ id: "b" });
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
