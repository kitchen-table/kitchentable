import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { vi } from "vitest";
import type { Device } from "../devices";
import { PairingModal } from "./PairingModal";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue({}) }));

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

function show(over: Partial<Device> = {}, waiting = 1) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <PairingModal device={device(over)} waiting={waiting} onClose={() => undefined} />
    </QueryClientProvider>,
  );
}

describe("PairingModal", () => {
  it("pre-fills the name and says it was only a guess", () => {
    show({ name: "iPhone", named_by: "guess" });

    expect(screen.getByLabelText(/Name this device/)).toBeDefined();
    expect(screen.getByText(/guessed from the browser/)).toBeDefined();
  });

  it("says when the name came from the device itself", () => {
    // A better name appearing from nowhere is unsettling for a product whose
    // pitch is that nothing leaves your desk. Say where it came from.
    show({ name: "Kitchen iPad", named_by: "network" });

    expect(screen.getByText(/name this device gives itself on your network/)).toBeDefined();
    expect(screen.queryByText(/guessed from the browser/)).toBeNull();
  });

  it("shows the fingerprint and browser so the owner can match the phone in their hand", () => {
    show();
    expect(screen.getByText(/f3:9a:21 · Safari · iPhone/)).toBeDefined();
  });

  it("mentions a queue only when there is one", () => {
    const { unmount } = show({}, 3);
    expect(screen.getByText("2 more waiting")).toBeDefined();
    unmount();

    show({}, 1);
    expect(screen.queryByText(/more waiting/)).toBeNull();
  });

  it("offers both answers", () => {
    show();
    expect(screen.getByRole("button", { name: "Approve & pair" })).toBeDefined();
    expect(screen.getByRole("button", { name: "Deny" })).toBeDefined();
  });
});
