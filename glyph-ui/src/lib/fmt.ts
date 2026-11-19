// Number and time formats of the app (widgets/bars.rs fmt_time, lists.rs meta).
export function fmtTime(sec: number): string {
  if (!isFinite(sec) || sec < 0) sec = 0;
  const s = Math.floor(sec);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export function fmtMs(ms?: number | null): string {
  return ms == null ? "-:--" : fmtTime(ms / 1000);
}

/** Total length as m:ss or h:mm:ss. */
export function fmtTotal(ms: number): string {
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), r = s % 60;
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${String(r).padStart(2, "0")}` : `${m}:${String(r).padStart(2, "0")}`;
}

/** Plain sans-serif list metadata without brackets (e.g. `8 Songs · 39:18`). */
export function listMeta(n: number, totalMs: number, singular: string, many: string): string {
  return `${n === 1 ? `1 ${singular}` : many} · ${fmtTotal(totalMs)}`;
}

export function compact(n: number): string {
  if (n >= 1e6) return `${(n / 1e6).toFixed(n >= 1e7 ? 0 : 1)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(n >= 1e4 ? 0 : 1)}K`;
  return String(n);
}

export const clamp = (v: number, a: number, b: number) => Math.min(b, Math.max(a, v));
/** Drops later items whose key was already seen. Results from several
 *  sources can hold the same song, artist or album twice; a list keyed by
 *  it would crash on the second one. */
export function uniqBy<T>(items: readonly T[], key: (x: T) => string): T[] {
  const seen = new Set<string>();
  return items.filter((x) => {
    const k = key(x);
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });
}
