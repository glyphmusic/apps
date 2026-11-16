// The bridge between the UI and Glyph's Rust core. In the Tauri app every
// `call` is a Tauri command (`invoke`) and every event a Tauri event named
// `glyph://<event>`. The web player (`npm run build:web`, served at
// glyph.my/web) talks to glyph-server and plays in the browser (web.ts).
// Anywhere else (`npm run dev`) the demo backend stands in, so the whole UI
// can be looked at without the Rust side.
import type { BackendEvents, Snapshot, Track } from "./types";

export interface Backend {
  readonly kind: "tauri" | "web" | "demo";
  snapshot(): Promise<Snapshot>;
  call<T = void>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  on<K extends keyof BackendEvents>(ev: K, cb: (payload: BackendEvents[K]) => void): () => void;
  /** URL of a song's cover picture (cached by the backend), or null. */
  cover(t: Track, signal?: AbortSignal): Promise<string | null>;
  /** Trims cached cover URL lookups down to at most `keep` recent entries. */
  trimCovers?(keep: number): void;
}

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** The web player build, or this UI served under `/web` (glyph.my/web). */
export function isWeb(): boolean {
  return import.meta.env.VITE_GLYPH_WEB === "1" || (typeof location !== "undefined" && /^\/web(\/|$)/.test(location.pathname));
}

let current: Backend | null = null;

export async function connect(): Promise<Backend> {
  if (current) return current;
  if (isTauri()) {
    const { TauriBackend } = await import("./tauri");
    current = new TauriBackend();
  } else if (import.meta.env.VITE_GLYPH_WEB === "1" || isWeb()) {
    // (The first test is fixed at build time, so the web build leaves the demo out.)
    const { WebBackend } = await import("./web");
    current = new WebBackend();
  } else {
    const { DemoBackend } = await import("./demo");
    current = new DemoBackend();
  }
  return current;
}

export function backend(): Backend {
  if (!current) throw new Error("backend not connected");
  return current;
}
