import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Backend } from "./backend";
import type { BackendEvents, Snapshot, Track } from "./types";
import { trackKey } from "./types";

const MAX_COVER_CACHE = 800;
const MAX_COVER_ACTIVE = 5;
/** Asked before the backend is up: tried again for this long instead of answering "no cover". */
const OFFLINE_RETRY_MS = 30_000;
/** After this long a slow cover request no longer blocks the others. */
const COVER_SLOT_MS = 6_000;
/** After this long a cover request is tried again later (the core keeps working on it). */
const COVER_GIVE_UP_MS = 20_000;
/** Server said "too many requests" (or was slow): first quiet retry after this, then doubled. */
const COVER_RETRY_MIN_MS = 2_000;
const COVER_RETRY_MAX_MS = 30_000;
/** Retries are spread over this much extra time, so they do not all come back at once. */
const COVER_RETRY_SPREAD_MS = 1_500;
/** A cover that is still not there after this long counts as "no cover". */
const COVER_RETRY_FOR_MS = 5 * 60_000;

type CoverItem = {
  t: Track;
  k: string;
  signal?: AbortSignal;
  resolve: (v: string | null) => void;
  since: number;
  tries: number;
  done: boolean;
};
/** One answer of the core: a picture, no picture, or "ask again in `ms`". */
type CoverAnswer = { url: string | null; offline?: boolean; busyMs?: number };

/** `cover_busy:<ms>` from the core: wait that long before asking again. */
export function coverBusyMs(e: unknown): number | undefined {
  const m = /cover_busy:(\d+)/.exec(String(e));
  return m ? Number(m[1]) : undefined;
}

/** Wait before retry number `tries` (1, 2, ...): doubles, at least what the server asked for. */
export function coverRetryDelay(tries: number, busyMs = 0, spread = Math.random()): number {
  const backoff = Math.min(COVER_RETRY_MAX_MS, COVER_RETRY_MIN_MS * 2 ** Math.max(0, tries - 1));
  return Math.max(backoff, busyMs) + Math.round(spread * COVER_RETRY_SPREAD_MS);
}

export class TauriBackend implements Backend {
  readonly kind = "tauri" as const;
  private covers = new Map<string, Promise<string | null>>();
  private coverQueue: CoverItem[] = [];
  private coverAsks = new Map<string, Promise<CoverAnswer>>();
  private coverActive = 0;

  snapshot(): Promise<Snapshot> {
    return invoke<Snapshot>("snapshot");
  }

