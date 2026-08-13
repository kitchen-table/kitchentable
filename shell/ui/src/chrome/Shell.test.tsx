import { fireEvent, render, screen, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { App } from "../generated";
import { Shell } from "./Shell";

function app(over: Partial<App> = {}): App {
  return {
    slug: "trip-planner",
    name: "Trip Planner",
    entry: "index.html",
    visibility: "private",
    version: 1,
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

function show(apps: App[] = [app()], pending = 0) {
  return render(
    <QueryClientProvider client={new QueryClient()}>
      <Shell apps={apps} pending={pending} />
    </QueryClientProvider>,
  );
}

describe("Shell", () => {
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

  it("badges waiting devices, and shows nothing when none are waiting", () => {
    const { unmount } = show([app()], 2);
    const waiting = screen.getByRole("button", { name: "2 devices waiting for approval" });
    expect(within(waiting).getByText("2")).toBeDefined();
    unmount();

    // A zero badge would be a permanent red dot on a product whose whole point
    // is that nothing is happening unless you said so.
    show([app()], 0);
    const idle = screen.getByRole("button", { name: "0 devices waiting for approval" });
    expect(within(idle).queryByText("0")).toBeNull();
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
