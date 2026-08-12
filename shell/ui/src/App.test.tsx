import { render, screen } from "@testing-library/react";
import { App } from "./App";

describe("App", () => {
  it("renders the product name", () => {
    render(<App />);
    expect(screen.getByRole("heading", { name: "Kitchen Table" })).toBeDefined();
  });

  it("shows the disconnected state while there is no daemon", () => {
    render(<App />);
    expect(screen.getByText("not connected")).toBeDefined();
  });

  it("shows serving state once the daemon reports it", () => {
    render(<App serving={{ state: "serving" }} />);
    expect(screen.getByText("serving")).toBeDefined();
  });

  it("explains a degraded state rather than just saying something is wrong", () => {
    render(
      <App
        serving={{
          state: "degraded",
          reason: "local_network_denied",
          message: "Devices on your network can't see your apps yet.",
        }}
      />,
    );
    expect(screen.getByText("not on your network")).toBeDefined();
  });
});
