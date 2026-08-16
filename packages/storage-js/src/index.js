/**
 * The storage client an app's own JavaScript calls.
 *
 * Served by the daemon at `/__kt/storage.js`, which attaches `window.kt`:
 *
 *     const store = kt.storage();
 *     await store.set("items", ["passports"]);
 *     await store.get("items");
 *
 * **Where those bytes end up is the owner's choice, not the app's.** The app
 * writes the same four calls either way and never has to care, which is the
 * point: an owner decides in the Storage tab whether a trip checklist is one
 * shared list or a private copy per phone, and no app has to be rewritten for
 * that to change.
 *
 * The three shapes, decided once from `/__kt/storage.json`:
 *
 *   - **Synced.** Reads and writes go to the daemon's shared store, so an edit
 *     on a phone shows up on a laptop.
 *   - **Separate on each device.** The device's own `localStorage` is the real
 *     store - which is what makes it work with the Mac asleep, off the network,
 *     or shut - and each write is copied to the daemon afterwards when the
 *     owner has backup on. The copy is a copy: if it fails, the write already
 *     succeeded and the app is not told.
 *   - **No Kitchen Table at all.** `localStorage`, same as above without the
 *     copy. This is what makes an app written for Kitchen Table still work
 *     from a plain web server or a `file://` URL.
 */

const BASE = "/__kt/storage";
const CONFIG = "/__kt/storage.json";

/**
 * A write carries this so a cross-site request cannot forge one: setting a
 * custom header needs a CORS preflight, and the daemon answers none.
 */
const WRITE_HEADER = "x-kitchen-table";

/**
 * What kind of store this is, decided once and remembered.
 *
 * `null` until something has been read or written. Resolved lazily rather than
 * at load, because every method here is already async and a probe on load would
 * be a request every app makes whether or not it stores anything.
 */
let settled = null;

/** Let a test start again. Not part of the app-facing API. */
export function forget() {
  settled = null;
}

/** The shape used when Kitchen Table is not keeping this app's data. */
const LOCAL_ONLY = { mode: "local", backup: false, device: false };

async function config() {
  if (settled) return settled;
  let response;
  try {
    response = await fetch(CONFIG, { credentials: "same-origin" });
  } catch {
    // No daemon at all: the app is hosted somewhere else.
    settled = LOCAL_ONLY;
    return settled;
  }
  if (!response.ok) {
    // 404 is a daemon serving no storage; 400 is the `<ip>/<slug>/` route,
    // where every app shares an origin and the daemon will not pretend it can
    // keep them apart. A 403 is the gate refusing this viewer, and is not a
    // reason to hand them a private local copy of an app they cannot open -
    // but there is nothing useful to throw from here, so it reads as local and
    // the first real request surfaces the refusal.
    settled = LOCAL_ONLY;
    return settled;
  }
  settled = await response.json();
  return settled;
}

/** Whether this app's data is kept by Kitchen Table, and how. */
export async function where() {
  return config();
}

/**
 * Where a local copy keeps its keys.
 *
 * Namespaced by more than the origin because the fallback route puts every app
 * on one. `packing-list` and `chores-rota` reached at `<ip>/<slug>/` would
 * otherwise share a `localStorage` too, which is the same collision the daemon
 * refuses to paper over - so this does not paper over it either.
 */
function namespace() {
  const segment = location.pathname.split("/").filter(Boolean)[0] ?? "";
  return `kt/${location.hostname}/${segment}`;
}

function localKey(personal, key) {
  return `${namespace()}/${personal ? "personal" : "app"}/${key}`;
}

const local = {
  get(personal, key) {
    const raw = localStorage.getItem(localKey(personal, key));
    return raw === null ? undefined : JSON.parse(raw);
  },
  set(personal, key, value) {
    localStorage.setItem(localKey(personal, key), JSON.stringify(value));
  },
  delete(personal, key) {
    localStorage.removeItem(localKey(personal, key));
  },
  list(personal, prefix) {
    const head = localKey(personal, prefix ?? "");
    const cut = localKey(personal, "").length;
    const found = [];
    for (let i = 0; i < localStorage.length; i += 1) {
      const full = localStorage.key(i);
      if (full === null || !full.startsWith(head)) continue;
      found.push({
        key: full.slice(cut),
        value: JSON.parse(localStorage.getItem(full) ?? "null"),
      });
    }
    return found.sort((a, b) => (a.key < b.key ? -1 : 1));
  },
};

