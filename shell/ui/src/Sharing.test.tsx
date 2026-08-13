import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { vi } from "vitest";
import { Sharing } from "./Sharing";
import type { App } from "./generated";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}));

function app(over: Partial<App> = {}): App {
  return {
    slug: "trip",
    name: "Trip Planner",
    entry: "index.html",
    visibility: "private",
    version: 1,
    url: "http://trip.local",
    fallback_url: "http://192.168.0.5/trip",
    path: "/ws/Trip",
    size_bytes: 42_000,
    deployed_at: 1_760_000_000,
    entry_exists: true,
    ...over,
  };
}

function renderSharing(over: Partial<App> = {}) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <Sharing app={app(over)} />
    </QueryClientProvider>,
  );
}

describe("Sharing", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue([]);
  });

  it("offers every visibility level, with Household not Network", () => {
    renderSharing();
    for (const label of ["Private", "Household", "Invited", "Public"]) {
      expect(screen.getByRole("button", { name: new RegExp(label) })).toBeDefined();
    }
    expect(screen.queryByText(/^Network$/)).toBeNull();
  });

  it("explains what each level means rather than relying on the label", () => {
    renderSharing();
    expect(
      screen.getByText("Anyone on a network you've marked as home"),
    ).toBeDefined();
    expect(screen.getByText("Your own devices only")).toBeDefined();
  });

  it("marks the current level as selected", () => {
    renderSharing({ visibility: "invited" });
    const invited = screen.getByRole("button", { name: /Invited/ });
    expect(invited.getAttribute("aria-pressed")).toBe("true");
  });

  it("sends the wire value when a level is chosen", async () => {
    renderSharing();
    fireEvent.click(screen.getByRole("button", { name: /Household/ }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("kt_call", {
        method: "share.set_visibility",
        params: { slug: "trip", visibility: "network" },
      });
    });
  });

  it("warns that links do nothing unless the app is Invited", () => {
    renderSharing({ visibility: "private" });
    expect(screen.getByText(/only let someone in while this app is set to/)).toBeDefined();
  });

  it("drops that warning once the app is Invited", () => {
    renderSharing({ visibility: "invited" });
    expect(screen.queryByText(/only let someone in while/)).toBeNull();
  });

  it("says so when there are no links yet", async () => {
    renderSharing({ visibility: "invited" });
    await waitFor(() => {
      expect(screen.getByText(/No links yet/)).toBeDefined();
    });
  });
});
