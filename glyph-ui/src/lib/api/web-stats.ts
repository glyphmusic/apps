// Listening statistics in the browser (mirrors glyph_core::stats).
// Stored locally under `glyph.web.stats` while `config.stats.enabled` is on,
// and synced as `stats` items keyed `<install_id>:<YYYY-MM>` when signed in.
import type { StatsRange, StatsRecap, StatsSummary } from "../pages/stats/types";
import { primaryArtist, trackKey, type Track } from "./types";
import { load, remove, saveSoon } from "./web-store";

const MAX_ARTISTS = 150;
const MAX_TRACKS = 200;
const KEEP_MONTHS = 25;

export interface ArtistStat {
  ms: number;
  plays: number;
}

export interface TrackStat {
  track: Track;
  ms: number;
  plays: number;
}

export interface MonthStats {
  ms: number;
  plays: number;
  skips: number;
  artists: Record<string, [string, ArtistStat]>;
  tracks: Record<string, TrackStat>;
  hours: number[];
  weekdays: number[];
  days: number[];
}

interface StatsData {
  months: Record<string, MonthStats>;
  remote: Record<string, MonthStats>;
}

function emptyMonth(): MonthStats {
  return {
    ms: 0,
    plays: 0,
    skips: 0,
    artists: {},
    tracks: {},
    hours: new Array<number>(24).fill(0),
    weekdays: new Array<number>(7).fill(0),
    days: new Array<number>(31).fill(0),
  };
}

function normalizeMonth(raw: Partial<MonthStats> | null | undefined): MonthStats {
  const m = emptyMonth();
  if (!raw || typeof raw !== "object") return m;
  m.ms = Number(raw.ms) || 0;
  m.plays = Number(raw.plays) || 0;
  m.skips = Number(raw.skips) || 0;
  if (raw.artists && typeof raw.artists === "object") {
    for (const [k, v] of Object.entries(raw.artists)) {
      if (Array.isArray(v) && v.length === 2 && typeof v[0] === "string" && v[1]) {
        m.artists[k] = [v[0], { ms: Number(v[1].ms) || 0, plays: Number(v[1].plays) || 0 }];
      }
    }
  }
  if (raw.tracks && typeof raw.tracks === "object") {
    for (const [k, v] of Object.entries(raw.tracks)) {
      if (v && typeof v === "object" && v.track) {
        m.tracks[k] = { track: v.track, ms: Number(v.ms) || 0, plays: Number(v.plays) || 0 };
      }
    }
  }
  if (Array.isArray(raw.hours)) for (let i = 0; i < 24; i++) m.hours[i] = Number(raw.hours[i]) || 0;
  if (Array.isArray(raw.weekdays)) for (let i = 0; i < 7; i++) m.weekdays[i] = Number(raw.weekdays[i]) || 0;
  if (Array.isArray(raw.days)) for (let i = 0; i < 31; i++) m.days[i] = Number(raw.days[i]) || 0;
  return m;
}

function trimMonth(m: MonthStats) {
  const ak = Object.entries(m.artists);
  if (ak.length > MAX_ARTISTS) {
    ak.sort((a, b) => b[1][1].ms - a[1][1].ms);
    m.artists = Object.fromEntries(ak.slice(0, MAX_ARTISTS));
  }
  const tk = Object.entries(m.tracks);
  if (tk.length > MAX_TRACKS) {
    tk.sort((a, b) => b[1].ms - a[1].ms);
    m.tracks = Object.fromEntries(tk.slice(0, MAX_TRACKS));
  }
}

