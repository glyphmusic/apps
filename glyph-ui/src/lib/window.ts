// Window actions. Tauri: the frameless window's own controls; browser
// preview: nothing to do.
import type { Window as TauriWindow } from "@tauri-apps/api/window";
import { isTauri } from "./api/backend";

let cached: TauriWindow | null = null;
let loading: Promise<TauriWindow | null> | null = null;

function win(): Promise<TauriWindow | null> {
  if (!isTauri()) return Promise.resolve(null);
  if (cached) return Promise.resolve(cached);
  return (loading ??= import("@tauri-apps/api/window").then((m) => (cached = m.getCurrentWindow())));
}
if (isTauri()) void win();

export const minimize = async () => (await win())?.minimize();
export const toggleMaximize = async () => {
  const cw = await win();
  if (!cw) return;
  await cw.toggleMaximize();
  // Windows can desync WebView focus/pointer capture across SW_RESTORE + SWP_NOACTIVATE.
  if (!(await cw.isMaximized())) await cw.setFocus();
};
export const isMaximized = async () => (await win())?.isMaximized() ?? false;
export const close = async () => (await win())?.close();

/** Tracks whether the current window is maximized (button click, double-click, snap). */
export function watchMaximized(cb: (maximized: boolean) => void): () => void {
  let dead = false;
  let off: (() => void) | null = null;
  win().then(async (cw) => {
    if (!cw || dead) return;
    const sync = async () => {
      const m = await cw.isMaximized();
      if (!dead) cb(m);
    };
    await sync();
    const u = await cw.onResized(sync);
    if (dead) u();
    else off = u;
  });
  return () => { dead = true; off?.(); };
}
export function startDrag() {
  if (cached) return cached.startDragging();
  return win().then((cw) => cw?.startDragging());
}
export const isMac = () => /Mac|iPhone|iPad/.test(navigator.platform);
export const shortcut = () => (isMac() ? "⌘K" : "Ctrl K");

/** Titlebar mouse down: drag, double click maximises. */
export function titlebarDown(e: MouseEvent) {
  if (!isTauri() || e.button !== 0) return;
  if ((e.target as HTMLElement).closest("button, input, a, [data-no-drag]")) return;
  e.preventDefault();
  if (e.detail === 2) void toggleMaximize();
  else void startDrag();
}

/** This webview is the mini player window (Tauri opens it with `#mini`). */
export const inMiniWindow = () => isTauri() && location.hash === "#mini";
/** Mini player window: new size in CSS pixels. */
export async function setWindowSize(w: number, h: number) {
  const cw = await win();
  if (!cw) return;
  const { LogicalSize } = await import("@tauri-apps/api/dpi");
  await cw.setSize(new LogicalSize(Math.round(w), Math.round(h)));
}
/** Mini player window: resize from the bottom right corner. */
export const startResize = async () => (await win())?.startResizeDragging("SouthEast");
