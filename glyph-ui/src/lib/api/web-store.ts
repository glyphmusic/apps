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

export function save(name: string, value: unknown) {
  try {
    localStorage.setItem(PREFIX + name, JSON.stringify(value));
  } catch {
    /* storage full or blocked: keep it in memory only */
  }
}

export function remove(name: string) {
  try {
    localStorage.removeItem(PREFIX + name);
  } catch {
    /* ignore */
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
