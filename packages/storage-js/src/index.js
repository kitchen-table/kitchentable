/**
 * The storage client an app's own JavaScript calls.
 *
 * Served by the daemon at `/__kt/storage.js`, which attaches `window.kt`. An
 * app then does:
 *
 *     const store = kt.storage();                  // shared with everyone
 *     const mine  = kt.storage({ personal: true }); // this device only
 *     await store.set("items", ["passports"]);
 *     await store.get("items");
 *
 * **It degrades to `localStorage` rather than failing.** This is the whole
 * reason an app written for Kitchen Table is not trapped in it: the same file
 * opened from a plain web server, or from `file://`, still works - it just
 * remembers on that one device instead of across the house. Three things send
 * it down that path, and all three are cases where Kitchen Table has said, in
 * so many words, that it is not keeping this app's data:
 *
 *   - the daemon is not there at all (the app is hosted somewhere else);
 *   - the daemon is there but serves no storage;
 *   - the app was reached at `<ip>/<slug>/`, where every app shares one origin
 *     so the daemon refuses to pretend it can keep them apart.
 *
 * The last one matters most, because it is the route Android often needs. An
 * app opened that way keeps working and stops syncing, which is a far better
 * failure than a checklist that throws.
 */

const BASE = "/__kt/storage";

/**
 * A write carries this so a cross-site request cannot forge one: setting a
 * custom header needs a CORS preflight, and the daemon answers none.
 */
const WRITE_HEADER = "x-kitchen-table";

/**
 * Which backend is answering, decided once and remembered.
 *
 * `null` means nobody has asked yet. Resolved lazily rather than probed at
 * load, because every method here is already async and a probe on load would
 * be a request every app makes whether or not it stores anything.
 */
let backend = null;

/** Let a test start again. Not part of the app-facing API. */
export function forget() {
  backend = null;
}

/**
 * Where a local fallback keeps its keys.
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

/** The local half, used when Kitchen Table is not keeping this app's data. */
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
    const found = [];
    for (let i = 0; i < localStorage.length; i += 1) {
      const full = localStorage.key(i);
      if (full === null || !full.startsWith(head)) continue;
      found.push({
        key: full.slice(localKey(personal, "").length),
        value: JSON.parse(localStorage.getItem(full) ?? "null"),
      });
    }
    return found.sort((a, b) => (a.key < b.key ? -1 : 1));
  },
};

/**
 * Whether a response means "Kitchen Table is not keeping this app's data".
 *
 * Deliberately narrow. A 403 is the gate saying this viewer may not have it,
 * and falling back there would quietly hand somebody a private copy of an app
 * they were refused - so a refusal is a refusal and is thrown.
 */
function meansNoBackend(status) {
  // 404: this daemon serves no storage. 400: the app was reached on a route
  // where its data cannot be kept apart from its neighbours'.
  return status === 404 || status === 400;
}

async function request(method, path, body) {
  const headers = {};
  if (method !== "GET") headers[WRITE_HEADER] = "1";
  return fetch(path, {
    method,
    headers,
    body,
    // The session cookie is the whole authorisation, and a cross-origin fetch
    // would not send it.
    credentials: "same-origin",
  });
}

/**
 * Decide once whether the daemon is answering, using the caller's own first
 * request rather than a separate probe.
 */
function remember(kind) {
  if (backend === null) backend = kind;
  return backend;
}

function query(personal, extra) {
  const parts = [];
  if (personal) parts.push("personal=1");
  if (extra) parts.push(extra);
  return parts.length ? `?${parts.join("&")}` : "";
}

/**
 * A view of one app's store.
 *
 * `personal: true` is this device's own, which needs a paired device - on an
 * app that pairs nobody the daemon refuses rather than quietly sharing it, and
 * that refusal reaches the caller.
 */
export function storage(options = {}) {
  const personal = options.personal === true;

  return {
    async get(key) {
      if (backend === "local") return local.get(personal, key);
      let response;
      try {
        response = await request("GET", `${BASE}/${encodeURIComponent(key)}${query(personal)}`);
      } catch {
        remember("local");
        return local.get(personal, key);
      }
      if (meansNoBackend(response.status)) {
        remember("local");
        return local.get(personal, key);
      }
      remember("kt");
      // A key that was never set. Distinct from a stored `null`, which is why
      // the daemon answers 404 rather than an empty body.
      if (response.status === 404) return undefined;
      if (!response.ok) throw new Error(await response.text());
      return response.json();
    },

    async set(key, value) {
      const body = JSON.stringify(value);
      if (backend === "local") return local.set(personal, key, value);
      let response;
      try {
        response = await request(
          "PUT",
          `${BASE}/${encodeURIComponent(key)}${query(personal)}`,
          body,
        );
      } catch {
        remember("local");
        return local.set(personal, key, value);
      }
      if (meansNoBackend(response.status)) {
        remember("local");
        return local.set(personal, key, value);
      }
      remember("kt");
      if (!response.ok) throw new Error(await response.text());
    },

    async delete(key) {
      if (backend === "local") return local.delete(personal, key);
      let response;
      try {
        response = await request("DELETE", `${BASE}/${encodeURIComponent(key)}${query(personal)}`);
      } catch {
        remember("local");
        return local.delete(personal, key);
      }
      if (meansNoBackend(response.status)) {
        remember("local");
        return local.delete(personal, key);
      }
      remember("kt");
      if (!response.ok) throw new Error(await response.text());
    },

    async list(prefix = "") {
      if (backend === "local") return local.list(personal, prefix);
      let response;
      try {
        response = await request(
          "GET",
          `${BASE}${query(personal, prefix ? `prefix=${encodeURIComponent(prefix)}` : "")}`,
        );
      } catch {
        remember("local");
        return local.list(personal, prefix);
      }
      if (meansNoBackend(response.status)) {
        remember("local");
        return local.list(personal, prefix);
      }
      remember("kt");
      if (!response.ok) throw new Error(await response.text());
      const entries = await response.json();
      return entries.map((entry) => ({ key: entry.key, value: JSON.parse(entry.value) }));
    },
  };
}

/**
 * Whether this app's data is being kept by Kitchen Table.
 *
 * `null` until something has been read or written, because that is the first
 * moment anybody knows. An app that wants to say "changes are only on this
 * device" can read it after its first load.
 */
export function syncing() {
  return backend === null ? null : backend === "kt";
}

// Served as a plain script, so the app reaches this as a global rather than an
// import. Guarded because the same file is imported directly by its tests.
if (typeof window !== "undefined") {
  window.kt = { storage, syncing };
}
