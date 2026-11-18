// What the web player keeps in the browser (localStorage): settings,
// library, queue, a few player choices. Nothing of it leaves the browser.
// Every read and write is guarded: private windows and blocked storage
// just start empty.

const PREFIX = "glyph.web.";

export function load<T>(name: string): T | null {
  try {
    const raw = localStorage.getItem(PREFIX + name);
    return raw ? (JSON.parse(raw) as T) : null;
  } catch {
    return null;
  }
}

/** Big entries (the library) are mirrored into IndexedDB: localStorage only
 *  holds about 5 MB, and a large library that no longer fit there used to be
 *  lost on reload (empty playlists until the next sync). */
const big = (name: string) => name === "library" || name.startsWith("account_lib.");

export function save(name: string, value: unknown) {
  let json: string;
  try {
    json = JSON.stringify(value);
  } catch {
    return;
  }
  try {
    localStorage.setItem(PREFIX + name, json);
  } catch {
    /* storage full or blocked: IndexedDB below keeps big entries */
    if (big(name)) {
      try { localStorage.removeItem(PREFIX + name); } catch { /* ignore */ }
    }
  }
  if (big(name)) void idbPut(name, json);
}

// ---- IndexedDB mirror ----
let dbp: Promise<IDBDatabase | null> | null = null;
function db(): Promise<IDBDatabase | null> {
  if (dbp) return dbp;
  dbp = new Promise((resolve) => {
    try {
      if (typeof indexedDB === "undefined") return resolve(null);
      const req = indexedDB.open("glyph-web", 1);
      req.onupgradeneeded = () => { req.result.createObjectStore("kv"); };
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => resolve(null);
      req.onblocked = () => resolve(null);
    } catch {
      resolve(null);
    }
  });
  return dbp;
}
async function idbPut(name: string, json: string) {
  const d = await db();
  if (!d) return;
  try {
    d.transaction("kv", "readwrite").objectStore("kv").put(json, name);
  } catch { /* ignore */ }
}
/** A big entry from IndexedDB (used when localStorage had none). */
export async function loadBig<T>(name: string): Promise<T | null> {
  // Never hold up startup: a slow or stuck IndexedDB counts as "nothing saved".
  const timeout = new Promise<null>((r) => setTimeout(() => r(null), 1500));
  return Promise.race([readBig<T>(name), timeout]);
}
async function readBig<T>(name: string): Promise<T | null> {
  const d = await db();
  if (!d) return null;
  return new Promise((resolve) => {
    try {
      const req = d.transaction("kv", "readonly").objectStore("kv").get(name);
      req.onsuccess = () => {
        try { resolve(typeof req.result === "string" ? (JSON.parse(req.result) as T) : null); } catch { resolve(null); }
      };
      req.onerror = () => resolve(null);
    } catch {
      resolve(null);
    }
  });
}

export function remove(name: string) {
  try {
    localStorage.removeItem(PREFIX + name);
  } catch {
    /* ignore */
  }
  if (big(name)) {
    void db().then((d) => { try { d?.transaction("kv", "readwrite").objectStore("kv").delete(name); } catch { /* ignore */ } });
  }
}

const timers = new Map<string, number>();

/** Cancel any queued delayed save for `name`. */
export function cancelSoon(name: string) {
  clearTimeout(timers.get(name));
  timers.delete(name);
}

/** Save after a short pause, so many changes in a row write once. */
export function saveSoon(name: string, value: () => unknown, ms = 400) {
  clearTimeout(timers.get(name));
  timers.set(name, window.setTimeout(() => { timers.delete(name); save(name, value()); }, ms));
}

/** Random id of this browser for the server's rate limits (like the app's install id). */
export function installId(): string {
  let id = load<string>("install_id");
  if (!id || !/^[A-Za-z0-9_-]{8,64}$/.test(id)) {
    id = (crypto.randomUUID?.() ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`).replace(/[^A-Za-z0-9_-]/g, "");
    save("install_id", id);
  }
  return id;
}
