import { render, screen } from "@testing-library/react";
import { StatusBar } from "./StatusBar";
import type { SysStatus } from "./generated";

function status(over: Partial<SysStatus> = {}): SysStatus {
  return {
    protocol_version: 1,
    daemon_version: "0.1.0",
    workspace: "/ws",
    serving: { state: "serving" },
    app_count: 2,
    install_key: null,
    uptime_secs: 42,
    ...over,
  };
}

describe("StatusBar", () => {
  it("says it is serving when it is", () => {
    render(<StatusBar status={status()} />);
    expect(screen.getByText("Serving")).toBeDefined();
  });

  it("gives a degraded state a headline and the daemon's own explanation", () => {
    render(
      <StatusBar
        status={status({
          serving: {
            state: "degraded",
            reason: "local_network_denied",
            message: "Devices on your network can't see your apps yet.",
          },
        })}
      />,
    );

    expect(screen.getByText("Not visible on your network")).toBeDefined();
    expect(
      screen.getByText("Devices on your network can't see your apps yet."),
    ).toBeDefined();
  });

  it("names each degraded reason distinctly", () => {
    // A generic warning leaves someone with nothing to do; onboarding.md
    // requires every denial to have a visible recovery path.
    const reasons = [
      ["port_fallback", "Serving on a different port"],
      ["https_unavailable", "Not encrypted"],
      ["mdns_unavailable", "No .local names"],
    ] as const;

    for (const [reason, headline] of reasons) {
      const { unmount } = render(
        <StatusBar
          status={status({ serving: { state: "degraded", reason, message: "why" } })}
        />,
      );
      expect(screen.getByText(headline)).toBeDefined();
      unmount();
    }
  });

  it("renders nothing before the first status arrives", () => {
    const { container } = render(<StatusBar />);
    expect(container.firstChild).toBeNull();
  });
});