function addInto(dst: MonthStats, src: MonthStats) {
  dst.ms += src.ms;
  dst.plays += src.plays;
  dst.skips += src.skips;
  for (const [k, [name, a]] of Object.entries(src.artists)) {
    const cur = dst.artists[k] ?? (dst.artists[k] = [name, { ms: 0, plays: 0 }]);
    cur[1].ms += a.ms;
    cur[1].plays += a.plays;
  }
  for (const [k, t] of Object.entries(src.tracks)) {
    const cur = dst.tracks[k];
    if (cur) {
      cur.ms += t.ms;
      cur.plays += t.plays;
    } else {
      dst.tracks[k] = { track: t.track, ms: t.ms, plays: t.plays };
    }
  }
  for (let i = 0; i < 24; i++) dst.hours[i] = (dst.hours[i] ?? 0) + (src.hours[i] ?? 0);
  for (let i = 0; i < 7; i++) dst.weekdays[i] = (dst.weekdays[i] ?? 0) + (src.weekdays[i] ?? 0);
  for (let i = 0; i < 31; i++) dst.days[i] = (dst.days[i] ?? 0) + (src.days[i] ?? 0);
}

function mergeMaxInto(dst: MonthStats, src: MonthStats) {
  dst.ms = Math.max(dst.ms, src.ms);
  dst.plays = Math.max(dst.plays, src.plays);
  dst.skips = Math.max(dst.skips, src.skips);
  for (const [k, [name, a]] of Object.entries(src.artists)) {
    const cur = dst.artists[k] ?? (dst.artists[k] = [name, { ms: 0, plays: 0 }]);
    cur[1].ms = Math.max(cur[1].ms, a.ms);
    cur[1].plays = Math.max(cur[1].plays, a.plays);
  }
  for (const [k, t] of Object.entries(src.tracks)) {
    const cur = dst.tracks[k];
    if (cur) {
      cur.ms = Math.max(cur.ms, t.ms);
      cur.plays = Math.max(cur.plays, t.plays);
    } else {
      dst.tracks[k] = { track: t.track, ms: t.ms, plays: t.plays };
    }
  }
  for (let i = 0; i < 24; i++) dst.hours[i] = Math.max(dst.hours[i] ?? 0, src.hours[i] ?? 0);
  for (let i = 0; i < 7; i++) dst.weekdays[i] = Math.max(dst.weekdays[i] ?? 0, src.weekdays[i] ?? 0);
  for (let i = 0; i < 31; i++) dst.days[i] = Math.max(dst.days[i] ?? 0, src.days[i] ?? 0);
  trimMonth(dst);
}

function nowParts(d = new Date()) {
  const ym = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
  const hour = d.getHours();
  const weekday = (d.getDay() + 6) % 7; // Monday = 0 .. Sunday = 6
  const day = Math.max(1, Math.min(31, d.getDate())) - 1;
  return { ym, hour, weekday, day };
}

function previousMonth(ym: string): string {
  const y = Number(ym.slice(0, 4)) || 1970;
  const m = Number(ym.slice(5, 7)) || 1;
  return m <= 1 ? `${y - 1}-12` : `${y}-${String(m - 1).padStart(2, "0")}`;
}

function creditedArtists(artist: string): [string, string][] {
  const parts = artist
    .split(/\s*(?:,|&|\bfeat\.?\b|\bft\.?\b|\bwith\b|\bx\b|;|\/)\s*/i)
    .map((s) => s.trim())
    .filter(Boolean);
  const list = parts.length ? parts.slice(0, 3) : [primaryArtist(artist)].filter(Boolean);
  const out: [string, string][] = [];
  const seen = new Set<string>();
  for (const name of list) {
    const k = name.toLowerCase();
    if (k && !seen.has(k)) {
      seen.add(k);
      out.push([k, name]);
    }
  }
  return out;
}

export class WebStats {
  private months: Record<string, MonthStats> = {};
  private remote: Record<string, MonthStats> = {};
  private onChange?: () => void;

  constructor(onChange?: () => void) {
    this.onChange = onChange;
    const raw = load<StatsData>("stats");
    if (raw?.months) {
      for (const [k, v] of Object.entries(raw.months)) this.months[k] = normalizeMonth(v);
    }
    if (raw?.remote) {
      for (const [k, v] of Object.entries(raw.remote)) this.remote[k] = normalizeMonth(v);
    }
  }

