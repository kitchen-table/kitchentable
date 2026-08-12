import { fireEvent, render, screen, within } from "@testing-library/react";
import { Library } from "./Library";
import type { App } from "./generated";

function app(over: Partial<App> = {}): App {
  return {
    slug: "trip-planner",
    name: "Trip Planner",
    entry: "index.html",
    visibility: "private",
    version: 1,
    url: "http://trip-planner.local",
    fallback_url: "http://192.168.0.5/trip-planner",
    path: "/ws/Trip Planner",
    ...over,
  };
}

describe("Library", () => {
  it("lists the apps it is given", () => {
    render(
      <Library
        apps={[app(), app({ slug: "chores", name: "Chores Rota" })]}
        workspace="/ws"
      />,
    );

    expect(screen.getByRole("heading", { name: "Trip Planner" })).toBeDefined();
    expect(screen.getByRole("heading", { name: "Chores Rota" })).toBeDefined();
  });

  it("labels the network level Household, not Network", () => {
    // The wire value stays `network`; only the label changed. Getting this
    // wrong would put an internal name in front of a person.
    render(<Library apps={[app({ visibility: "network" })]} workspace="/ws" />);

    expect(screen.getAllByText("Household").length).toBeGreaterThan(0);
    expect(screen.queryByText("Network")).toBeNull();
  });

  it("filters by visibility and counts each level", () => {
    render(
      <Library
        apps={[
          app({ slug: "a", name: "Alpha", visibility: "private" }),
          app({ slug: "b", name: "Beta", visibility: "invited" }),
          app({ slug: "c", name: "Gamma", visibility: "invited" }),
        ]}
        workspace="/ws"
      />,
    );

    const invited = screen.getByRole("button", { name: /Invited/ });
    expect(within(invited).getByText("2")).toBeDefined();

    fireEvent.click(invited);
    expect(screen.queryByRole("heading", { name: "Alpha" })).toBeNull();
    expect(screen.getByRole("heading", { name: "Beta" })).toBeDefined();
  });

  it("tells someone what to do when the workspace is empty", () => {
    render(<Library apps={[]} workspace="/ws" />);
    expect(screen.getByText(/Drop a folder into your workspace/)).toBeDefined();
  });

  it("flags an app the network made us rename", () => {
    // Otherwise the URL on the card looks like a bug.
    render(
      <Library
        apps={[app({ slug: "notes", hostname: "notes-2.local", url: "http://notes-2.local" })]}
        workspace="/ws"
      />,
    );
    expect(screen.getByText("renamed")).toBeDefined();
  });

  it("does not flag an app that got the name it asked for", () => {
    render(
      <Library
        apps={[app({ slug: "notes", hostname: "notes.local", url: "http://notes.local" })]}
        workspace="/ws"
      />,
    );
    expect(screen.queryByText("renamed")).toBeNull();
  });

  it("links each app to its live URL", () => {
    render(<Library apps={[app()]} workspace="/ws" />);
    const link = screen.getByRole("link", { name: "trip-planner.local" });
    expect(link.getAttribute("href")).toBe("http://trip-planner.local");
  });
});
