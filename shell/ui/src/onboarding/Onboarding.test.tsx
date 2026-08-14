import { fireEvent, render, screen } from "@testing-library/react";
import { Onboarding } from "./Onboarding";
import { isComplete, reset } from "./steps";
import type { App, SysStatus } from "../generated";

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
