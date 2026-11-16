import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Backend } from "./backend";
import type { BackendEvents, Snapshot, Track } from "./types";
import { trackKey } from "./types";

const MAX_COVER_CACHE = 300;
const MAX_COVER_ACTIVE = 3;

export class TauriBackend implements Backend {
  readonly kind = "tauri" as const;
  private covers = new Map<string, Promise<string | null>>();
  private coverQueue: { t: Track; k: string; signal?: AbortSignal; resolve: (v: string | null) => void }[] = [];
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
      this.coverQueue.push({ t, k, signal, resolve });
      this.drainCovers();
    });
  }

  private drainCovers() {
    while (this.coverActive < MAX_COVER_ACTIVE && this.coverQueue.length > 0) {
      const item = this.coverQueue.shift()!;
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
      const p = invoke<string | null>("cover_path", { track: item.t })
        .then((path) => (path ? convertFileSrc(path) : null))
        .catch((e) => { if (/backend_offline/.test(String(e))) this.covers.delete(item.k); return null; })
        .finally(() => {
          this.coverActive--;
          this.drainCovers();
        });
      this.covers.set(item.k, p);
      p.then((v) => {
        if (!v && this.covers.get(item.k) === p) {
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
