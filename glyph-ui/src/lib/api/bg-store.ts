// Background pictures in the browser (web player and demo): the same three
// commands the app has (glyph_app::backgrounds), kept in IndexedDB. Nothing
// leaves the browser. Without IndexedDB (private windows) they live until
// the page closes.

export interface BgItem { id: string; url: string; plugin: string | null }

const MAX_BYTES = 12 * 1024 * 1024;
const MAX_COUNT = 40;

let dbp: Promise<IDBDatabase | null> | null = null;
function db(): Promise<IDBDatabase | null> {
  if (dbp) return dbp;
  dbp = new Promise((resolve) => {
    try {
      if (typeof indexedDB === "undefined") return resolve(null);
      const req = indexedDB.open("glyph-backgrounds", 1);
      req.onupgradeneeded = () => { req.result.createObjectStore("pics"); };
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => resolve(null);
      req.onblocked = () => resolve(null);
    } catch {
      resolve(null);
    }
  });
  return dbp;
}

/** Pictures by id, newest last; mirrors IndexedDB once read. */
const mem = new Map<string, Blob>();
const urls = new Map<string, string>();
let loaded = false;

async function loadAll() {
  if (loaded) return;
  loaded = true;
  const d = await db();
  if (!d) return;
  await new Promise<void>((resolve) => {
    try {
      const st = d.transaction("pics", "readonly").objectStore("pics");
      const req = st.openCursor();
      req.onsuccess = () => {
        const c = req.result;
        if (!c) return resolve();
        if (c.value instanceof Blob && typeof c.key === "string") mem.set(c.key, c.value);
        c.continue();
      };
      req.onerror = () => resolve();
    } catch {
      resolve();
    }
  });
}

function urlOf(id: string): string {
  let u = urls.get(id);
  if (!u) { u = URL.createObjectURL(mem.get(id)!); urls.set(id, u); }
  return u;
}

function typeOf(b: Uint8Array): string | null {
  if (b[0] === 0x89 && b[1] === 0x50 && b[2] === 0x4e && b[3] === 0x47) return "image/png";
  if (b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff) return "image/jpeg";
  if (b[0] === 0x52 && b[1] === 0x49 && b[2] === 0x46 && b[3] === 0x46 && b[8] === 0x57 && b[9] === 0x45 && b[10] === 0x42 && b[11] === 0x50) return "image/webp";
  return null;
}

export async function bgList(): Promise<{ items: BgItem[] }> {
  await loadAll();
  return { items: [...mem.keys()].reverse().map((id) => ({ id, url: urlOf(id), plugin: null })) };
}

export async function bgAdd(data: string): Promise<string> {
  await loadAll();
  const bin = atob(data);
  if (bin.length > MAX_BYTES) throw new Error("The picture is too large.");
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  const type = typeOf(bytes);
  if (!type) throw new Error("Only PNG, JPEG and WebP pictures work.");
  const hash = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  const id = `own:${[...hash.slice(0, 8)].map((x) => x.toString(16).padStart(2, "0")).join("")}`;
  if (mem.has(id)) return id;
  if (mem.size >= MAX_COUNT) throw new Error(`At most ${MAX_COUNT} pictures; remove one first.`);
  const blob = new Blob([bytes], { type });
  mem.set(id, blob);
  const d = await db();
  try { d?.transaction("pics", "readwrite").objectStore("pics").put(blob, id); } catch { /* kept in memory */ }
  return id;
}

export async function bgRemove(id: string): Promise<void> {
  await loadAll();
  mem.delete(id);
  const u = urls.get(id);
  if (u) { URL.revokeObjectURL(u); urls.delete(id); }
  const d = await db();
  try { d?.transaction("pics", "readwrite").objectStore("pics").delete(id); } catch { /* ignore */ }
}