  /** Commands the Rust side does not have yet (docs/TAURI.md) answer
   *  nothing instead of breaking the page, like the demo backend. */
  call<T = void>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
    return invoke<T>(cmd, args).catch((e: unknown) => {
      if (/command .* not found/i.test(String(e))) {
        console.warn("[tauri] not ported yet:", cmd);
        return undefined as T;
      }
      throw e;
    });
  }

  on<K extends keyof BackendEvents>(ev: K, cb: (payload: BackendEvents[K]) => void): () => void {
    let off: (() => void) | null = null;
    let dead = false;
    listen<BackendEvents[K]>(`glyph://${ev}`, (e) => cb(e.payload)).then((u) => (dead ? u() : (off = u)));
    return () => { dead = true; off?.(); };
  }

  /** The core caches covers on disk; the webview loads them through Tauri's asset protocol. */
  cover(t: Track, signal?: AbortSignal): Promise<string | null> {
    if (signal?.aborted) return Promise.resolve(null);
    const k = trackKey(t);
    const cached = this.covers.get(k);
    if (cached) {
      // Touch for LRU order.
      this.covers.delete(k);
      this.covers.set(k, cached);
      return cached;
    }
    return new Promise<string | null>((resolve) => {
      const item: CoverItem = { t, k, signal, since: performance.now(), tries: 0, done: false, resolve: (v) => {
        if (item.done) return;
        item.done = true;
        resolve(v);
      } };
      // Scrolled away: stop waiting (also while a quiet retry is pending).
      signal?.addEventListener("abort", () => item.resolve(null), { once: true });
      // Covers already on this device (downloaded songs, local files, the
      // cover cache) are shown right away and never wait behind the network.
      invoke<string | null>("cover_local", { track: t })
        .then((path) => (path ? convertFileSrc(path) : null))
        .catch(() => null)
        .then((local) => {
          if (item.done) return;
          if (local) {
            this.remember(k, Promise.resolve(local));
            item.resolve(local);
            return;
          }
          this.coverQueue.push(item);
          this.drainCovers();
        });
    });
  }

  private remember(k: string, p: Promise<string | null>) {
    this.covers.delete(k);
    if (this.covers.size >= MAX_COVER_CACHE) {
      const oldest = this.covers.keys().next().value;
      if (oldest !== undefined) this.covers.delete(oldest);
    }
    this.covers.set(k, p);
  }

  private drainCovers() {
    while (this.coverActive < MAX_COVER_ACTIVE && this.coverQueue.length > 0) {
      // Newest first: what was just scrolled into view (or the sidebar) is
      // asked before a long backlog of rows that may already be gone.
      const item = this.coverQueue.pop()!;
      if (item.done || item.signal?.aborted) {
        item.resolve(null);
        continue;
      }
      const existing = this.covers.get(item.k);
      if (existing) {
        this.remember(item.k, existing);
        void existing.then(item.resolve);
        continue;
      }
      // Another row with the same song is already asking: share its answer.
      const running = this.coverAsks.get(item.k);
      if (running) {
        void running.then((a) => this.settle(item, a));
        continue;
      }
      this.coverActive++;
      let released = false;
      // A slot is given back after COVER_SLOT_MS even when the answer never
      // comes (a lost bridge reply on Android, a stuck download): before, a
      // few lost answers held every slot and no cover loaded anymore.
      const release = () => {
        if (released) return;
        released = true;
        this.coverActive--;
        this.drainCovers();
      };
      const slot = setTimeout(release, COVER_SLOT_MS);
      const ask: Promise<CoverAnswer> = Promise.race([
        invoke<string | null>("cover_path", { track: item.t })
          .then((path): CoverAnswer => ({ url: path ? convertFileSrc(path) : null }))
          .catch((e): CoverAnswer => {
            if (/backend_offline/.test(String(e))) return { url: null, offline: true };
            const busyMs = coverBusyMs(e);
            return busyMs !== undefined ? { url: null, busyMs } : { url: null };
          }),
        // Slow (the core is queueing behind the server's limit): try again later.
        new Promise<CoverAnswer>((r) => setTimeout(() => r({ url: null, busyMs: 0 }), COVER_GIVE_UP_MS)),
      ]).finally(() => {
        clearTimeout(slot);
        release();
      });
      this.coverAsks.set(item.k, ask);
      void ask.then((a) => {
        if (this.coverAsks.get(item.k) === ask) this.coverAsks.delete(item.k);
        if (a.url) {
          this.remember(item.k, Promise.resolve(a.url));
        } else if (a.offline === undefined && a.busyMs === undefined) {
          // A real "no cover": remembered for a while, then asked again.
          const miss = Promise.resolve(null);
          this.remember(item.k, miss);
          setTimeout(() => { if (this.covers.get(item.k) === miss) this.covers.delete(item.k); }, 15_000);
        }
        this.settle(item, a);
      });
    }
  }

  /** Gives `item` its answer, or puts it back in line for a quiet retry. */
  private settle(item: CoverItem, a: CoverAnswer) {
    if (item.done) return;
    const waited = performance.now() - item.since;
    if (a.offline) {
      // The core was still starting: this is no real miss. Ask again
      // shortly (the sidebar used to keep placeholder icons for good).
      if (waited < OFFLINE_RETRY_MS) {
        setTimeout(() => { if (!item.done) { this.coverQueue.unshift(item); this.drainCovers(); } }, 800);
        return;
      }
    } else if (a.busyMs !== undefined && waited < COVER_RETRY_FOR_MS) {
      // The server asked us to slow down (or answered slowly): keep the row
      // loading and try again in the background, a bit later each time,
      // instead of showing "no cover" for half a minute.
      item.tries++;
      setTimeout(() => {
        if (item.done || item.signal?.aborted) return item.resolve(null);
        this.coverQueue.push(item);
        this.drainCovers();
      }, coverRetryDelay(item.tries, a.busyMs));
      return;
    }
    item.resolve(a.url);
  }

  trimCovers(keep: number): void {
    const cap = Math.max(0, keep);
    while (this.covers.size > cap) {
      const oldest = this.covers.keys().next().value;
      if (oldest === undefined) break;
      this.covers.delete(oldest);
    }
  }
}
