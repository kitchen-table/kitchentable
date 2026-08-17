import { fireEvent, render as rtlRender, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import * as dialog from "@tauri-apps/plugin-dialog";
import { vi } from "vitest";
import { Onboarding } from "./Onboarding";
import { isComplete, reset } from "./steps";
import type { App, SysStatus } from "../generated";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

/** The window resizes for onboarding; nothing here has a window to resize. */
const resized: string[] = [];
vi.mock("@tauri-apps/api/window", () => ({
  LogicalSize: class {
    width: number;
    height: number;
    constructor(width: number, height: number) {
      this.width = width;
      this.height = height;
    }
  },
  getCurrentWindow: () => ({
    setSize: (size: { width: number; height: number }) => {
      resized.push(`${size.width}x${size.height}`);
      return Promise.resolve();
    },
    setMinSize: () => Promise.resolve(),
    setResizable: () => Promise.resolve(),
    center: () => Promise.resolve(),
  }),
}));

/**
 * Onboarding asks the daemon and the OS through queries, exactly as the rest
 * of the window does, so it needs the provider the real app wraps it in.
 */
function render(ui: React.ReactElement) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return rtlRender(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

function status(over: Partial<SysStatus> = {}): SysStatus {
  return {
    protocol_version: 1,
    daemon_version: "0.1.0",
    workspace: "/Users/ada/KitchenTable",
    serving: { state: "serving" },
    app_count: 1,
    install_key: null,
    relay: { state: "off" },
    uptime_secs: 10,
    workspace_locked: false,
    ...over,
  };
}

const welcomeApp: App = {
  slug: "welcome",
  name: "Welcome",
  entry: "index.html",
  visibility: "private",
  version: 1,
  paused: false,
  relay: "off",
  storage: "synced",
  storage_backup: true,
    public_label: "trip",
  url: "http://welcome.local",
  fallback_url: "http://192.168.0.5/welcome",
  loopback_url: "http://localhost/welcome",
  path: "/ws/Welcome",
  size_bytes: 42_000,
  deployed_at: 1_760_000_000,
  entry_exists: true,
};

/// Click forward until the given step is on screen. Reading the counter
/// rather than counting clicks means these tests say which screen they mean.
function goToStep(n: number) {
  for (let guard = 0; guard < 12; guard++) {
    if (screen.queryByText(`Step ${n} of 8`)) return;
    const next =
      screen.queryByRole("button", { name: "Get started" }) ??
      screen.getByRole("button", { name: /Continue|Finish/ });
    fireEvent.click(next);
  }
  throw new Error(`never reached step ${n}`);
}

describe("Onboarding", () => {
  beforeEach(reset);

  it("opens on the welcome screen", () => {
    render(<Onboarding apps={[]} status={status()} onDone={() => {}} />);
    expect(
      screen.getByRole("heading", { name: "Welcome to Kitchen Table" }),
    ).toBeDefined();
  });

  it("walks through all eight steps and finishes", () => {
    const onDone = vi.fn();
    render(<Onboarding apps={[welcomeApp]} status={status()} onDone={onDone} />);

    expect(screen.getByText("Step 1 of 8")).toBeDefined();
    goToStep(8);

    fireEvent.click(screen.getByRole("button", { name: "Finish" }));
    expect(onDone).toHaveBeenCalled();
    expect(isComplete()).toBe(true);
  });

  it("shows the network prompt before the OS does", () => {
    // The whole reason the flow exists: macOS shows this once, and a
    // reflexive Deny would silently kill the product.
    render(<Onboarding apps={[]} status={status()} onDone={() => {}} />);
    goToStep(3);

    expect(screen.getByText(/would like to find and connect to devices/)).toBeDefined();
    expect(screen.getByText(/macOS shows this once/)).toBeDefined();
  });

  it("explains the recovery path when the permission was denied", () => {
    render(
      <Onboarding
        apps={[]}
        status={status({
          serving: {
            state: "degraded",
            reason: "local_network_denied",
            message: "denied",
          },
        })}
        onDone={() => {}}
      />,
    );
    goToStep(3);

    expect(screen.getByText(/can’t see your apps yet/)).toBeDefined();
    expect(screen.getByText(/System Settings/)).toBeDefined();
  });

  it("claims nothing about the network until it has actually asked", async () => {
    // The bug this replaced: the screen drew the dialog, asked nobody, and
    // then reported "Allowed" because the daemon had not complained.
    render(<Onboarding apps={[]} status={status()} onDone={() => {}} />);
    goToStep(3);

    expect(screen.queryByText("Allowed")).toBeNull();
    expect(screen.getByRole("button", { name: "Show me the prompt" })).toBeDefined();
  });

  it("really asks macOS when the button is pressed", async () => {
    vi.mocked(invoke).mockResolvedValue({ state: "allowed" });
    render(<Onboarding apps={[]} status={status()} onDone={() => {}} />);
    goToStep(3);

    fireEvent.click(screen.getByRole("button", { name: "Show me the prompt" }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("kt_local_network"));
    // And only then does it say so.
    await waitFor(() => expect(screen.getByText("Allowed")).toBeDefined());
  });

  it("keeps offering the prompt when nothing answered", async () => {
    // A quiet network and a blocked one look identical from a socket. Saying
    // "denied" here would send somebody to a settings pane to fix nothing.
    vi.mocked(invoke).mockResolvedValue({ state: "unknown" });
    render(<Onboarding apps={[]} status={status()} onDone={() => {}} />);
    goToStep(3);

    fireEvent.click(screen.getByRole("button", { name: "Show me the prompt" }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("kt_local_network"));
    expect(screen.queryByText("Allowed")).toBeNull();
    expect(screen.queryByText(/can’t see your apps yet/)).toBeNull();
    expect(screen.getByRole("button", { name: "Show me the prompt" })).toBeDefined();
  });

  it("offers the way back when macOS refused", async () => {
    vi.mocked(invoke).mockResolvedValue({ state: "denied" });
    render(<Onboarding apps={[]} status={status()} onDone={() => {}} />);
    goToStep(3);

    fireEvent.click(screen.getByRole("button", { name: "Show me the prompt" }));

    await waitFor(() => expect(screen.getByText(/can’t see your apps yet/)).toBeDefined());
    expect(screen.getByText(/System Settings/)).toBeDefined();
    // The mockup's "try again", for somebody who has just flipped the switch.
    expect(screen.getByRole("button", { name: "try again" })).toBeDefined();
  });

  it("takes the shape onboarding was drawn at, and hands the app back its own", async () => {
    // A one-column flow stranded in a 1240x820 workspace window is the ugly
    // state this replaced.
    resized.length = 0;
    const { unmount } = render(
      <Onboarding apps={[]} status={status()} onDone={() => {}} />,
    );

    await waitFor(() => expect(resized).toContain("960x640"));

    unmount();
    await waitFor(() => expect(resized).toContain("1240x820"));
  });

  it("lets the workspace be changed, which is what the step is called", async () => {
    vi.mocked(dialog.open).mockResolvedValue("/Users/ada/Elsewhere");
    vi.mocked(invoke).mockResolvedValue({});
    render(<Onboarding apps={[]} status={status()} onDone={() => {}} />);
    goToStep(2);

    fireEvent.click(screen.getByRole("button", { name: "Choose a different folder…" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "kt_call",
        expect.objectContaining({
          method: "sys.set_workspace",
          params: { path: "/Users/ada/Elsewhere" },
        }),
      ),
    );
    // Stored is not watched: the daemon reads the setting when it starts, and
    // the next step shows what is in the workspace.
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("kt_restart_daemon"));
  });

  it("says why the folder cannot be changed when the environment fixed it", () => {
    render(
      <Onboarding
        apps={[]}
        status={status({ workspace_locked: true })}
        onDone={() => {}}
      />,
    );
    goToStep(2);

    expect(screen.queryByRole("button", { name: "Choose a different folder…" })).toBeNull();
    expect(screen.getByText(/fixed by/)).toBeDefined();
  });

  it("gives the phone something to point a camera at", async () => {
    // The two-minute promise ends here. It used to tell you to run `kt url` in
    // a terminal, which is the instruction this whole flow exists to avoid.
    render(<Onboarding apps={[welcomeApp]} status={status()} onDone={() => {}} />);
    goToStep(4);

    expect(screen.getByLabelText("QR code for http://welcome.local")).toBeDefined();
    expect(screen.getByText("http://welcome.local")).toBeDefined();
    expect(screen.queryByText(/kt url/)).toBeNull();
  });

  it("answers a real device on the pairing step, not a drawn one", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string, args?: unknown) => {
      const method = (args as { method?: string } | undefined)?.method;
      if (cmd === "kt_call" && method === "device.list") {
        return Promise.resolve([
          {
            id: "dev-1",
            name: "iPhone",
            status: "pending",
            named_by: "guess",
            user_agent: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0) Safari/605.1",
            fingerprint: "f3:9a:21",
            first_seen: 1_760_000_000,
            last_seen: 1_760_000_000,
          },
        ]);
      }
      return Promise.resolve([]);
    }) as typeof invoke);

    render(<Onboarding apps={[welcomeApp]} status={status()} onDone={() => {}} />);
    goToStep(5);

    await waitFor(() => expect(screen.getByText(/iPhone wants to open/)).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Approve & pair" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "kt_call",
        expect.objectContaining({ method: "device.approve" }),
      ),
    );
  });

  it("says it is waiting rather than drawing a device that does not exist", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    render(<Onboarding apps={[welcomeApp]} status={status()} onDone={() => {}} />);
    goToStep(5);

    await waitFor(() => expect(screen.getByText("Waiting for a device")).toBeDefined());
    expect(screen.queryByRole("button", { name: "Approve & pair" })).toBeNull();
  });

  it("offers the launch-at-login choice where the flow promises it", async () => {
    render(<Onboarding apps={[]} status={status()} onDone={() => {}} />);
    goToStep(6);

    // The step promises that closing the window keeps serving; across a
    // restart that is only true with this on.
    expect(
      screen.getByRole("switch", { name: "Start Kitchen Table when I log in" }),
    ).toBeDefined();
  });

  it("shows the real workspace path, not a placeholder", () => {
    render(<Onboarding apps={[]} status={status()} onDone={() => {}} />);
    goToStep(2);
    expect(screen.getByText("/Users/ada/KitchenTable")).toBeDefined();
  });

  it("shows the real first app and its URL", () => {
    render(<Onboarding apps={[welcomeApp]} status={status()} onDone={() => {}} />);
    goToStep(4);

    expect(screen.getByRole("link", { name: "http://welcome.local" })).toBeDefined();
    expect(screen.getByText(/192.168.0.5/)).toBeDefined();
  });

  it("says nothing is shared until you say so", () => {
    render(<Onboarding apps={[welcomeApp]} status={status()} onDone={() => {}} />);
    goToStep(5);
    expect(screen.getByText(/Every app starts Private/)).toBeDefined();
  });

  it("lets the optional steps be skipped but not the required ones", () => {
    render(<Onboarding apps={[]} status={status()} onDone={() => {}} />);

    goToStep(4);
    expect(screen.queryByRole("button", { name: "Skip" })).toBeNull();

    goToStep(7);
    expect(screen.getByRole("button", { name: "Skip" })).toBeDefined();
  });

  it("can go back", () => {
    render(<Onboarding apps={[]} status={status()} onDone={() => {}} />);
    goToStep(3);

    fireEvent.click(screen.getByRole("button", { name: "← Back" }));
    expect(screen.getByText("Step 2 of 8")).toBeDefined();
  });

  it("is not marked done until it is finished", () => {
    render(<Onboarding apps={[]} status={status()} onDone={() => {}} />);
    goToStep(5);
    expect(isComplete()).toBe(false);
  });
});
