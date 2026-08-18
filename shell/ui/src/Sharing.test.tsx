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
    storage: "synced",
    storage_backup: true,
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

/**
 * A daemon that has a handle, so publishing is possible at all.
 *
 * Needed by every test that reaches the Turn-on button, because without a
 * handle there is no public hostname for any app here and the button is
 * deliberately not drawn — see `RelayHasNowhereToGo`. The default mock has no
 * handle, which is the state of every install today.
 */
function withHandle() {
  invoke.mockImplementation((_cmd: unknown, args: unknown) => {
    const method = (args as { method: string }).method;
    if (method === "sys.status") {
      return Promise.resolve({
        relay_handle: "adarsh",
        relay_domain: "kitchentable.cloud",
        storage: "synced",
        storage_backup: true,
      });
    }
    return Promise.resolve([]);
  });
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

  /** Open the turn-on modal on a machine that can actually publish. */
  async function openTheGate(over: Partial<App> = {}) {
    withHandle();
    renderSharing({ visibility: "invited", ...over });
    const button = await screen.findByRole("button", { name: "Turn on relay" });
    await waitFor(() => expect(button.hasAttribute("disabled")).toBe(false));
    fireEvent.click(button);
  }

  it("does not offer the switch when this machine has no handle", async () => {
    // The default mock has no handle, which is every install today. Publishing
    // needs `<label>-<handle>.<domain>`, so with no handle there is no name for
    // any app here — pressing the button wrote `relay: standard` into the
    // manifest, changed nothing anybody could reach, and explained itself
    // afterwards. A switch that can only do nothing must not be drawn.
    renderSharing({ visibility: "invited" });

    expect(await screen.findByText(/Not available yet/)).toBeDefined();
    expect(screen.getByText(/nowhere to publish to yet/)).toBeDefined();
    const button = screen.queryByRole("button", { name: "Turn on relay" });
    expect(button).toBeNull();
  });

  it("offers the upgrade where the switch cannot be, and opens the browser", async () => {
    // The card that says "no handle" is the one place the person is already
    // asking for exactly what the paid tier sells, so it carries the way in.
    // The ceremony is the browser's: the daemon answers with the page to open
    // and then notices the link landing by itself.
    invoke.mockImplementation((_cmd: unknown, args: unknown) => {
      const method = (args as { method: string }).method;
      if (method === "account.begin_upgrade") {
        return Promise.resolve({
          url: "https://kitchentable.cloud/upgrade?install=k3y",
        });
      }
      return Promise.resolve([]);
    });
    const opened = vi.spyOn(window, "open").mockReturnValue(null);
    renderSharing({ visibility: "invited" });

    fireEvent.click(await screen.findByRole("button", { name: "Link an account" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("kt_call", {
        method: "account.begin_upgrade",
        params: null,
      });
      // No Tauri host under vitest, so the browser fallback is the evidence
      // the URL went somewhere rather than being returned into silence.
      expect(opened).toHaveBeenCalledWith(
        "https://kitchentable.cloud/upgrade?install=k3y",
        "_blank",
        "noopener",
      );
    });
    opened.mockRestore();
  });

  it("says finish-in-your-browser while the checkout is in flight", async () => {
    // "Not linked" beside a checkout somebody is midway through reads as the
    // button having failed. Waiting is its own state, drawn as one.
    invoke.mockImplementation((_cmd: unknown, args: unknown) => {
      const method = (args as { method: string }).method;
      if (method === "account.status") {
        return Promise.resolve({ install_key: "k3y", link: { state: "waiting" } });
      }
      return Promise.resolve([]);
    });
    renderSharing({ visibility: "invited" });

    expect(await screen.findByText("Finish in your browser")).toBeDefined();
    expect(screen.getByRole("button", { name: "Reopen the page" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "Link an account" })).toBeNull();
  });

  it("waits for the daemon before claiming the switch works", async () => {
    // Unknown is its own state. Drawing the offer as live before the daemon has
    // said whether there is a handle is a claim about the machine, not a delay.
    withHandle();
    renderSharing({ visibility: "invited" });

    expect(
      screen.getByRole("button", { name: "Turn on relay" }).hasAttribute("disabled"),
    ).toBe(true);
    await waitFor(() =>
      expect(
        screen
          .getByRole("button", { name: "Turn on relay" })
          .hasAttribute("disabled"),
      ).toBe(false),
    );
  });

  it("does not publish an app without asking how it should be served", async () => {
    await openTheGate();

    // The switch opens the confirmation. Nothing reaches the daemon until it is
    // answered, because publishing as a side effect of any other action is the
    // whole thing this modal exists to prevent.
    expect(screen.getByRole("dialog")).toBeDefined();
    expect(invoke).not.toHaveBeenCalledWith(
      "kt_call",
      expect.objectContaining({ method: "share.set_relay" }),
    );
  });

  it("publishes with the mode the owner picked", async () => {
    await openTheGate();
    fireEvent.click(screen.getByRole("button", { name: /Stay reachable/ }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("kt_call", {
        method: "share.set_relay",
        params: { slug: "trip", mode: "standard" },
      });
    });
  });

  it("names Standard at every visibility, without applying it", async () => {
    // This used to suggest Strict for Invited. Strict has no snapshot, so a
    // Strict app dies with the laptop — and Invited is the common case, so the
    // suggestion pointed most owners at the mode that turns off the thing the
    // paid tier is bought for. See `suggestedFor` in relay.ts.
    await openTheGate();
    expect(screen.getByText(/Invited apps use/).textContent).toContain(
      "Standard",
    );

    cleanup();
    await openTheGate({ visibility: "public" });
    expect(screen.getByText(/Public apps use/).textContent).toContain(
      "Standard",
    );
  });

  it("does not draw a choice, because there is only one mode", async () => {
    // Strict is deferred, so publishing has one answer. A radio pair with one
    // working option is a choice drawn where none exists — the same fault the
    // rest of this tab was built to remove. The modal is a confirmation of what
    // publishing does instead, and it still says where the missing mode went.
    await openTheGate();

    expect(screen.queryByRole("button", { name: /Full privacy/ })).toBeNull();
    expect(screen.getByText(/full-privacy mode/).textContent).toContain(
      "designed and not built",
    );
  });

  it("shows one mode card once the relay is on, and it is not a control", () => {
    // The block used to draw Standard and Strict side by side with radios, so
    // an owner could switch. With one mode there is nothing to switch to, and a
    // radio that cannot change anything is a lie about what you may do.
    renderSharing({ visibility: "invited", relay: "standard" });

    expect(screen.queryByRole("button", { name: /Full privacy/ })).toBeNull();
    expect(
      screen.queryByRole("button", { name: /Stay reachable/ }),
    ).toBeNull();
    expect(screen.getByText("Standard")).toBeTruthy();
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
          storage: "synced",
          storage_backup: true,
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
