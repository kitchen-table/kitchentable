import { render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { beforeEach, expect, it, vi } from "vitest";
import { App } from "./App";
import { reset } from "./onboarding/steps";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
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
    setSize: () => Promise.resolve(),
    setMinSize: () => Promise.resolve(),
    setResizable: () => Promise.resolve(),
    center: () => Promise.resolve(),
  }),
}));

const call = vi.mocked(invoke);

beforeEach(() => {
  call.mockReset();
  localStorage.clear();
  reset();
});

/**
 * First run, in the shape it actually happens: the daemon has a database to
 * create and keys to mint, so the window's first health check loses the race.
 */
function daemonUpAfter(failures: number) {
  let asked = 0;
  call.mockImplementation(((cmd: string) => {
    if (cmd === "kt_health") {
      asked += 1;
      return asked > failures
        ? Promise.resolve({ state: "ready", we_started_it: true })
        : Promise.resolve({ state: "not_running" });
    }
    if (cmd === "kt_notify_state") {
      return Promise.resolve({ state: "unbundled", alerts: false });
    }
    return Promise.resolve([]);
  }) as typeof invoke);
}

it("waits rather than announcing a failure while the daemon is still starting", async () => {
  // The bug this pins: on a clean machine the very first thing anybody saw was
  // "the background service is not running", and it stayed for a full minute
  // because nothing asked again.
  daemonUpAfter(2);
  render(<App />);

  await waitFor(() => expect(screen.getByText("Starting…")).toBeDefined());
  expect(screen.queryByText(/not running/)).toBeNull();

  // And it gets there on its own, without anybody pressing anything.
  await waitFor(() => expect(screen.getByText(/Welcome to Kitchen Table/)).toBeDefined(), {
    timeout: 5_000,
  });
});

it("still says so plainly when the daemon really is not there", async () => {
  call.mockImplementation(((cmd: string) =>
    cmd === "kt_health"
      ? Promise.resolve({ state: "not_running" })
      : Promise.resolve([])) as typeof invoke);

  render(<App />);

  // The grace period is for a daemon on its way up, not cover for one that
  // never arrives.
  await waitFor(
    () => expect(screen.getByText(/is not running/)).toBeDefined(),
    { timeout: 12_000 },
  );
}, 15_000);
