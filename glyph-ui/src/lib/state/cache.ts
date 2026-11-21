// Local caches, persisted UI sizing and snapshot retry helpers used by App state.
import type { Backend } from "../api/backend";
import { trackKey, type HomeFeeds, type Snapshot, type Track } from "../api/types";

export function pruneTrackedKeys(
  asked: { size: number; keys(): IterableIterator<string>; delete(key: string): boolean },
  dict: Record<string, unknown>,
  limit: number,
) {
  while (asked.size > limit) {
    const oldest = asked.keys().next().value;
    if (oldest === undefined) break;
    asked.delete(oldest);
    delete dict[oldest];
  }
}

export function loadWidth(key: string, def: number, min: number, max: number): number {
  try {
    const raw = typeof localStorage !== "undefined" ? localStorage.getItem(key) : null;
    const n = raw == null ? NaN : Number(raw);
    return Number.isFinite(n) ? Math.round(Math.max(min, Math.min(max, n))) : def;
  } catch {
    return def;
  }
}

export function saveWidth(key: string, w: number) {
  try {
    if (typeof localStorage !== "undefined") localStorage.setItem(key, String(w));
  } catch { /* ignore */ }
}

export async function fetchInitialSnapshot(api: Backend): Promise<Snapshot> {
  if (api.kind !== "tauri") return api.snapshot();
  return new Promise<Snapshot>((resolve, reject) => {
    let settled = false;
    let activeMs = 0;
    let retried = false;
    let inFlight = 0;
    let lastTick = performance.now();
    const finish = (fn: () => void) => {
      if (settled) return;
      settled = true;
      clearInterval(timer);
      if (typeof document !== "undefined") {
        document.removeEventListener("visibilitychange", syncClock);
      }
      fn();
    };
    const launch = () => {
      inFlight++;
      api.snapshot().then(
        (snap) => {
          inFlight--;
          finish(() => resolve(snap));
        },
        (err) => {
          inFlight--;
          if (settled) return;
          if (!retried) {
            retried = true;
            launch();
            return;
          }
          if (inFlight === 0) finish(() => reject(err));
        },
      );
    };
    const syncClock = () => {
      const now = performance.now();
      if (typeof document === "undefined" || !document.hidden) {
        activeMs += Math.min(500, Math.max(0, now - lastTick));
      }
      lastTick = now;
      if (activeMs >= 3500 && !retried) {
        retried = true;
        launch();
      }
      if (activeMs >= 10000) {
        finish(() => reject(new Error("Timed out waiting for app snapshot")));
      }
    };
    const timer = setInterval(syncClock, 250);
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", syncClock);
    }
    launch();
  });
}

// ---- artist photos (sidebar, tiles), kept across restarts ----
const PHOTO_CACHE_KEY = "glyph.artist-photos.v1";

export function loadArtistPhotos(): Record<string, string | null> {
  try {
    const raw = typeof localStorage !== "undefined" ? localStorage.getItem(PHOTO_CACHE_KEY) : null;
    const v = raw ? JSON.parse(raw) : null;
    return v && typeof v === "object" ? (v as Record<string, string | null>) : {};
  } catch {
    return {};
  }
}

let photoTimer: ReturnType<typeof setTimeout> | undefined;
export function saveArtistPhotos(snapshot: Record<string, string | null>) {
  clearTimeout(photoTimer);
  photoTimer = setTimeout(() => {
    try {
      if (typeof localStorage === "undefined") return;
      const entries = Object.entries(snapshot).filter(([, u]) => !!u).slice(-300);
      localStorage.setItem(PHOTO_CACHE_KEY, JSON.stringify(Object.fromEntries(entries)));
    } catch { /* ignore */ }
  }, 1500);
}

// ---- Home feeds cache ----
const HOME_CACHE_KEY = "glyph.home-cache.v1";

export function homeCacheLoad(): HomeFeeds | null {
  try {
    const raw = typeof localStorage !== "undefined" ? localStorage.getItem(HOME_CACHE_KEY) : null;
    const h = raw ? (JSON.parse(raw) as HomeFeeds) : null;
    return h && Array.isArray(h.recs) && Array.isArray(h.trending) ? { ...h, new_releases: h.new_releases ?? [] } : null;
  } catch {
    return null;
  }
}

let homeCacheTimer: ReturnType<typeof setTimeout> | undefined;
export function homeCacheSave(snap: HomeFeeds) {
  clearTimeout(homeCacheTimer);
  homeCacheTimer = setTimeout(() => {
    try {
      if (typeof localStorage === "undefined") return;
      localStorage.setItem(
        HOME_CACHE_KEY,
        JSON.stringify({ recs: snap.recs.slice(0, 60), trending: snap.trending.slice(0, 60), new_releases: snap.new_releases.slice(0, 40), loading: false }),
      );
    } catch { /* ignore */ }
  }, 1000);
}

// ---- album page cache (songs of albums opened before, kept across restarts) ----
const ALBUM_CACHE_KEY = "glyph.album-cache.v1";
const ALBUM_CACHE_MAX = 80;
let albumCache: Map<string, Track[]> | null = null;
const albumCacheKey = (title: string, artist: string) => `${title.trim().toLowerCase()}|${artist.trim().toLowerCase()}`;

function albumCacheMap(): Map<string, Track[]> {
  if (albumCache) return albumCache;
  albumCache = new Map();
  try {
    const raw = typeof localStorage !== "undefined" ? localStorage.getItem(ALBUM_CACHE_KEY) : null;
    const arr = raw ? (JSON.parse(raw) as [string, Track[]][]) : [];
    for (const [k, v] of arr) if (Array.isArray(v)) albumCache.set(k, v);
  } catch { /* ignore */ }
  return albumCache;
}

export function albumCacheGet(title: string, artist: string): Track[] | null {
  const m = albumCacheMap();
  const k = albumCacheKey(title, artist);
  const v = m.get(k);
  if (!v) return null;
  m.delete(k);
  m.set(k, v);
  return v;
}

let albumCacheTimer: ReturnType<typeof setTimeout> | undefined;
export function albumCachePut(title: string, artist: string, tracks: Track[]) {
  const m = albumCacheMap();
  const k = albumCacheKey(title, artist);
  m.delete(k);
  m.set(k, tracks);
  while (m.size > ALBUM_CACHE_MAX) m.delete(m.keys().next().value as string);
  clearTimeout(albumCacheTimer);
  albumCacheTimer = setTimeout(() => {
    try {
      if (typeof localStorage !== "undefined") localStorage.setItem(ALBUM_CACHE_KEY, JSON.stringify([...m.entries()]));
    } catch { /* full: memory only */ }
  }, 800);
}

export function sameTrackKeys(a: Track[], b: Track[]): boolean {
  return a.length === b.length && a.every((x, i) => trackKey(x) === trackKey(b[i]));
}
