import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, vi } from "vitest";
import { Sharing } from "./Sharing";
import { expiry } from "./activity";
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
    paused: false,
    relay: "off",
    public_label: "trip",
    url: "http://trip.local",
    fallback_url: "http://192.168.0.5/trip",
    loopback_url: "http://localhost/trip",
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

  it("marks Public as needing the relay, so the picker stops promising reach it has not got", () => {
    renderSharing();
    const publicRow = screen.getByRole("button", { name: /Public/ });
    expect(publicRow.textContent).toContain("needs relay");

    // Only that row. The other three do not reach anywhere by design.
    for (const label of ["Private", "Household", "Invited"]) {
      const row = screen.getByRole("button", { name: new RegExp(label) });
      expect(row.textContent).not.toContain("needs relay");
    }
  });
});

describe("Sharing: the relay", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue([]);
  });

  it("reports an unpublished app as off, and never guesses at a published one", () => {
    renderSharing({ visibility: "invited", relay: "off" });
    expect(screen.getByText("OFF")).toBeDefined();

    cleanup();
    // Published, but the daemon has not said whether the tunnel is up. Not
    // knowing is its own state: this used to render a confident ON.
    renderSharing({ visibility: "invited", relay: "standard" });
    expect(screen.getByText("CHECKING")).toBeDefined();
    expect(screen.queryByText("ON")).toBeNull();
  });

  it("does not publish an app without asking how it should be served", async () => {
    renderSharing({ visibility: "invited" });
    fireEvent.click(screen.getByRole("button", { name: "Turn on relay" }));

    // The switch opens the choice. Nothing reaches the daemon until a mode is
    // picked, because defaulting the sensitive case silently is the whole
    // thing this modal exists to prevent.
    expect(screen.getByRole("dialog")).toBeDefined();
    expect(invoke).not.toHaveBeenCalledWith(
      "kt_call",
      expect.objectContaining({ method: "share.set_relay" }),
    );
  });

  it("publishes with the mode the owner picked", async () => {
    renderSharing({ visibility: "invited" });
    fireEvent.click(screen.getByRole("button", { name: "Turn on relay" }));
    fireEvent.click(screen.getByRole("button", { name: /Stay reachable/ }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("kt_call", {
        method: "share.set_relay",
        params: { slug: "trip", mode: "standard" },
      });
    });
  });

  it("suggests Strict for Invited and Standard for Public, without applying it", () => {
    renderSharing({ visibility: "invited" });
    fireEvent.click(screen.getByRole("button", { name: "Turn on relay" }));
    expect(screen.getByText(/Suggested for Invited apps:/).textContent).toContain(
      "Strict",
    );

    cleanup();
    renderSharing({ visibility: "public" });
    fireEvent.click(screen.getByRole("button", { name: "Turn on relay" }));
    expect(screen.getByText(/Suggested for Public apps:/).textContent).toContain(
      "Standard",
    );
  });

  it("offers Strict but refuses to pretend it works", () => {
    // Strict has no transport until C4. Quietly serving Standard in its place
    // would hand the owner the mode where the operator *can* read the app,
    // having just been told it cannot. Disabled, and said out loud.
    renderSharing({ visibility: "invited" });
    fireEvent.click(screen.getByRole("button", { name: "Turn on relay" }));

    const strict = screen.getByRole("button", { name: /Full privacy/ });
    expect(strict.hasAttribute("disabled")).toBe(true);
    expect(strict.textContent).toContain("no transport yet");

    fireEvent.click(strict);
    expect(invoke).not.toHaveBeenCalledWith(
      "kt_call",
      expect.objectContaining({ method: "share.set_relay" }),
    );
  });

  it("turns the relay off in one click, with no second question", async () => {
    // Switching on is a choice; switching off is a retreat to the safe state
    // and never needs confirming.
    renderSharing({ visibility: "invited", relay: "standard" });
    fireEvent.click(screen.getByRole("button", { name: "Turn off relay" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("kt_call", {
        method: "share.set_relay",
        params: { slug: "trip", mode: "off" },
      });
    });
  });

  it("never strands a published app with no way to unpublish it", async () => {
    // Reachable in one click: publish as Invited, then set it Private. The
    // block was hidden outright for those levels, so the relay stayed on with
    // the switch-off button hidden behind the very change that stranded it.
    renderSharing({ visibility: "private", relay: "standard" });

    expect(screen.getByRole("alert").textContent).toContain(
      "published but nothing can reach it",
    );
    fireEvent.click(screen.getByRole("button", { name: "Turn off relay" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("kt_call", {
        method: "share.set_relay",
        params: { slug: "trip", mode: "off" },
      });
    });
  });

  it("does not offer the relay for levels it could only ever refuse", () => {
    // A relayed request is neither on loopback nor on the home network, so
    // Private and Household can only produce refusals. An inert switch that
    // does nothing but refuse is the same lie as a blurb promising reach.
    for (const visibility of ["private", "network"] as const) {
      renderSharing({ visibility });
      expect(screen.queryByRole("button", { name: "Turn on relay" })).toBeNull();
      expect(screen.getByText(/stay on this network/)).toBeDefined();
      cleanup();
    }
  });

  it("never lets the picker publish an app as a side effect", async () => {
    renderSharing({ visibility: "private" });
    fireEvent.click(screen.getByRole("button", { name: /Public/ }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("kt_call", {
        method: "share.set_visibility",
        params: { slug: "trip", visibility: "public" },
      });
    });
    expect(invoke).not.toHaveBeenCalledWith(
      "kt_call",
      expect.objectContaining({ method: "share.set_relay" }),
    );
  });

  it("says why there is no public address rather than showing nothing", () => {
    // Publishing with no handle is a real state - every install today. Showing
    // only the local URL made a working switch look broken.
    renderSharing({ visibility: "invited", relay: "standard" });
    expect(screen.getByText(/No public address yet/)).toBeDefined();
  });

  it("shows no public address for an app that is not published", () => {
    renderSharing({ visibility: "invited", relay: "off" });
    expect(screen.queryByText(/No public address yet/)).toBeNull();
    expect(screen.queryByText(/AWAY/)).toBeNull();
  });

  it("says there is no snapshot, because there is not one", () => {
    // Snapshot failover is C5. The mockup's status line is the place that
    // reports actual state, so it reports the actual state.
    renderSharing({ visibility: "invited", relay: "standard" });
    expect(screen.getByText(/No snapshot stored/)).toBeDefined();
  });
});

