import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { vi } from "vitest";
import type { SysStatus } from "../generated";
import type { Appearance } from "../theme";
import { Settings } from "./Settings";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-autostart", () => ({
  enable: vi.fn(),
  disable: vi.fn(),
  isEnabled: vi.fn(),
}));

const call = vi.mocked(invoke);
const pick = vi.mocked(open);
const onAtLogin = vi.mocked(enable);
const offAtLogin = vi.mocked(disable);
const atLogin = vi.mocked(isEnabled);

function status(over: Partial<SysStatus> = {}): SysStatus {
  return {
    protocol_version: 1,
    daemon_version: "0.1.0",
    workspace: "/Users/adarsh/KitchenTable",
    serving: { state: "serving" },
    app_count: 3,
    install_key: null,
    relay: { state: "off" },
    uptime_secs: 60,
    workspace_locked: false,
    ...over,
  };
}

/**
 * Answer `kt_health` and the socket separately.
 *
 * The workspace card asks both: the socket to store a choice, and health to
 * find out whether this app is the one that could restart the daemon.
 */
function hosted({
  weStartedIt = true,
  socket,
}: {
  weStartedIt?: boolean;
  socket?: (method: string, params: unknown) => unknown;
} = {}) {
  // The picker and the autostart plugin both need a Tauri host to exist.
  (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {};

  call.mockImplementation((command, args) => {
    if (command === "kt_health") {
      return Promise.resolve({ state: "ready", we_started_it: weStartedIt });
    }
    if (command === "kt_restart_daemon") return Promise.resolve(undefined);
    const { method, params } = args as { method: string; params: unknown };
    return Promise.resolve(socket ? socket(method, params) : {});
  });
}

function show(over: Partial<SysStatus> = {}, appearance: Appearance = "system") {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const onAppearance = vi.fn();
  const view = render(
    <QueryClientProvider client={client}>
      <Settings status={status(over)} appearance={appearance} onAppearance={onAppearance} />
    </QueryClientProvider>,
  );
  return { ...view, onAppearance };
}

/**
 * The status chip on one row.
 *
 * Asserted through the row rather than by looking for the chip's text, so a
 * second card growing a chip that happens to say the same word cannot make one
 * of these tests pass for the wrong reason.
 */
function chipFor(rowTitle: string): string | undefined {
  const row = screen.getByText(rowTitle).parentElement?.parentElement;
  return row?.lastElementChild?.textContent ?? undefined;
}

describe("Settings", () => {
  beforeEach(() => {
    call.mockReset();
    pick.mockReset();
    onAtLogin.mockReset();
    offAtLogin.mockReset();
    atLogin.mockReset();
    atLogin.mockResolvedValue(false);
    delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    call.mockResolvedValue({});
  });

  it("shows the folder the daemon is actually watching", () => {
    show();
    expect(screen.getByText("/Users/adarsh/KitchenTable")).toBeDefined();
  });

  it("stores a picked folder and says the change lands on the next start", async () => {
    // The honest half. The daemon reads the setting when it starts, so a card
    // implying the swap has happened would have somebody looking for apps that
    // are not there yet.
    hosted();
    pick.mockResolvedValue("/Users/adarsh/Apps");
    call.mockImplementation((command, args) => {
      if (command === "kt_health") {
        return Promise.resolve({ state: "ready", we_started_it: true });
      }
      const { method } = args as { method: string };
      if (method === "sys.set_workspace") {
        return Promise.resolve({ pending: "/Users/adarsh/Apps", restart_required: true });
      }
      return Promise.resolve({});
    });

    show();
    fireEvent.click(screen.getByRole("button", { name: "Change" }));

    await waitFor(() => expect(screen.getByText(/from its next start/)).toBeDefined());
    expect(screen.getByText("/Users/adarsh/Apps")).toBeDefined();
    // And the folder in the top row is still the one being served from.
    expect(screen.getByText("/Users/adarsh/KitchenTable")).toBeDefined();
  });

  it("offers the restart only when this app is the one that could do it", async () => {
    // The supervisor deliberately leaves a daemon it adopted alone, so pressing
    // Restart on one would do nothing at all.
    hosted({ weStartedIt: false });
    pick.mockResolvedValue("/Users/adarsh/Apps");
    call.mockImplementation((command, args) => {
      if (command === "kt_health") {
        return Promise.resolve({ state: "ready", we_started_it: false });
      }
      const { method } = args as { method: string };
      if (method === "sys.set_workspace") {
        return Promise.resolve({ pending: "/Users/adarsh/Apps", restart_required: true });
      }
      return Promise.resolve({});
    });

    show();
    fireEvent.click(screen.getByRole("button", { name: "Change" }));

    await waitFor(() => expect(screen.getByText(/running outside this app/)).toBeDefined());
    expect(screen.queryByRole("button", { name: /Restart/ })).toBeNull();
  });

  it("shows the daemon's own refusal rather than a generic failure", async () => {
    // "That is a git repository, and every folder inside a workspace gets an
    // app.json written into it" is the sentence somebody standing in a file
    // picker needs. Only the daemon can say it.
    hosted();
    pick.mockResolvedValue("/Users/adarsh/code/project");
    call.mockImplementation((command, args) => {
      if (command === "kt_health") {
        return Promise.resolve({ state: "ready", we_started_it: true });
      }
      const { method } = args as { method: string };
      if (method === "sys.set_workspace") {
        return Promise.reject({
          code: "bad_request",
          message: '"/Users/adarsh/code/project" is a git repository',
        });
      }
      return Promise.resolve({});
    });

    show();
    fireEvent.click(screen.getByRole("button", { name: "Change" }));

    await waitFor(() => expect(screen.getByText(/is a git repository/)).toBeDefined());
  });

  it("says nothing changed when the picker is cancelled", async () => {
    hosted();
    pick.mockResolvedValue(null);

    show();
    fireEvent.click(screen.getByRole("button", { name: "Change" }));

    await waitFor(() => expect(pick).toHaveBeenCalled());
    expect(screen.queryByText(/from its next start/)).toBeNull();
  });

  it("draws the folder as fixed, with the reason, when the environment set it", () => {
    // KT_WORKSPACE outranks the stored setting deliberately. A Change button
    // here would write a value the daemon will never read and appear to do
    // nothing after a restart.
    show({ workspace_locked: true });

    expect(screen.queryByRole("button", { name: "Change" })).toBeNull();
    expect(screen.getByText(/KT_WORKSPACE/)).toBeDefined();
  });

  it("reads launch-at-login from the system rather than assuming it", async () => {
    hosted();
    atLogin.mockResolvedValue(true);

    show();

    const toggle = await waitFor(() => {
      const found = screen.getByRole("switch", { name: "Start Kitchen Table at login" });
      expect(found.getAttribute("aria-checked")).toBe("true");
      return found;
    });

    fireEvent.click(toggle);
    await waitFor(() => expect(offAtLogin).toHaveBeenCalled());
    expect(onAtLogin).not.toHaveBeenCalled();
  });

  it("will not move the login switch before the system has answered", () => {
    // A switch drawn off while the answer is still in flight is a claim about
    // somebody's machine, not a delay.
    hosted();
    atLogin.mockReturnValue(new Promise(() => {}));

    show();

    const toggle = screen.getByRole("switch", { name: "Start Kitchen Table at login" });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(toggle).toHaveProperty("disabled", true);
    fireEvent.click(toggle);
    expect(onAtLogin).not.toHaveBeenCalled();
  });

  it("does not offer launch-at-login outside the desktop app", () => {
    // The same UI runs in a plain browser via `pnpm dev` and under vitest, and
    // §7.10's bug was exactly this: reaching for an IPC that is not there.
    show();

    expect(screen.getByText(/only available in the desktop app/)).toBeDefined();
    expect(atLogin).not.toHaveBeenCalled();
  });

  it("reports mDNS as announcing when the daemon is serving", () => {
    show();
    expect(chipFor("Names on the network")).toBe("announcing");
    expect(screen.getByText(/appname\.local/)).toBeDefined();
  });

  it("says storage goes with the network names when mDNS is down", () => {
    // The quiet failure from §3.9: no announcement means no hostname, which
    // means no storage, and the client falls back to the browser so nobody
    // notices. This row is where somebody would find out.
    show({
      serving: {
        state: "degraded",
        reason: "mdns_unavailable",
        message: "Friendly .local names are not available.",
      },
    });

    expect(chipFor("Names on the network")).toBe("unavailable");
    expect(screen.getByText(/storage is not/)).toBeDefined();
  });

  it("treats a denied Local Network permission as names being unavailable", () => {
    show({
      serving: {
        state: "degraded",
        reason: "local_network_denied",
        message: "macOS is blocking Kitchen Table from the local network.",
      },
    });

    expect(chipFor("Names on the network")).toBe("unavailable");
  });

  it("says nothing about HTTPS at all", () => {
    // The mockup has a certificate row. HTTPS is neither built nor designed, so
    // the row's only content would be an explanation of its own absence - and
    // the status bar's "Not encrypted" badge already says the true thing at the
    // moment somebody cares. Bring the row back when there is a certificate to
    // report on.
    const { container } = show();

    expect(container.textContent).not.toMatch(/HTTPS/);
    expect(container.textContent).not.toMatch(/certificate/i);
  });

  it("says the MCP server is not running, and lists no tools", () => {
    show();
    expect(screen.getByText("not running")).toBeDefined();
    expect(screen.getByText(/no tools to list/)).toBeDefined();
  });

  it("offers system, light and dark, with the current one marked", () => {
    // Three states, not two. "Follows macOS" is the default and is a real
    // answer, and the title bar's button cannot express it.
    const { onAppearance } = show({}, "system");

    const options = screen.getAllByRole("radio");
    expect(options.map((option) => option.textContent)).toEqual(["System", "Light", "Dark"]);
    expect(screen.getByRole("radio", { name: "System" }).getAttribute("aria-checked")).toBe("true");

    fireEvent.click(screen.getByRole("radio", { name: "Dark" }));
    expect(onAppearance).toHaveBeenCalledWith("dark");
  });

  it("does not ship the mockup's review-only switches", () => {
    // "Empty workspace" and "System banners" are scaffolding for reviewing the
    // mockup. In front of somebody's real workspace they are a hazard.
    const { container } = show();

    expect(container.textContent).not.toMatch(/Empty workspace/);
    expect(container.textContent).not.toMatch(/Mockup states/);
    expect(container.textContent).not.toMatch(/Daemon reconnecting/);
  });

  it("draws the relay tier without a button that goes nowhere", () => {
    const { container } = show();

    expect(screen.getByText(/works away from home/)).toBeDefined();
    expect(container.textContent).toMatch(/nothing to sign up to/);
    expect(screen.queryByRole("button", { name: /Upgrade/ })).toBeNull();
  });
});
