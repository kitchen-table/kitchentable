import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { vi } from "vitest";
import type { App } from "../generated";
import { DangerZone } from "./DangerZone";

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
    public_label: "trip",
    url: "http://trip-planner.local",
    fallback_url: "http://192.168.0.5/trip-planner",
    path: "/ws/Trip Planner",
    size_bytes: 42_000,
    entry_exists: true,
    ...over,
  };
}

function show(over: Partial<App> = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const onGone = vi.fn();
  render(
    <QueryClientProvider client={client}>
      <DangerZone app={app(over)} onGone={onGone} />
    </QueryClientProvider>,
  );
  return { onGone };
}

/** The method names sent to the daemon, in order. */
function calls(): string[] {
  return call.mock.calls.map((c) => (c[1] as { method: string }).method);
}

describe("DangerZone", () => {
  beforeEach(() => {
    call.mockReset();
    call.mockResolvedValue({});
  });

  it("asks before pausing, because readers get no warning", () => {
    show();
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));

    expect(screen.getByText(/stop answering for everyone, including you/i)).toBeDefined();
    expect(calls()).toEqual([]);
  });

  it("pauses once confirmed", async () => {
    show();
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    fireEvent.click(screen.getByRole("button", { name: "Pause serving" }));

    await waitFor(() => expect(calls()).toEqual(["app.pause"]));
  });

  it("resumes without asking, because putting it back harms nobody", async () => {
    show({ paused: true });
    fireEvent.click(screen.getByRole("button", { name: "Resume" }));

    await waitFor(() => expect(calls()).toEqual(["app.resume"]));
  });

  it("says who a paused app is refusing, which is everyone", () => {
    show({ paused: true });
    expect(screen.getByText(/Nobody can open it, including you/)).toBeDefined();
    expect(screen.getByText(/Their links still work/)).toBeDefined();
  });

  it("promises the folder is untouched, on the button and in the question", () => {
    // "Delete" beside a folder of someone's work has to be unambiguous about
    // what it does and does not remove.
    show();
    expect(screen.getByText(/folder on disk is left untouched/)).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(screen.getByText(/folder stays exactly where it is on disk/)).toBeDefined();
    expect(calls()).toEqual([]);
  });

  it("forgets once confirmed, and leaves the surface behind", async () => {
    const { onGone } = show();
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    fireEvent.click(screen.getByRole("button", { name: "Delete from Kitchen Table" }));

    await waitFor(() => expect(calls()).toEqual(["app.forget"]));
    // Staying on the detail view of an app that no longer exists would show a
    // page about nothing.
    await waitFor(() => expect(onGone).toHaveBeenCalled());
  });

  it("cancelling does nothing at all", () => {
    show();
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(calls()).toEqual([]);
    expect(screen.queryByText(/folder stays exactly where it is/)).toBeNull();
  });

  it("shows what went wrong rather than looking like it worked", async () => {
    call.mockRejectedValue({ code: "io", message: "the manifest is read-only" });
    show();
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    fireEvent.click(screen.getByRole("button", { name: "Pause serving" }));

    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toContain("read-only"),
    );
  });
});
