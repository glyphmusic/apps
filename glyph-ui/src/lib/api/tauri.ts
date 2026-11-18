import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Backend } from "./backend";
import type { BackendEvents, Snapshot, Track } from "./types";
import { trackKey } from "./types";

const MAX_COVER_CACHE = 300;
const MAX_COVER_ACTIVE = 5;
/** Asked before the backend is up: tried again for this long instead of answering "no cover". */
const OFFLINE_RETRY_MS = 30_000;
/** After this long a slow cover request no longer blocks the others. */
const COVER_SLOT_MS = 6_000;
/** After this long a cover request counts as "no cover" (asked again later). */
const COVER_GIVE_UP_MS = 20_000;

export class TauriBackend implements Backend {
  readonly kind = "tauri" as const;
  private covers = new Map<string, Promise<string | null>>();
  private coverQueue: { t: Track; k: string; signal?: AbortSignal; resolve: (v: string | null) => void; since: number }[] = [];
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
      this.coverQueue.push({ t, k, signal, resolve, since: performance.now() });
      this.drainCovers();
    });
  }

  private drainCovers() {
    while (this.coverActive < MAX_COVER_ACTIVE && this.coverQueue.length > 0) {
      // Newest first: what was just scrolled into view (or the sidebar) is
      // asked before a long backlog of rows that may already be gone.
      const item = this.coverQueue.pop()!;
      if (item.signal?.aborted) {
        item.resolve(null);
        continue;
      }
      const existing = this.covers.get(item.k);
      if (existing) {
        this.covers.delete(item.k);
        this.covers.set(item.k, existing);
        void existing.then(item.resolve);
        continue;
      }
      if (this.covers.size >= MAX_COVER_CACHE) {
        const oldest = this.covers.keys().next().value;
        if (oldest !== undefined) this.covers.delete(oldest);
      }
      this.coverActive++;
      // Before the backend is up a miss is no real miss: asked again next time.
      let offline = false;
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
      const p = Promise.race([
        invoke<string | null>("cover_path", { track: item.t })
          .then((path) => (path ? convertFileSrc(path) : null))
          .catch((e) => { if (/backend_offline/.test(String(e))) offline = true; return null; }),
        new Promise<null>((r) => setTimeout(() => r(null), COVER_GIVE_UP_MS)),
      ]).finally(() => {
        clearTimeout(slot);
        release();
      });
      this.covers.set(item.k, p);
      p.then((v) => {
        if (offline) {
          // The core was still starting: this is no real miss. Ask again
          // shortly (the sidebar used to keep placeholder icons for good).
          if (this.covers.get(item.k) === p) this.covers.delete(item.k);
          if (performance.now() - item.since < OFFLINE_RETRY_MS && !item.signal?.aborted) {
            setTimeout(() => { this.coverQueue.unshift(item); this.drainCovers(); }, 800);
            return;
          }
        } else if (!v && this.covers.get(item.k) === p) {
          setTimeout(() => { if (this.covers.get(item.k) === p) this.covers.delete(item.k); }, 15_000);
        }
        item.resolve(v);
      });
    }
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
