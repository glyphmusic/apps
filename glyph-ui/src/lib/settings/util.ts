// Small helpers of the settings pages.
import { app } from "../state.svelte";
import { backend } from "../api/backend";

/** True when running on a mobile OS or in the compact phone layout. */
export function isMobile(): boolean {
  return app.compact || app.platform === "android" || app.platform === "ios";
}

/** The phone app (Android / iOS), not just a narrow window. */
export function onPhone(): boolean {
  return app.platform === "android" || app.platform === "ios";
}

/** Opens a link in the system browser. */
export function openUrl(url: string) {
  backend().call("open_url", { url }).catch(() => {});
}

/** Copies text; false when the browser refuses. */
export async function copyText(text: string): Promise<boolean> {
  try { await navigator.clipboard.writeText(text); return true; } catch { return false; }
}

export async function readClipboard(): Promise<string> {
  try { return await navigator.clipboard.readText(); } catch { return ""; }
}

/** Hands a text to the user as a file (browser download; the Tauri side may show a save dialog instead). */
export function saveText(name: string, text: string, mime = "text/plain") {
  const url = URL.createObjectURL(new Blob([text], { type: mime }));
  const a = document.createElement("a");
  a.href = url; a.download = name;
  document.body.appendChild(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 2000);
}

/** Lets the user pick files and reads them as text. */
export function pickFiles(accept = "", multiple = false): Promise<{ name: string; text: string }[]> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file"; input.accept = accept; input.multiple = multiple;
    input.onchange = async () => resolve(await Promise.all([...(input.files ?? [])].map(async (f) => ({ name: f.name, text: await f.text() }))));
    input.oncancel = () => resolve([]);
    input.click();
  });
}

/** utils: 1536 -> "1.5 KB" (glyph_core::storage::format_bytes). */
export function formatBytes(b: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = b, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  if (i === 0) return `${b} B`;
  return v < 10 ? `${v.toFixed(1)} ${units[i]}` : `${v.toFixed(0)} ${units[i]}`;
}

/** account.rs when_ms: "just now", "5 min ago", "3 h ago", "2 d ago". */
export function whenMs(ms: number, t: (k: string) => string, tf: (k: string, a: Record<string, string | number>) => string): string {
  const d = Math.max(0, Date.now() - ms) / 1000;
  if (d < 120) return t("account.sync.just_now");
  if (d < 7200) return tf("account.sync.min_ago", { n: Math.floor(d / 60) });
  if (d < 172800) return tf("account.hours_ago", { n: Math.floor(d / 3600) });
  return tf("account.days_ago", { n: Math.floor(d / 86400) });
}

/** The messages a backend call rejects with. */
export function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Calls `f` once, `ms` after the last call (sliders that save the config). */
export function debounced(f: () => void, ms = 250): () => void {
  let id = 0;
  return () => { clearTimeout(id); id = window.setTimeout(f, ms); };
}
