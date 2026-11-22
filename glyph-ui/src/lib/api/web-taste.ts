// Lightweight local Taste Engine for the Web Player & Demo backend.
// Mirrors glyph-core's TasteData / TasteProfile / DailyMix semantics and
// respects `config.taste.enabled`, `config.taste.server_collab`, and
// `config.taste.metadata_enrichment`.

import type {
  CollabArtistRec,
  DailyMix,
  Library,
  OnboardingCatalogResponse,
  TasteConfig,
  TasteProfile,
  Track,
  VersionInsight,
  WeightedTag,
} from "./types";
import { primaryArtist, trackKey } from "./types";

export const VERSION_STYLES = [
  "live",
  "acoustic",
  "remix",
  "slowed",
  "sped_up",
  "nightcore",
  "instrumental",
  "karaoke",
  "cover",
  "8d",
  "lofi",
  "demo",
] as const;

const DEFAULT_ODD = new Set(["karaoke", "instrumental", "8d", "nightcore", "sped_up", "slowed", "cover"]);

const BUILTIN_GENRES: Record<string, string[]> = {
  "tame impala": ["Psychedelic", "Indie", "Dream Pop"],
  "arctic monkeys": ["Indie Rock", "Alternative", "Rock"],
  "men i trust": ["Dream Pop", "Indie Pop", "Chillwave"],
  "daft punk": ["Electronic", "House", "Disco"],
  "fred again..": ["Electronic", "House", "UK Garage"],
  "kaytranada": ["House", "Electronic", "R&B"],
  "kendrick lamar": ["Hip-Hop", "Rap"],
  "tyler, the creator": ["Hip-Hop", "Alternative", "Soul"],
  "travis scott": ["Trap", "Hip-Hop"],
  "frank ocean": ["R&B", "Soul", "Alternative"],
  "sza": ["R&B", "Soul", "Pop"],
  "the weeknd": ["Pop", "R&B", "Synthpop"],
  "billie eilish": ["Pop", "Alternative", "Electropop"],
  "dua lipa": ["Pop", "Disco", "Dance"],
  "radiohead": ["Alternative", "Indie Rock", "Art Pop"],
  "linkin park": ["Rock", "Alternative", "Metal"],
  "metallica": ["Metal", "Rock"],
  "deftones": ["Metal", "Shoegaze", "Alternative"],
  "miles davis": ["Jazz"],
  "nujabes": ["Lo-Fi", "Hip-Hop", "Jazz"],
  "lana del rey": ["Dream Pop", "Indie Pop", "Pop"],
  "bon iver": ["Indie Folk", "Folk & Acoustic"],
  "bad bunny": ["Latin", "Trap"],
  "burna boy": ["Afrobeats", "Pop"],
  "hans zimmer": ["Soundtrack", "Classical"],
  "ludovico einaudi": ["Classical", "Instrumental"],
  "gorillaz": ["Alternative", "Electronic", "Hip-Hop"],
  "flume": ["Electronic", "Downtempo"],
  "khruangbin": ["Psychedelic", "Funk", "Soul"],
  "crumb": ["Psychedelic", "Indie", "Jazz"],
  "beach house": ["Dream Pop", "Shoegaze", "Indie"],
};

function lookupBuiltinGenres(k: string): string[] {
  const clean = k.trim().toLowerCase();
  return (
    BUILTIN_GENRES[clean] ??
    BUILTIN_GENRES[clean.replace(/\.+$/, "")] ??
    BUILTIN_GENRES[`${clean}..`] ??
    BUILTIN_GENRES[clean.replace(/^the\s+/, "")] ??
    BUILTIN_GENRES[`the ${clean}`] ??
    (clean === "tyler" || clean === "tyler the creator" ? BUILTIN_GENRES["tyler, the creator"] : undefined) ??
    []
  );
}

const GENRE_MOODS: Record<string, string[]> = {
  "Dream Pop": ["Dreamy", "Atmospheric"],
  "Shoegaze": ["Dreamy", "Atmospheric"],
  "Psychedelic": ["Dreamy", "Groovy"],
  "Electronic": ["Energetic", "Atmospheric"],
  "House": ["Groovy", "Energetic"],
  "Indie": ["Chill", "Nostalgic"],
  "Indie Rock": ["Nostalgic", "Energetic"],
  "Indie Pop": ["Dreamy", "Upbeat"],
  "Alternative": ["Melancholic", "Energetic"],
  "Hip-Hop": ["Groovy", "Energetic"],
  "Rap": ["Energetic", "Intense"],
  "Trap": ["Dark", "Energetic"],
  "R&B": ["Warm", "Late Night"],
  "Soul": ["Warm", "Groovy"],
  "Pop": ["Upbeat"],
  "Rock": ["Energetic"],
  "Metal": ["Intense", "Dark"],
  "Jazz": ["Chill", "Warm"],
  "Lo-Fi": ["Chill", "Focus"],
  "Classical": ["Focus", "Atmospheric"],
  "Soundtrack": ["Atmospheric", "Focus"],
  "Folk & Acoustic": ["Warm", "Chill"],
  "Latin": ["Upbeat", "Groovy"],
  "Afrobeats": ["Groovy", "Upbeat"],
};

