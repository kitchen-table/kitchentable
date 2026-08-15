import { invoke } from "@tauri-apps/api/core";
import { vi } from "vitest";
import { hasTauri, openInstead } from "./external";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const call = vi.mocked(invoke);

/** Pretend there is a Tauri host, the way the real one announces itself. */
function withTauri(present: boolean) {
  if (present) {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  } else {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  }
}

describe("opening a URL outside the window", () => {
  beforeEach(() => {
    call.mockReset();
    call.mockResolvedValue(undefined);
    withTauri(false);
  });

  afterEach(() => withTauri(false));

  it("hands the URL to the OS in the desktop app", async () => {
    // The bug: a webview has no second tab, so `target="_blank"` alone is
    // swallowed and the Open button did nothing at all.
    withTauri(true);
    const event = { preventDefault: vi.fn() };
    openInstead("http://localhost/trip-planner/")(event);

    expect(event.preventDefault).toHaveBeenCalled();
    await vi.waitFor(() =>
      expect(call).toHaveBeenCalledWith("plugin:opener|open_url", {
        url: "http://localhost/trip-planner/",
      }),
    );
  });

  it("leaves the click alone in a plain browser", () => {
    // The same UI runs under `pnpm -C shell/ui dev` and under vitest, where an
    // anchor already opens a tab correctly. Taking the click there would break
    // something that works.
    const event = { preventDefault: vi.fn() };
    openInstead("http://localhost/trip-planner/")(event);

    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(call).not.toHaveBeenCalled();
  });

  it("reports a refusal rather than failing silently", async () => {
    // Silence is the failure mode being fixed; swapping one silent failure for
    // another would be no improvement.
    withTauri(true);
    call.mockRejectedValue({ message: "not allowed" });
    const onError = vi.fn();
    openInstead("http://localhost/x/", onError)({ preventDefault: vi.fn() });

    await vi.waitFor(() => expect(onError).toHaveBeenCalledWith({ message: "not allowed" }));
  });

  it("survives an IPC that throws rather than rejecting", async () => {
    // `invoke` reaches for the IPC synchronously - the failure that took the
    // whole window down in the events module. Here it happens inside an async
    // function, so it arrives as a rejection, and the click must not throw
    // back into React either way.
    withTauri(true);
    call.mockImplementation(() => {
      throw new Error("no ipc");
    });
    const onError = vi.fn();

    expect(() =>
      openInstead("http://localhost/x/", onError)({ preventDefault: vi.fn() }),
    ).not.toThrow();
    await vi.waitFor(() => expect(onError).toHaveBeenCalled());
  });

  it("knows whether it has a host to ask", () => {
    expect(hasTauri()).toBe(false);
    withTauri(true);
    expect(hasTauri()).toBe(true);
  });
});