describe("Sharing: whether it is actually reachable", () => {
  /** A daemon reporting a given tunnel state. */
  function withTunnel(state: unknown, over: Partial<App> = {}) {
    invoke.mockImplementation((_cmd: unknown, args: unknown) => {
      const method = (args as { method: string }).method;
      if (method === "sys.status") {
        return Promise.resolve({
          relay_handle: "adarsh",
          relay_domain: "kitchentable.cloud",
          relay: state,
        });
      }
      return Promise.resolve([]);
    });
    return renderSharing({ visibility: "invited", relay: "standard", ...over });
  }

  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue([]);
  });

  it("says ON only when the tunnel is actually up", async () => {
    withTunnel({ state: "connected" });
    await waitFor(() => expect(screen.getByText("ON")).toBeDefined());
  });

  it("does not claim ON while the tunnel is down", async () => {
    // The whole bug: publishing is an intention, being reachable is a fact.
    // A published app with a dead tunnel showed a confident green ON, and the
    // owner found out from somebody saying the link was broken.
    withTunnel({ state: "connecting" });
    await waitFor(() => expect(screen.getByText("RECONNECTING")).toBeDefined());
    expect(screen.queryByText("ON")).toBeNull();
    expect(screen.getByRole("status").textContent).toContain("not reachable from outside");
  });

  it("shows why it gave up, when it has", async () => {
    withTunnel({ state: "needs_attention", message: "this install was revoked" });
    await waitFor(() => expect(screen.getByText("OFFLINE")).toBeDefined());
    expect(screen.getByRole("status").textContent).toContain("revoked");
  });

  it("flags a published app on a machine that is not even dialling", async () => {
    withTunnel({ state: "off" });
    await waitFor(() => expect(screen.getByText("NOT DIALLING")).toBeDefined());
  });

  it("still says OFF for an app nobody published, whatever the tunnel is doing", async () => {
    withTunnel({ state: "connected" }, { relay: "off" });
    await waitFor(() => expect(screen.getByText("OFF")).toBeDefined());
  });
});