export interface LocalTasteStore {
  onboarding_artists: string[];
  onboarding_tracks: Track[];
  onboarding_genres: string[];
  onboarding_done: boolean;
  artist_feedback: Record<string, number>;
  disliked_tracks: Record<string, Track>;
  tag_feedback: Record<string, number>;
  version_feedback: Record<string, number>;
  artist_plays: Record<string, number>;
  artist_skips: Record<string, number>;
  time_buckets: [number, number, number, number];
  version_plays: Record<string, number>;
  version_skips: Record<string, number>;
  artist_tags: Record<string, { genres: string[]; moods: string[]; similar: string[] }>;
  collab_recs: CollabArtistRec[];
  collab_contributors: number;
  collab_model_ready: boolean;
}

export function emptyTasteStore(): LocalTasteStore {
  return {
    onboarding_artists: [],
    onboarding_tracks: [],
    onboarding_genres: [],
    onboarding_done: false,
    artist_feedback: {},
    disliked_tracks: {},
    tag_feedback: {},
    version_feedback: {},
    artist_plays: {},
    artist_skips: {},
    time_buckets: [0, 0, 0, 0],
    version_plays: {},
    version_skips: {},
    artist_tags: {},
    collab_recs: [],
    collab_contributors: 0,
    collab_model_ready: false,
  };
}

export function timeBucketOf(hour = new Date().getHours()): "morning" | "afternoon" | "evening" | "night" {
  const h = ((hour % 24) + 24) % 24;
  if (h >= 5 && h < 11) return "morning";
  if (h >= 11 && h < 17) return "afternoon";
  if (h >= 17 && h < 22) return "evening";
  return "night";
}

export function detectVersions(title: string): string[] {
  const clean = ` ${title.toLowerCase().replace(/[^a-z0-9]+/g, " ")} `;
  const out: string[] = [];
  const has = (p: string) => clean.includes(` ${p} `);
  if (has("karaoke")) out.push("karaoke");
  if (has("instrumental")) out.push("instrumental");
  if (has("8d")) out.push("8d");
  if (has("nightcore")) out.push("nightcore");
  if (has("sped up") || has("speed up")) out.push("sped_up");
  if (has("slowed")) out.push("slowed");
  if (has("cover") || has("tribute") || has("remake")) out.push("cover");
  if (has("remix") || has("club mix") || has("extended mix")) out.push("remix");
  if (has("acoustic") || has("unplugged")) out.push("acoustic");
  if (has("live") || has("live at") || has("live from")) out.push("live");
  if (has("lofi") || has("lo fi")) out.push("lofi");
  if (has("demo")) out.push("demo");
  return out;
}

export function fallbackOnboardingCatalog(tracks: Track[] = []): OnboardingCatalogResponse {
  const entries = Object.entries(BUILTIN_GENRES).slice(0, 28);
  const genres: string[] = [];
  const artists = entries.map(([norm, gs]) => {
    const genre = gs[0] ?? "Pop";
    if (!genres.includes(genre)) genres.push(genre);
    const name = norm
      .split(" ")
      .map((w) => (w === "sza" ? "SZA" : w.charAt(0).toUpperCase() + w.slice(1)))
      .join(" ");
    const sample = tracks.find((t) => primaryArtist(t.artist).toLowerCase() === norm) ?? null;
    return { name, genre, photo: sample?.cover_url ?? null, sample_track: sample };
  });
  return { genres, artists, tracks: tracks.slice(0, 24) };
}

