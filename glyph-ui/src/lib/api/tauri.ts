import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Backend } from "./backend";
import type { BackendEvents, Snapshot, Track } from "./types";
import { trackKey } from "./types";

const MAX_COVER_CACHE = 300;

export class TauriBackend implements Backend {
  readonly kind = "tauri" as const;
  private covers = new Map<string, Promise<string | null>>();

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
  cover(t: Track): Promise<string | null> {
    const k = trackKey(t);
    let p = this.covers.get(k);
    if (p) {
      // Touch for LRU order.
      this.covers.delete(k);
      this.covers.set(k, p);
      return p;
    }
    if (this.covers.size >= MAX_COVER_CACHE) {
      const oldest = this.covers.keys().next().value;
      if (oldest !== undefined) this.covers.delete(oldest);
    }
    // Before the backend is up a miss is no real miss: asked again next time.
    p = invoke<string | null>("cover_path", { track: t })
      .then((path) => (path ? convertFileSrc(path) : null))
      .catch((e) => { if (/backend_offline/.test(String(e))) this.covers.delete(k); return null; });
    this.covers.set(k, p);
    p.then((v) => { if (!v && this.covers.get(k) === p) setTimeout(() => { if (this.covers.get(k) === p) this.covers.delete(k); }, 30_000); });
    return p;
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
