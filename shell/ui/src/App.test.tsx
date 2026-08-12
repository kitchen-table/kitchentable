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
});