export function computeTasteProfile(
  lib: Library,
  store: LocalTasteStore,
  cfg: TasteConfig,
  serverAvailable: boolean,
): TasteProfile {
  const mode: "hybrid" | "local" | "off" = !cfg.enabled
    ? "off"
    : cfg.server_collab && serverAvailable
      ? "hybrid"
      : "local";

  const weights = new Map<string, { name: string; w: number; reasons: Set<string> }>();
  const add = (raw: string, delta: number, reason?: string) => {
    const name = primaryArtist(raw).trim();
    const k = name.toLowerCase();
    if (!k) return;
    const e = weights.get(k) ?? { name, w: 0, reasons: new Set<string>() };
    e.w += delta;
    if (reason) e.reasons.add(reason);
    weights.set(k, e);
  };

  for (const t of lib.liked) add(t.artist, 3.0, "liked");
  for (const f of lib.followed) add(f, 6.0, "followed");
  for (const a of store.onboarding_artists) add(a, 6.5, "onboarding");
  for (const t of store.onboarding_tracks) add(t.artist, 4.0, "onboarding");
  for (const r of lib.recent) {
    if (r.kind === "song") add(r.track.artist, 1.3, "recent");
    else if (r.kind === "artist") add(r.name, 1.5, "recent");
  }
  for (const [a, n] of Object.entries(store.artist_plays)) add(a, Math.log1p(n) * 1.2, n >= 3 ? "frequent" : undefined);
  for (const [a, n] of Object.entries(store.artist_skips)) add(a, -Math.min(10, n) * 1.0, n >= 2 ? "high_skips" : undefined);
  for (const [a, p] of Object.entries(store.artist_feedback)) add(a, p > 0 ? 8 : -12, p > 0 ? "more" : "less");

  const hidden = new Set((lib.hidden_artists ?? []).map((h) => h.trim().toLowerCase()));
  for (const h of hidden) weights.delete(h);

  const pos = [...weights.entries()].filter(([, v]) => v.w > 0).sort((a, b) => b[1].w - a[1].w);
  const maxW = Math.max(1, pos[0]?.[1].w ?? 1);

  const genreRaw = new Map<string, number>();
  const moodRaw = new Map<string, number>();
  for (const g of store.onboarding_genres) genreRaw.set(g, (genreRaw.get(g) ?? 0) + 1.2);
  for (const [k, v] of weights) {
    const normAff = v.w >= 0 ? v.w / maxW : Math.max(-1, v.w / 6);
    const meta = store.artist_tags[k];
    const gs = meta?.genres?.length ? meta.genres : lookupBuiltinGenres(k);
    const ms = meta?.moods?.length ? meta.moods : gs.flatMap((g) => GENRE_MOODS[g] ?? []);
    gs.forEach((g, i) => genreRaw.set(g, (genreRaw.get(g) ?? 0) + normAff / (1 + i * 0.35)));
    ms.forEach((m, i) => moodRaw.set(m, (moodRaw.get(m) ?? 0) + normAff / (1 + i * 0.35)));
  }
  for (const [tag, pref] of Object.entries(store.tag_feedback)) {
    const delta = pref > 0 ? 2.5 : -3.5;
    let hit = false;
    for (const [g, cur] of genreRaw) {
      if (g.toLowerCase() === tag) { genreRaw.set(g, cur + delta); hit = true; }
    }
    for (const [m, cur] of moodRaw) {
      if (m.toLowerCase() === tag) { moodRaw.set(m, cur + delta); hit = true; }
    }
    if (!hit) {
      const lbl = tag.split(" ").map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join(" ");
      genreRaw.set(lbl, (genreRaw.get(lbl) ?? 0) + delta);
    }
  }

  const posGenres = [...genreRaw.entries()].filter(([, v]) => v > 0.05).sort((a, b) => b[1] - a[1]).slice(0, 10);
  const maxG = Math.max(1, posGenres[0]?.[1] ?? 1);
  const sumG = Math.max(0.001, posGenres.reduce((s, [, v]) => s + v, 0));
  const top_genres: WeightedTag[] = posGenres.map(([name, v]) => ({
    name,
    score: Math.min(1, v / maxG),
    share_pct: Math.max(1, Math.round((v / sumG) * 100)),
    feedback: store.tag_feedback[name.toLowerCase()] ?? 0,
  }));

  const avoided_genres: WeightedTag[] = [...genreRaw.entries()]
    .filter(([, v]) => v <= -0.2)
    .sort((a, b) => a[1] - b[1])
    .slice(0, 8)
    .map(([name, v]) => ({
      name,
      score: Math.min(1, Math.abs(v) / 3),
      share_pct: Math.min(100, Math.round((Math.abs(v) / 3) * 100)),
      feedback: store.tag_feedback[name.toLowerCase()] ?? 0,
    }));

  const posMoods = [...moodRaw.entries()].filter(([, v]) => v > 0.05).sort((a, b) => b[1] - a[1]).slice(0, 8);
  const maxM = Math.max(1, posMoods[0]?.[1] ?? 1);
  const sumM = Math.max(0.001, posMoods.reduce((s, [, v]) => s + v, 0));
  const moods: WeightedTag[] = posMoods.map(([name, v]) => ({
    name,
    score: Math.min(1, v / maxM),
    share_pct: Math.max(1, Math.round((v / sumM) * 100)),
    feedback: store.tag_feedback[name.toLowerCase()] ?? 0,
  }));

  const totalPlays = Object.values(store.artist_plays).reduce((a, b) => a + b, 0) + lib.recent.filter((r) => r.kind === "song").length;
  const totalSkips = Object.values(store.artist_skips).reduce((a, b) => a + b, 0);
  const outcomes = totalPlays + totalSkips;
  const bSum = store.time_buckets.reduce((a, b) => a + b, 0);
  const time_of_day: [number, number, number, number] = bSum > 0
    ? [
        Math.round((store.time_buckets[0] / bSum) * 100),
        Math.round((store.time_buckets[1] / bSum) * 100),
        Math.round((store.time_buckets[2] / bSum) * 100),
        Math.round((store.time_buckets[3] / bSum) * 100),
      ]
    : [15, 25, 35, 25];
  const peakIdx = time_of_day.indexOf(Math.max(...time_of_day));
  const peak_period = (["morning", "afternoon", "evening", "night"] as const)[peakIdx] ?? timeBucketOf();

  const versions: VersionInsight[] = VERSION_STYLES.map((style) => {
    const fb = store.version_feedback[style] ?? 0;
    const plays = store.version_plays[style] ?? 0;
    const skips = store.version_skips[style] ?? 0;
    const score = fb !== 0 ? (fb > 0 ? 0.85 : -0.95) : plays + skips > 0 ? (plays - skips) / (plays + skips + 2) : DEFAULT_ODD.has(style) ? -0.55 : 0;
    return { style, score, plays, skips, feedback: fb };
  });

  const liked_artists = pos.slice(0, 12).map(([k, v]) => {
    const tagged = store.artist_tags[k]?.genres;
    const gs = tagged?.length ? tagged : lookupBuiltinGenres(k);
    return {
      name: v.name,
      score: Math.min(1, v.w / maxW),
      reasons: [...v.reasons],
      genres: gs.slice(0, 2),
    };
  });

  const disliked_artists: TasteProfile["disliked_artists"] = [];
  const seenDis = new Set<string>();
  for (const h of lib.hidden_artists ?? []) {
    const k = h.trim().toLowerCase();
    if (k && !seenDis.has(k)) {
      seenDis.add(k);
      disliked_artists.push({ name: h, kind: "hidden", skips: store.artist_skips[k] ?? 0 });
    }
  }
  for (const [a, pref] of Object.entries(store.artist_feedback)) {
    const k = a.trim().toLowerCase();
    if (pref < 0 && k && !seenDis.has(k)) {
      seenDis.add(k);
      disliked_artists.push({ name: a, kind: pref <= -2 ? "blocked" : "less", skips: store.artist_skips[k] ?? 0 });
    }
  }
  for (const [k, skips] of Object.entries(store.artist_skips)) {
    if (skips >= 2 && (store.artist_plays[k] ?? 0) <= skips && !seenDis.has(k)) {
      seenDis.add(k);
      disliked_artists.push({ name: k.split(" ").map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join(" "), kind: "high_skips", skips });
    }
  }

  const signals_count = lib.liked.length + lib.followed.length + store.onboarding_artists.length + store.onboarding_tracks.length + Object.keys(store.disliked_tracks).length + totalPlays + totalSkips;
  const variety = pos.length <= 1 ? 0 : Math.min(1, Math.log(pos.length) / Math.log(12));

  return {
    mode,
    confidence: Math.min(1, 1 - Math.exp(-signals_count / 18)),
    signals_count,
    onboarding_done: store.onboarding_done,
    top_genres,
    avoided_genres,
    moods,
    habits: {
      time_of_day,
      peak_period,
      completion_rate: outcomes > 0 ? Math.max(0, Math.min(1, totalPlays / outcomes)) : 0,
      early_skip_rate: outcomes > 0 ? Math.max(0, Math.min(1, totalSkips / outcomes)) : 0,
      variety_score: variety,
      total_plays: totalPlays,
      total_skips: totalSkips,
      versions,
    },
    liked_artists,
    disliked_artists,
    disliked_tracks: Object.values(store.disliked_tracks),
    onboarding_artists: store.onboarding_artists,
    onboarding_tracks: store.onboarding_tracks,
    onboarding_genres: store.onboarding_genres,
    collab_recs: mode === "hybrid" ? store.collab_recs : [],
    collab_contributors: mode === "hybrid" ? store.collab_contributors : 0,
    collab_model_ready: mode === "hybrid" ? store.collab_model_ready : false,
  };
}