  private persist(notify = true) {
    saveSoon("stats", () => ({ months: this.months, remote: this.remote }), 1500);
    if (notify) this.onChange?.();
  }

  private monthMut(ym: string): MonthStats {
    let m = this.months[ym];
    if (!m) {
      m = emptyMonth();
      this.months[ym] = m;
      const keys = Object.keys(this.months).sort();
      while (keys.length > KEEP_MONTHS) {
        const drop = keys.shift()!;
        delete this.months[drop];
      }
    }
    return m;
  }

  isEmpty(): boolean {
    return Object.keys(this.months).length === 0 && Object.keys(this.remote).length === 0;
  }

  ownMonths(): Record<string, MonthStats> {
    return JSON.parse(JSON.stringify(this.months));
  }

  mergeGuestMonths(guest: Record<string, MonthStats>) {
    for (const [ym, gm] of Object.entries(guest)) {
      const m = this.monthMut(ym);
      addInto(m, normalizeMonth(gm));
      trimMonth(m);
    }
    this.persist();
  }

  clear() {
    this.months = {};
    this.remote = {};
    remove("stats");
  }

  addTime(track: Track, ms: number) {
    const add = Math.round(ms);
    if (add <= 0) return;
    const { ym, hour, weekday, day } = nowParts();
    const m = this.monthMut(ym);
    m.ms += add;
    m.hours[hour] = (m.hours[hour] ?? 0) + add;
    m.weekdays[weekday] = (m.weekdays[weekday] ?? 0) + add;
    m.days[day] = (m.days[day] ?? 0) + add;
    for (const [ak, display] of creditedArtists(track.artist)) {
      const e = m.artists[ak] ?? (m.artists[ak] = [display, { ms: 0, plays: 0 }]);
      e[1].ms += add;
    }
    const tk = trackKey(track);
    if (tk) {
      const t = m.tracks[tk];
      if (t) t.ms += add;
      else m.tracks[tk] = { track, ms: add, plays: 0 };
    }
    trimMonth(m);
    this.persist();
  }

  addPlay(track: Track) {
    const { ym } = nowParts();
    const m = this.monthMut(ym);
    m.plays += 1;
    for (const [ak, display] of creditedArtists(track.artist)) {
      const e = m.artists[ak] ?? (m.artists[ak] = [display, { ms: 0, plays: 0 }]);
      e[1].plays += 1;
    }
    const tk = trackKey(track);
    if (tk) {
      const t = m.tracks[tk];
      if (t) t.plays += 1;
      else m.tracks[tk] = { track, ms: 0, plays: 1 };
    }
    trimMonth(m);
    this.persist();
  }

  addSkip() {
    const { ym } = nowParts();
    const m = this.monthMut(ym);
    m.skips += 1;
    this.persist();
  }

  /** Sync items for this browser install (`<install_id>:<YYYY-MM>`). */
  syncItems(device: string): [string, MonthStats][] {
    return Object.entries(this.months).map(([ym, m]) => [`${device}:${ym}`, m]);
  }

  /** Apply a synced `stats` item from the account. */
  applyRemote(device: string, key: string, value: unknown | null): boolean {
    const prefix = `${device}:`;
    if (key.startsWith(prefix)) {
      const ym = key.slice(prefix.length);
      if (ym.length === 7 && value && typeof value === "object") {
        const m = this.monthMut(ym);
        mergeMaxInto(m, normalizeMonth(value as Partial<MonthStats>));
        this.persist(false);
      }
      return false;
    }
    if (value == null) {
      if (key in this.remote) {
        delete this.remote[key];
        this.persist(false);
        return true;
      }
      return false;
    }
    const next = normalizeMonth(value as Partial<MonthStats>);
    const prev = this.remote[key];
    const changed = JSON.stringify(prev) !== JSON.stringify(next);
    this.remote[key] = next;
    if (changed) this.persist(false);
    return changed;
  }

