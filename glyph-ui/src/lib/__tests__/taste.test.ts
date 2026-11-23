import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { DemoBackend } from "../api/demo";
import { TRACKS } from "../api/demo-data";
import {
  buildLocalDailyMixes,
  buildServerRecommendPayload,
  computeTasteProfile,
  detectVersions,
  emptyTasteStore,
  inferWebIntent,
  recordWebSessionEvent,
} from "../api/web-taste";
import type { OnboardingCatalogResponse, TasteProfile, DailyMix, HomeFeeds } from "../api/types";
import { primaryArtist, trackKey } from "../api/types";

describe("taste engine (web & demo)", () => {
  const prevWindow = (globalThis as { window?: unknown }).window;
  beforeAll(() => {
    if (typeof (globalThis as { window?: unknown }).window === "undefined") {
      (globalThis as { window?: unknown }).window = globalThis;
    }
  });
  afterAll(() => {
    if (prevWindow === undefined) delete (globalThis as { window?: unknown }).window;
  });
  it("detects version styles with strict word boundaries", () => {
    expect(detectVersions("Discover")).toEqual([]);
    expect(detectVersions("Alive")).toEqual([]);
    expect(detectVersions("Undercover Martyn")).toEqual([]);
    expect(detectVersions("Song (Live at Red Rocks)")).toContain("live");
    expect(detectVersions("Song - Sped Up")).toContain("sped_up");
    expect(detectVersions("Song (Slowed + Reverb)")).toContain("slowed");
    expect(detectVersions("Song - Acoustic Remix")).toEqual(expect.arrayContaining(["acoustic", "remix"]));
    expect(detectVersions("Song (Karaoke Version)")).toContain("karaoke");
  });

  it("respects server_collab opt-out and taste off mode in computeTasteProfile", () => {
    const backend = new DemoBackend();
    const store = emptyTasteStore();
    store.collab_recs = [
      { artist: "Pond", score: 0.9, because_of: ["Tame Impala"], genres: ["Psychedelic"], collaborative: true },
    ];
    store.collab_contributors = 15;
    store.collab_model_ready = true;

    const serverMode = computeTasteProfile(backend.library, store, { enabled: true, server_collab: true, metadata_enrichment: true }, true);
    expect(serverMode.mode).toBe("server");
    expect(serverMode.collab_recs).toHaveLength(1);
    expect(serverMode.top_genres.length).toBeGreaterThan(0);
    expect(serverMode.intent).toBeDefined();

    const localOnly = computeTasteProfile(backend.library, store, { enabled: true, server_collab: false, metadata_enrichment: true }, true);
    expect(localOnly.mode).toBe("local");
    expect(localOnly.collab_recs).toEqual([]);
    expect(localOnly.intent).toBeDefined();

    const off = computeTasteProfile(backend.library, store, { enabled: false, server_collab: true, metadata_enrichment: true }, true);
    expect(off.mode).toBe("off");
    expect(off.collab_recs).toEqual([]);
  });

  it("builds Daily Mixes without hidden artists, disliked tracks, or consecutive same-artist tracks", () => {
    const backend = new DemoBackend();
    const store = emptyTasteStore();
    const hated = TRACKS[0];
    store.disliked_tracks[trackKey(hated)] = hated;
    backend.library.hidden_artists = ["Gorillaz"];

    const mixes = buildLocalDailyMixes(backend.library, store, TRACKS, {
      enabled: true,
      server_collab: true,
      metadata_enrichment: true,
    });
    expect(mixes.length).toBeGreaterThanOrEqual(2);
    for (const mix of mixes) {
      expect(mix.tracks.some((t) => trackKey(t) === trackKey(hated))).toBe(false);
      expect(mix.tracks.some((t) => primaryArtist(t.artist).toLowerCase() === "gorillaz")).toBe(false);
      for (let i = 1; i < mix.tracks.length; i++) {
        expect(primaryArtist(mix.tracks[i].artist).toLowerCase()).not.toBe(
          primaryArtist(mix.tracks[i - 1].artist).toLowerCase(),
        );
      }
    }

    const disabled = buildLocalDailyMixes(backend.library, store, TRACKS, {
      enabled: false,
      server_collab: false,
      metadata_enrichment: false,
    });
    expect(disabled).toEqual([]);
  });

  it("runs onboarding, profile feedback, track dislike, mixes, and reset via DemoBackend", async () => {
    const backend = new DemoBackend();

    const catalog = await backend.call<OnboardingCatalogResponse>("taste_onboarding_catalog");
    expect(catalog.artists.length).toBeGreaterThan(15);
    expect(catalog.genres.length).toBeGreaterThan(5);

    const saved = await backend.call<TasteProfile>("taste_onboarding_save", {
      artists: ["Daft Punk", "Miles Davis", "Radiohead"],
      tracks: [TRACKS[1]],
      genres: ["Electronic", "Jazz"],
      follow_artists: false,
    });
    expect(saved.onboarding_done).toBe(true);
    expect(saved.onboarding_artists).toEqual(["Daft Punk", "Miles Davis", "Radiohead"]);
    expect(saved.liked_artists.some((a) => a.name.toLowerCase() === "daft punk")).toBe(true);

    const disliked = await backend.call<TasteProfile>("taste_dislike_track", {
      track: TRACKS[2],
      disliked: true,
    });
    expect(disliked.disliked_tracks.some((t) => trackKey(t) === trackKey(TRACKS[2]))).toBe(true);

    const tagDown = await backend.call<TasteProfile>("taste_tag_feedback", {
      tag: "Metal",
      preference: -1,
    });
    expect(tagDown.avoided_genres.some((g) => g.name.toLowerCase() === "metal")).toBe(true);

    const verPrefer = await backend.call<TasteProfile>("taste_version_feedback", {
      style: "acoustic",
      preference: 1,
    });
    expect(verPrefer.habits.versions.find((v) => v.style === "acoustic")?.feedback).toBe(1);

    const home = await backend.call<HomeFeeds>("home");
    expect(home.mixes && home.mixes.length).toBeGreaterThan(0);

    const mixes = await backend.call<DailyMix[]>("taste_mixes");
    expect(mixes.length).toBeGreaterThan(0);

    const reset = await backend.call<TasteProfile>("taste_reset");
    expect(reset.onboarding_done).toBe(false);
    expect(reset.disliked_tracks).toEqual([]);
  });

  it("keeps Tyler, The Creator intact, falls back to builtin genres on empty tag arrays, and clears skips on restore", async () => {
    expect(primaryArtist("Tyler, The Creator")).toBe("Tyler, The Creator");
    expect(primaryArtist("Tyler, The Creator feat. Kali Uchis")).toBe("Tyler, The Creator");

    const backend = new DemoBackend();
    const store = emptyTasteStore();
    store.onboarding_artists = ["Tyler, The Creator", "The Weeknd"];
    store.artist_tags["the weeknd"] = { genres: [], moods: [], similar: ["Bruno Mars"] };

    const prof = computeTasteProfile(backend.library, store, { enabled: true, server_collab: false, metadata_enrichment: true }, false);
    const tyler = prof.liked_artists.find((a) => a.name === "Tyler, The Creator");
    const weeknd = prof.liked_artists.find((a) => a.name === "The Weeknd");
    expect(tyler?.genres).toContain("Hip-Hop");
    expect(weeknd?.genres).toContain("Pop");

    // DemoBackend initially has "skip demo" in high_skips; restoring via recommend_feedback(null) clears it.
    const before = await backend.call<TasteProfile>("taste_profile");
    expect(before.disliked_artists.some((d) => d.name.toLowerCase() === "skip demo")).toBe(true);
    await backend.call("recommend_feedback", { artist: "Skip Demo", preference: null });
    const after = await backend.call<TasteProfile>("taste_profile");
    expect(after.disliked_artists.some((d) => d.name.toLowerCase() === "skip demo")).toBe(false);
  });

  it("infers real-time session intent (vibe reset, comfort, artist dive) and builds server recommend payload", () => {
    const backend = new DemoBackend();
    const store = emptyTasteStore();

    // Two consecutive early skips trigger vibe_reset + vibe_pivot
    recordWebSessionEvent(store, { title: "A", artist: "Tame Impala", outcome: "skipped_early", context: "radio" });
    recordWebSessionEvent(store, { title: "B", artist: "Gorillaz", outcome: "skipped_early", context: "radio" });
    const resetIntent = inferWebIntent(store, [], [], 0.5);
    expect(resetIntent.mode).toBe("vibe_reset");
    expect(resetIntent.vibe_pivot).toBe(true);

    // Replays trigger comfort mode
    const comfortStore = emptyTasteStore();
    recordWebSessionEvent(comfortStore, { title: "Let It Happen", artist: "Tame Impala", outcome: "replayed", context: "library" });
    recordWebSessionEvent(comfortStore, { title: "Let It Happen", artist: "Tame Impala", outcome: "replayed", context: "library" });
    const comfortIntent = inferWebIntent(comfortStore, [], [], 0.35);
    expect(comfortIntent.mode).toBe("comfort");
    expect(comfortIntent.vibe_pivot).toBe(false);

    // Transition pairs are recorded across distinct positive artists and included in server payload
    const pairStore = emptyTasteStore();
    recordWebSessionEvent(pairStore, { title: "Song 1", artist: "Daft Punk", outcome: "completed", context: "library" });
    recordWebSessionEvent(pairStore, { title: "Song 2", artist: "Justice", outcome: "completed", context: "library" });
    expect(pairStore.session_pairs).toEqual([["Daft Punk", "Justice"]]);

    const payload = buildServerRecommendPayload(
      backend.library,
      pairStore,
      { enabled: true, server_collab: true, metadata_enrichment: true },
      "home",
      [TRACKS[0]],
    );
    expect(payload.purpose).toBe("home");
    expect(Array.isArray(payload.positive_artists)).toBe(true);
    expect((payload.session as { recent_events: unknown[] }).recent_events.length).toBe(2);
    expect(payload.session_pairs).toEqual([["Daft Punk", "Justice"]]);
  });
});
