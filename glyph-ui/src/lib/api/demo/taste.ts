// Demo backend handlers for the hybrid taste engine, onboarding catalog,
// Daily Mixes, and interactive Taste Profile feedback.
import type { DemoCtx, DemoHandler } from "../demo";
import type { TasteConfig, Track } from "../types";
import { trackKey } from "../types";
import { TRACKS } from "../demo-data";
import {
  buildLocalDailyMixes,
  computeTasteProfile,
  emptyTasteStore,
  fallbackOnboardingCatalog,
  type LocalTasteStore,
} from "../web-taste";

const stores = new WeakMap<DemoCtx, LocalTasteStore>();

function storeOf(ctx: DemoCtx): LocalTasteStore {
  let s = stores.get(ctx);
  if (!s) {
    s = emptyTasteStore();
    s.artist_plays = { "tame impala": 8, "men i trust": 6, gorillaz: 5, "daft punk": 4 };
    s.artist_replays = { "tame impala": 2 };
    s.artist_skips = { "skip demo": 3 };
    s.time_buckets = [12, 24, 38, 26];
    s.version_plays = { acoustic: 4 };
    s.version_skips = { nightcore: 3, karaoke: 2 };
    s.collab_recs = [
      { artist: "Pond", score: 0.92, because_of: ["Tame Impala", "Men I Trust"], genres: ["Psychedelic", "Indie"], collaborative: true },
      { artist: "Khruangbin", score: 0.84, because_of: ["Tame Impala", "Gorillaz"], genres: ["Psychedelic", "Funk"], collaborative: true },
      { artist: "Crumb", score: 0.78, because_of: ["Men I Trust"], genres: ["Psychedelic", "Indie"], collaborative: true },
    ];
    s.collab_contributors = 42;
    s.collab_model_ready = true;
    stores.set(ctx, s);
  }
  return s;
}

function tasteCfg(ctx: DemoCtx): TasteConfig {
  return ctx.config.taste ?? { enabled: true, server_collab: true, metadata_enrichment: true };
}

export function demoMixes(ctx: DemoCtx) {
  return buildLocalDailyMixes(ctx.library, storeOf(ctx), TRACKS, tasteCfg(ctx));
}

export const handlers: Record<string, DemoHandler> = {
  taste_profile: (ctx) => computeTasteProfile(ctx.library, storeOf(ctx), tasteCfg(ctx), true),
  taste_mixes: (ctx) => demoMixes(ctx),
  taste_refresh: (ctx) => computeTasteProfile(ctx.library, storeOf(ctx), tasteCfg(ctx), true),
  taste_onboarding_catalog: () => fallbackOnboardingCatalog(TRACKS),
  taste_onboarding_save: (ctx, a) => {
    const s = storeOf(ctx);
    s.onboarding_artists = Array.isArray(a.artists) ? a.artists.map(String) : [];
    s.onboarding_tracks = Array.isArray(a.tracks) ? (a.tracks as Track[]) : [];
    s.onboarding_genres = Array.isArray(a.genres) ? a.genres.map(String) : [];
    s.onboarding_done = true;
    if (a.follow_artists === true) {
      for (const name of s.onboarding_artists) {
        if (!ctx.library.followed.some((f) => f.toLowerCase() === name.toLowerCase())) {
          ctx.library.followed = [name, ...ctx.library.followed];
        }
      }
      ctx.saveLibrary();
    }
    return computeTasteProfile(ctx.library, s, tasteCfg(ctx), true);
  },
  taste_dislike_track: (ctx, a) => {
    const s = storeOf(ctx);
    const tr = a.track as Track;
    const k = trackKey(tr);
    if (a.disliked) s.disliked_tracks[k] = tr;
    else delete s.disliked_tracks[k];
    ctx.library.disliked_tracks = Object.keys(s.disliked_tracks);
    ctx.saveLibrary();
    return computeTasteProfile(ctx.library, s, tasteCfg(ctx), true);
  },
  taste_tag_feedback: (ctx, a) => {
    const s = storeOf(ctx);
    const k = String(a.tag ?? "").trim().toLowerCase();
    if (k) {
      if (a.preference == null || a.preference === 0) delete s.tag_feedback[k];
      else s.tag_feedback[k] = Math.max(-1, Math.min(1, Number(a.preference)));
    }
    return computeTasteProfile(ctx.library, s, tasteCfg(ctx), true);
  },
  taste_version_feedback: (ctx, a) => {
    const s = storeOf(ctx);
    const k = String(a.style ?? "").trim().toLowerCase();
    if (k) {
      if (a.preference == null || a.preference === 0) delete s.version_feedback[k];
      else s.version_feedback[k] = Math.max(-1, Math.min(1, Number(a.preference)));
    }
    return computeTasteProfile(ctx.library, s, tasteCfg(ctx), true);
  },
  taste_reset: (ctx) => {
    const fresh = emptyTasteStore();
    stores.set(ctx, fresh);
    ctx.library.disliked_tracks = [];
    ctx.saveLibrary();
    return computeTasteProfile(ctx.library, fresh, tasteCfg(ctx), true);
  },
  recommend_feedback: (ctx, a) => {
    const s = storeOf(ctx);
    const k = String(a.artist ?? "").trim().toLowerCase();
    if (k) {
      if (a.preference == null) {
        delete s.artist_feedback[k];
        delete s.artist_skips[k];
      } else {
        s.artist_feedback[k] = Number(a.preference);
      }
    }
  },
};
