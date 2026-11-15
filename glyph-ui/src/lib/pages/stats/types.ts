// Wire types of the statistics page (glyph_core::stats summaries). The
// commands are listed at the top of api/demo/extra.ts.
import type { Track } from "../../api/types";

export type StatsRange = "month" | "last_month" | "year" | "all";

export interface StatsSummary {
  range: StatsRange;
  /** Listening time (ms), plays, skipped plays. */
  ms: number;
  plays: number;
  skips: number;
  /** Different artists / songs heard. */
  artists: number;
  songs: number;
  /** Days with listening. */
  active_days: number;
  top_artists: { name: string; ms: number; plays: number; track?: Track | null }[];
  top_tracks: { track: Track; plays: number; ms: number }[];
  /** Listening time per hour of the day (24) and per weekday (7, Monday first). */
  hours: number[];
  weekdays: number[];
  /** Listening time per month, oldest first ("2026-03"). */
  months: { month: string; ms: number }[];
  /** Years with data, newest first ("2026"). */
  years: string[];
}

export interface StatsRecap {
  year: string;
  ms: number;
  plays: number;
  artists: number;
  songs: number;
  top_artists: { name: string; ms: number; plays: number; track?: Track | null }[];
  top_tracks: { track: Track; plays: number; ms: number }[];
  hours: number[];
  weekdays: number[];
  months: { month: string; ms: number }[];
}

/** "12 h 05 min" / "42 min" (stats.rs fmt_listen). */
export function fmtListen(ms: number): string {
  const min = Math.floor(ms / 60_000);
  return min >= 60 ? `${Math.floor(min / 60)} h ${String(min % 60).padStart(2, "0")} min` : `${min} min`;
}

/** Index of the largest value (first on ties), or null when all are 0. */
export function peakOf(v: number[]): number | null {
  let best = -1, at: number | null = null;
  v.forEach((x, i) => { if (x > best && x > 0) { best = x; at = i; } });
  return at;
}