async function request(method, path, body) {
  const headers = {};
  if (method !== "GET") headers[WRITE_HEADER] = "1";
  return fetch(path, { method, headers, body, credentials: "same-origin" });
}

function query(personal, extra) {
  const parts = [];
  if (personal) parts.push("personal=1");
  if (extra) parts.push(extra);
  return parts.length ? `?${parts.join("&")}` : "";
}

async function daemonGet(personal, key) {
  const response = await request("GET", `${BASE}/${encodeURIComponent(key)}${query(personal)}`);
  if (response.status === 404) return undefined;
  if (!response.ok) throw new Error(await response.text());
  return response.json();
}

async function daemonSet(personal, key, value) {
  const response = await request(
    "PUT",
    `${BASE}/${encodeURIComponent(key)}${query(personal)}`,
    JSON.stringify(value),
  );
  if (!response.ok) throw new Error(await response.text());
}

async function daemonDelete(personal, key) {
  const response = await request("DELETE", `${BASE}/${encodeURIComponent(key)}${query(personal)}`);
  if (!response.ok) throw new Error(await response.text());
}

async function daemonList(personal, prefix) {
  const response = await request(
    "GET",
    `${BASE}${query(personal, prefix ? `prefix=${encodeURIComponent(prefix)}` : "")}`,
  );
  if (!response.ok) throw new Error(await response.text());
  const entries = await response.json();
  return entries.map((entry) => ({ key: entry.key, value: JSON.parse(entry.value) }));
}

/**
 * Send the Mac a copy of what this device just did.
 *
 * Never throws and never blocks the answer the app is waiting on. The write it
 * is copying has already succeeded on the device, which is the store; this is
 * the spare key under the mat, and failing to cut one is not a reason to say
 * the door did not lock.
 */
async function copyToMac(settings, action) {
  if (!settings.backup || !settings.device) return;
  try {
    await action();
  } catch {
    // Deliberately silent to the app. The owner sees the device's last-seen
    // copy in the Storage tab, which is where a missing one is legible.
  }
}

/**
 * A view of one app's store.
 *
 * `personal: true` asks for this device's own corner of a *shared* app - the
 * private note inside a group trip planner. Under "separate on each device"
 * everything is already private to the device, so it changes nothing.
 */
export function storage(options = {}) {
  const personal = options.personal === true;

  return {
    async get(key) {
      const settings = await config();
      if (settings.mode === "synced") return daemonGet(personal, key);
      return local.get(personal, key);
    },

    async set(key, value) {
      const settings = await config();
      if (settings.mode === "synced") return daemonSet(personal, key, value);

      local.set(personal, key, value);
      await copyToMac(settings, () => daemonSet(true, key, value));
    },

    async delete(key) {
      const settings = await config();
      if (settings.mode === "synced") return daemonDelete(personal, key);

      local.delete(personal, key);
      await copyToMac(settings, () => daemonDelete(true, key));
    },

    async list(prefix = "") {
      const settings = await config();
      if (settings.mode === "synced") return daemonList(personal, prefix);
      return local.list(personal, prefix);
    },
  };
}

/**
 * Whether an edit here will reach this app's other devices.
 *
 * `null` until something has been read or written, because that is the first
 * moment anybody knows. An app that wants to say "changes stay on this device"
 * can await [`where`] instead and get the whole answer.
 */
export function syncing() {
  return settled === null ? null : settled.mode === "synced";
}

// Served as a plain script, so the app reaches this as a global rather than an
// import. Guarded because the same file is imported directly by its tests.
if (typeof window !== "undefined") {
  window.kt = { storage, syncing, where };
}