export function buildLocalDailyMixes(
  lib: Library,
  store: LocalTasteStore,
  candidates: Track[],
  cfg: TasteConfig,
): DailyMix[] {
  if (!cfg.enabled) return [];
  const hidden = new Set((lib.hidden_artists ?? []).map((h) => h.trim().toLowerCase()));
  const disliked = new Set(Object.keys(store.disliked_tracks));
  const ok = (t: Track) => !hidden.has(primaryArtist(t.artist).toLowerCase()) && !disliked.has(trackKey(t));

  const fam: Track[] = [];
  const seen = new Set<string>();
  for (const t of [...lib.liked, ...store.onboarding_tracks, ...lib.playlists.flatMap((p) => p.tracks)]) {
    const k = trackKey(t);
    if (ok(t) && !seen.has(k)) { seen.add(k); fam.push(t); }
  }
  const fresh: Track[] = [];
  for (const t of candidates) {
    const k = trackKey(t);
    if (ok(t) && !seen.has(k)) { seen.add(k); fresh.push(t); }
  }
  const all = [...fam, ...fresh];
  if (all.length < 3) return [];

  const spread = (list: Track[], lim = 16) => {
    const out: Track[] = [];
    const rest = [...list];
    while (rest.length && out.length < lim) {
      const last = out.length ? primaryArtist(out[out.length - 1].artist).toLowerCase() : "";
      const idx = rest.findIndex((t) => primaryArtist(t.artist).toLowerCase() !== last);
      if (idx < 0) break;
      out.push(rest.splice(idx, 1)[0]);
    }
    return out;
  };

  const subtitle = (list: Track[], lead?: string) => {
    const names = [...new Set(list.map((t) => primaryArtist(t.artist)))].slice(0, 3).join(", ");
    return lead && names ? `${lead} · ${names}` : lead || names;
  };

  const mixes: DailyMix[] = [];
  const prof = computeTasteProfile(lib, store, cfg, false);

  // Up to 2 genre mixes
  const used = new Set<string>();
  let idx = 0;
  for (const g of prof.top_genres.slice(0, 4)) {
    if (idx >= 2) break;
    const match = all.filter((t) => {
      if (used.has(trackKey(t))) return false;
      const ak = primaryArtist(t.artist).toLowerCase();
      const tagged = store.artist_tags[ak]?.genres;
      const gs = tagged?.length ? tagged : lookupBuiltinGenres(ak);
      return gs.some((x) => x.toLowerCase() === g.name.toLowerCase());
    });
    const tracks = spread(match, 16);
    if (tracks.length >= 3) {
      tracks.forEach((t) => used.add(trackKey(t)));
      idx++;
      mixes.push({
        id: `daily-genre-${idx}`,
        kind: "genre",
        title: `Daily Mix ${idx}`,
        subtitle: subtitle(tracks, g.name),
        tags: [g.name],
        accent_hue: (g.name.length * 47 + idx * 71) % 360,
        tracks,
      });
    }
  }

  if (!mixes.length) {
    const tracks = spread(all, 16);
    if (tracks.length >= 2) {
      mixes.push({
        id: "daily-genre-1",
        kind: "genre",
        title: "Daily Mix 1",
        subtitle: subtitle(tracks),
        tags: [],
        accent_hue: 328,
        tracks,
      });
    }
  }

  const bucket = timeBucketOf();
  const timeTitles = { morning: "Morning Mix", afternoon: "Afternoon Mix", evening: "Evening Mix", night: "Late Night Mix" };
  const timeTracks = spread(all.slice().reverse(), 16);
  if (timeTracks.length >= 3) {
    mixes.push({
      id: `daily-time-${bucket}`,
      kind: "time",
      title: timeTitles[bucket],
      subtitle: subtitle(timeTracks),
      tags: bucket === "night" ? ["Late Night", "Chill"] : bucket === "morning" ? ["Warm", "Upbeat"] : ["Groovy"],
      accent_hue: bucket === "night" ? 242 : bucket === "morning" ? 36 : 278,
      tracks: timeTracks,
    });
  }

  const discTracks = spread(fresh.length >= 3 ? fresh : all.slice(2), 16);
  if (discTracks.length >= 3) {
    mixes.push({
      id: "daily-discovery",
      kind: "discovery",
      title: "Discovery Mix",
      subtitle: subtitle(discTracks),
      tags: ["Fresh"],
      accent_hue: 325,
      tracks: discTracks,
    });
  }

  return mixes;
}