describe("Sharing: the public address", () => {
  /** A daemon that has a handle, so addresses exist at all. */
  function withHandle(over: Partial<App> = {}) {
    invoke.mockImplementation((_cmd: unknown, args: unknown) => {
      const method = (args as { method: string }).method;
      if (method === "sys.status") {
        return Promise.resolve({
          relay_handle: "adarsh",
          relay_domain: "kitchentable.cloud",
        });
      }
      return Promise.resolve([]);
    });
    return renderSharing({ visibility: "invited", relay: "standard", ...over });
  }

  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue([]);
  });

  it("shows the address as label plus the machine's own suffix", async () => {
    withHandle({ public_label: "trip" });
    await waitFor(() => {
      expect(screen.getByText(/trip-adarsh\.kitchentable\.cloud/)).toBeDefined();
    });
  });

  it("sends the new label, and only the label", async () => {
    withHandle({ public_label: "trip" });
    await waitFor(() => screen.getByRole("button", { name: "Edit" }));
    fireEvent.click(screen.getAllByRole("button", { name: "Edit" })[0]!);

    // Prefilled, so a rename starts from what the address currently is rather
    // than from an empty box.
    const field = screen.getByLabelText("PUBLIC ADDRESS") as HTMLInputElement;
    expect(field.value).toBe("trip");

    fireEvent.change(field, { target: { value: "holiday" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("kt_call", {
        method: "share.set_public_label",
        params: { slug: "trip", label: "holiday" },
      });
    });
  });

  it("warns that renaming breaks the address people already have", async () => {
    // The cost of this edit is invisible otherwise: the owner is retiring a
    // URL somebody may already be holding.
    withHandle();
    await waitFor(() => screen.getByRole("button", { name: "Edit" }));
    fireEvent.click(screen.getAllByRole("button", { name: "Edit" })[0]!);

    expect(screen.getByText(/stop being able to open this app/)).toBeDefined();
  });

  it("shows the daemon's refusal rather than a generic failure", async () => {
    // The daemon holds the length limit and the list of taken names, so it is
    // the only thing that can say which rule was broken.
    withHandle();
    await waitFor(() => screen.getByRole("button", { name: "Edit" }));
    fireEvent.click(screen.getAllByRole("button", { name: "Edit" })[0]!);

    invoke.mockRejectedValueOnce({
      code: "conflict",
      message: "‘chores’ already uses that address. Pick another.",
    });
    fireEvent.change(screen.getByLabelText("PUBLIC ADDRESS"), {
      target: { value: "chores" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(screen.getByRole("alert").textContent).toContain("already uses that address");
    });
  });

  it("offers no address to edit when the machine has no handle", async () => {
    renderSharing({ visibility: "invited", relay: "standard" });
    await waitFor(() => {
      expect(screen.getByText(/No public address yet/)).toBeDefined();
    });
    expect(screen.queryByRole("button", { name: "Edit" })).toBeNull();
  });
});

describe("expiry", () => {
  it("says when a link expires rather than only that it does", () => {
    // A chip reading "expires" raises the question and refuses to answer it.
    expect(expiry(null)).toBe("no expiry");
    // Day-then-month or month-then-day is the runner's locale, not ours. The
    // claim is that a date is there at all.
    expect(expiry(1_760_000_000)).toMatch(/^expires .*\d/);
  });
});