  private combinedByMonth(): Map<string, MonthStats> {
    const map = new Map<string, MonthStats>();
    const add = (ym: string, s: MonthStats) => {
      if (ym.length !== 7) return;
      let dst = map.get(ym);
      if (!dst) {
        dst = emptyMonth();
        map.set(ym, dst);
      }
      addInto(dst, s);
    };
    for (const [ym, s] of Object.entries(this.months)) add(ym, s);
    for (const [k, s] of Object.entries(this.remote)) {
      const idx = k.lastIndexOf(":");
      if (idx >= 0) add(k.slice(idx + 1), s);
    }
    return new Map([...map.entries()].sort((a, b) => a[0].localeCompare(b[0])));
  }

  private formatLists(m: MonthStats) {
    const top_tracks = Object.values(m.tracks)
      .filter((t) => t.ms > 0 || t.plays > 0)
      .sort((a, b) => b.ms - a.ms || b.plays - a.plays)
      .slice(0, 50)
      .map((t) => ({ track: t.track, plays: t.plays, ms: t.ms }));

    const top_artists = Object.values(m.artists)
      .filter(([, a]) => a.ms > 0 || a.plays > 0)
      .sort((a, b) => b[1].ms - a[1].ms || b[1].plays - a[1].plays)
      .slice(0, 50)
      .map(([name, a]) => {
        const seed = top_tracks.find((t) => t.track.artist.toLowerCase().includes(name.toLowerCase()))?.track ?? null;
        return { name, ms: a.ms, plays: a.plays, track: seed };
      });

    return { top_artists, top_tracks };
  }

  summary(range: StatsRange): StatsSummary {
    const { ym: cur } = nowParts();
    const last = previousMonth(cur);
    const yearPrefix = cur.slice(0, 4);
    const wanted = (ym: string) =>
      range === "month"
        ? ym === cur
        : range === "last_month"
          ? ym === last
          : range === "year"
            ? ym.startsWith(yearPrefix)
            : true;

    const byMonth = this.combinedByMonth();
    const tot = emptyMonth();
    let active_days = 0;
    for (const [ym, ms] of byMonth.entries()) {
      if (wanted(ym)) {
        addInto(tot, ms);
        active_days += ms.days.filter((d) => d > 0).length;
      }
    }

    const months = [...byMonth.entries()].map(([month, m]) => ({ month, ms: m.ms }));
    const yearsSet = new Set<string>();
    for (const { month, ms } of months) {
      if (ms > 0) yearsSet.add(month.slice(0, 4));
    }
    const years = [...yearsSet].sort().reverse();
    const { top_artists, top_tracks } = this.formatLists(tot);

    return {
      range,
      ms: tot.ms,
      plays: tot.plays,
      skips: tot.skips,
      artists: Object.keys(tot.artists).length,
      songs: Object.keys(tot.tracks).length,
      active_days,
      top_artists,
      top_tracks,
      hours: tot.hours,
      weekdays: tot.weekdays,
      months,
      years,
    };
  }

  recap(year: string): StatsRecap {
    const byMonth = this.combinedByMonth();
    const tot = emptyMonth();
    const months: { month: string; ms: number }[] = [];
    for (const [ym, ms] of byMonth.entries()) {
      if (ym.startsWith(year)) {
        addInto(tot, ms);
        months.push({ month: ym, ms: ms.ms });
      }
    }
    const { top_artists, top_tracks } = this.formatLists(tot);
    return {
      year,
      ms: tot.ms,
      plays: tot.plays,
      artists: Object.keys(tot.artists).length,
      songs: Object.keys(tot.tracks).length,
      top_artists,
      top_tracks,
      hours: tot.hours,
      weekdays: tot.weekdays,
      months,
    };
  }
}
