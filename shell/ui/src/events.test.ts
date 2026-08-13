import { QueryClient } from "@tanstack/react-query";
import { vi } from "vitest";
import type { Event } from "./generated";
import { invalidate } from "./events";

/** The query keys each event should make stale, and nothing else. */
function stale(event: Event): string[][] {
  const client = new QueryClient();
  const spy = vi.spyOn(client, "invalidateQueries");
  invalidate(client, event);
  return spy.mock.calls.map((call) => (call[0]?.queryKey ?? []) as string[]);
}

describe("daemon events", () => {
  it("reloads the library when an app appears or leaves", () => {
    expect(stale({ event: "app_removed", slug: "trip-planner" })).toEqual([["apps"]]);
  });

  it("reloads devices the moment somebody asks to be let in", () => {
    // This is the event the whole subscription exists for: before it, a phone
    // on a wait page could sit there for up to two seconds after the daemon
    // already knew about it.
    expect(stale({ event: "pairing_request", device_id: "d1" })).toEqual([["devices"]]);
  });

  it("reloads the log on access, and nothing else", () => {
    // Opens are the noisiest event there is. Refetching the library on every
    // one would make a busy household re-render constantly for no new
    // information.
    expect(stale({ event: "access", action: "opened", app_slug: "trip-planner" })).toEqual([
      ["log"],
    ]);
  });

  it("says nothing is stale when the daemon reports a problem", () => {
    // An error is for the owner to read, not a cue to refetch. Guessing which
    // view it belongs to would be worse than leaving it in the log.
    expect(stale({ event: "error", message: "the keychain is locked" })).toEqual([]);
  });
});
