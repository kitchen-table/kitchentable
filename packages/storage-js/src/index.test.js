import { beforeEach, describe, expect, it, vi } from "vitest";
import { forget, storage, syncing } from "./index.js";

/** Answer every request with one status and body. */
function answering(status, body = "") {
  return vi.fn(async () => ({
    status,
    ok: status >= 200 && status < 300,
    json: async () => JSON.parse(body || "null"),
    text: async () => body,
  }));
}

beforeEach(() => {
  forget();
  localStorage.clear();
  vi.restoreAllMocks();
});

describe("talking to the daemon", () => {
  it("writes to the app's own store, with the header a forged write cannot carry", async () => {
    const fetch = answering(204);
    vi.stubGlobal("fetch", fetch);

    await storage().set("items", ["passports"]);

    const [path, init] = fetch.mock.calls[0];
    expect(path).toBe("/__kt/storage/items");
    expect(init.method).toBe("PUT");
    expect(init.headers["x-kitchen-table"]).toBe("1");
    expect(init.body).toBe('["passports"]');
    expect(syncing()).toBe(true);
  });

  it("asks for the personal store only when told to", async () => {
    const fetch = answering(204);
    vi.stubGlobal("fetch", fetch);

    await storage().set("a", 1);
    await storage({ personal: true }).set("b", 2);

    expect(fetch.mock.calls[0][0]).toBe("/__kt/storage/a");
    expect(fetch.mock.calls[1][0]).toBe("/__kt/storage/b?personal=1");
  });

  it("tells a key that was never set from one holding null", async () => {
    vi.stubGlobal("fetch", answering(404));
    // 404 is also what "no storage here" looks like, so this asserts the
    // fallback answers rather than the call throwing.
    await expect(storage().get("never")).resolves.toBeUndefined();
  });

  it("passes a refusal through rather than answering from the device", async () => {
    // A 403 is the gate saying this viewer may not have the app. Falling back
    // would hand them a private local copy of something they were refused.
    vi.stubGlobal("fetch", answering(403, "Not available to this device."));

    await expect(storage().get("items")).rejects.toThrow(/Not available/);
    expect(localStorage.length).toBe(0);
  });
});

describe("when Kitchen Table is not keeping this app's data", () => {
  it("falls back to the device when there is no daemon at all", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new TypeError("Failed to fetch");
      }),
    );

    await storage().set("items", ["socks"]);
    expect(await storage().get("items")).toEqual(["socks"]);
    expect(syncing()).toBe(false);
  });

  it("falls back where every app shares one origin", async () => {
    // The `<ip>/<slug>/` route. The daemon refuses because it cannot keep one
    // app's data from another's there, and the app carries on unsynced rather
    // than breaking - which is the route Android most often needs.
    vi.stubGlobal("fetch", answering(400, "Storage needs this app's own address."));

    await storage().set("items", ["socks"]);
    expect(await storage().get("items")).toEqual(["socks"]);
    expect(syncing()).toBe(false);
  });

  it("keeps the shared and personal stores apart on the device too", async () => {
    vi.stubGlobal("fetch", answering(404));

    await storage().set("notes", "ours");
    await storage({ personal: true }).set("notes", "mine");

    expect(await storage().get("notes")).toBe("ours");
    expect(await storage({ personal: true }).get("notes")).toBe("mine");
  });

  it("does not let two apps on one origin share a local store", async () => {
    // Same collision the daemon refuses to paper over, so this does not either.
    vi.stubGlobal("fetch", answering(404));
    const at = (path) => {
      window.history.replaceState({}, "", path);
    };

    at("/packing-list/");
    await storage().set("items", ["ours"]);

    at("/chores-rota/");
    expect(await storage().get("items")).toBeUndefined();
  });

  it("lists by prefix, in key order", async () => {
    vi.stubGlobal("fetch", answering(404));
    const store = storage();

    await store.set("day:2", "b");
    await store.set("day:1", "a");
    await store.set("budget", "c");

    expect(await store.list("day:")).toEqual([
      { key: "day:1", value: "a" },
      { key: "day:2", value: "b" },
    ]);
  });

  it("deletes", async () => {
    vi.stubGlobal("fetch", answering(404));
    const store = storage();

    await store.set("k", 1);
    await store.delete("k");
    expect(await store.get("k")).toBeUndefined();
  });

  it("does not ask twice once it knows", async () => {
    const fetch = answering(404);
    vi.stubGlobal("fetch", fetch);
    const store = storage();

    await store.set("a", 1);
    await store.set("b", 2);
    await store.get("a");

    // Only the first call reaches the network; after that the answer is known.
    expect(fetch).toHaveBeenCalledTimes(1);
  });
});

it("says nothing about syncing until something has been asked", () => {
  expect(syncing()).toBeNull();
});
