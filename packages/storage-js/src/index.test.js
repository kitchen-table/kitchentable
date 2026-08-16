import { beforeEach, describe, expect, it, vi } from "vitest";
import { forget, storage, syncing, where } from "./index.js";

/**
 * A daemon that answers the config endpoint one way and everything else
 * another, recording every call so a test can assert what reached it.
 */
function daemon({ config, storeStatus = 204, body = "" } = {}) {
  const calls = [];
  const fetch = vi.fn(async (path, init = {}) => {
    calls.push({ path, method: init.method ?? "GET", body: init.body, headers: init.headers });
    if (path === "/__kt/storage.json") {
      if (config === "missing") return { ok: false, status: 404 };
      if (config === "offline") throw new TypeError("Failed to fetch");
      return { ok: true, status: 200, json: async () => config };
    }
    return {
      ok: storeStatus >= 200 && storeStatus < 300,
      status: storeStatus,
      json: async () => JSON.parse(body || "null"),
      text: async () => body,
    };
  });
  vi.stubGlobal("fetch", fetch);
  return { fetch, calls, to: (p) => calls.filter((c) => c.path.startsWith(p)) };
}

const SYNCED = { mode: "synced", backup: false, device: true };
const PER_DEVICE = { mode: "per_device", backup: true, device: true };

beforeEach(() => {
  forget();
  localStorage.clear();
  vi.restoreAllMocks();
});

describe("keep in sync everywhere", () => {
  it("reads and writes the daemon's shared store", async () => {
    const { calls } = daemon({ config: SYNCED });

    await storage().set("items", ["passports"]);

    const write = calls.find((c) => c.method === "PUT");
    expect(write.path).toBe("/__kt/storage/items");
    expect(write.body).toBe('["passports"]');
    expect(write.headers["x-kitchen-table"]).toBe("1");
    // Nothing on the device: the daemon is the store.
    expect(localStorage.length).toBe(0);
    expect(syncing()).toBe(true);
  });

  it("asks for a personal corner of a shared app when told to", async () => {
    const { to } = daemon({ config: SYNCED });

    await storage({ personal: true }).set("notes", "mine");

    expect(to("/__kt/storage/notes")[0].path).toContain("personal=1");
  });

  it("passes a refusal through rather than answering from the device", async () => {
    // A 403 is the gate saying this viewer may not have the app. Falling back
    // would hand them a private local copy of something they were refused.
    daemon({ config: SYNCED, storeStatus: 403, body: "Not available to this device." });

    await expect(storage().get("items")).rejects.toThrow(/Not available/);
    expect(localStorage.length).toBe(0);
  });

  it("tells a key that was never set from one holding null", async () => {
    daemon({ config: SYNCED, storeStatus: 404 });
    await expect(storage().get("never")).resolves.toBeUndefined();
  });
});

describe("separate on each device", () => {
  it("keeps the data on the device, so it works with the Mac asleep", async () => {
    const { to } = daemon({ config: PER_DEVICE });
    const store = storage();

    await store.set("workouts", [{ day: "mon" }]);

    // Readable without the daemon answering anything.
    expect(await store.get("workouts")).toEqual([{ day: "mon" }]);
    expect(localStorage.length).toBe(1);
    expect(to("/__kt/storage/workouts").every((c) => c.method !== "GET")).toBe(true);
    expect(syncing()).toBe(false);
  });

  it("copies each write to the Mac when backup is on", async () => {
    const { to } = daemon({ config: PER_DEVICE });

    await storage().set("workouts", [1]);

    const copy = to("/__kt/storage/workouts").find((c) => c.method === "PUT");
    // Under the device's own scope, so one phone's copy is never another's.
    expect(copy.path).toContain("personal=1");
    expect(copy.body).toBe("[1]");
  });

  it("sends nothing to the Mac when backup is off", async () => {
    const { to } = daemon({ config: { mode: "per_device", backup: false, device: true } });

    await storage().set("workouts", [1]);

    expect(to("/__kt/storage/")).toHaveLength(0);
  });

  it("sends nothing when there is no device to attribute a copy to", async () => {
    // A Household app pairs nobody. A copy that cannot be told from anyone
    // else's is worse than no copy: it is somebody's private data in a pile.
    const { to } = daemon({ config: { mode: "per_device", backup: true, device: false } });

    await storage().set("workouts", [1]);

    expect(to("/__kt/storage/")).toHaveLength(0);
  });

  it("does not fail the app's write when the copy fails", async () => {
    // The device is the store and the write already succeeded. A spare key
    // that could not be cut is not a reason to say the door did not lock.
    const { fetch } = daemon({ config: PER_DEVICE, storeStatus: 500, body: "boom" });
    const store = storage();

    await expect(store.set("workouts", [1])).resolves.toBeUndefined();
    expect(await store.get("workouts")).toEqual([1]);
    expect(fetch).toHaveBeenCalled();
  });

  it("deletes on the device and asks the Mac to forget its copy", async () => {
    const { to } = daemon({ config: PER_DEVICE });
    const store = storage();

    await store.set("workouts", [1]);
    await store.delete("workouts");

    expect(await store.get("workouts")).toBeUndefined();
    expect(to("/__kt/storage/workouts").some((c) => c.method === "DELETE")).toBe(true);
  });

  it("lists from the device, by prefix and in key order", async () => {
    daemon({ config: PER_DEVICE });
    const store = storage();

    await store.set("day:2", "b");
    await store.set("day:1", "a");
    await store.set("budget", "c");

    expect(await store.list("day:")).toEqual([
      { key: "day:1", value: "a" },
      { key: "day:2", value: "b" },
    ]);
  });
});

describe("no Kitchen Table at all", () => {
  it("works from a plain web server", async () => {
    daemon({ config: "offline" });
    const store = storage();

    await store.set("items", ["socks"]);
    expect(await store.get("items")).toEqual(["socks"]);
    expect(syncing()).toBe(false);
  });

  it("works where every app shares one origin", async () => {
    // The `<ip>/<slug>/` route, which is the one Android often needs. The
    // daemon refuses because it cannot keep one app's data from another's
    // there, and the app carries on unsynced rather than breaking.
    daemon({ config: "missing" });
    const store = storage();

    await store.set("items", ["socks"]);
    expect(await store.get("items")).toEqual(["socks"]);
  });

  it("does not let two apps on one origin share a local store", async () => {
    daemon({ config: "missing" });
    const at = (path) => window.history.replaceState({}, "", path);

    at("/packing-list/");
    await storage().set("items", ["ours"]);

    at("/chores-rota/");
    expect(await storage().get("items")).toBeUndefined();
  });
});

it("asks what kind of store this is exactly once", async () => {
  const { to } = daemon({ config: SYNCED });
  const store = storage();

  await store.set("a", 1);
  await store.set("b", 2);
  await store.get("a");

  expect(to("/__kt/storage.json")).toHaveLength(1);
});

it("says nothing about syncing until something has been asked", async () => {
  expect(syncing()).toBeNull();
  daemon({ config: SYNCED });
  await where();
  expect(syncing()).toBe(true);
});
