import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { DemoBackend } from "../api/demo";
import type { MatchAlternativesResponse, Track, WrongMatchReportResponse, WrongSongPreviewState } from "../api/types";
import { trackKey } from "../api/types";

describe("wrong song flow", () => {
  const prevWindow = (globalThis as { window?: unknown }).window;
  beforeAll(() => {
    if (typeof (globalThis as { window?: unknown }).window === "undefined") {
      (globalThis as { window?: unknown }).window = globalThis;
    }
  });
  afterAll(() => {
    if (prevWindow === undefined) delete (globalThis as { window?: unknown }).window;
  });

  it("fetches alternatives, runs 30s preview, saves vote without premature global promotion, and re-downloads", async () => {
    const api = new DemoBackend();
    try {
      const snap = await api.snapshot();
      const song: Track = snap.library.liked[0];
      const alts = await api.call<MatchAlternativesResponse>("wrong_song_alternatives", {
        track: song,
        query: null,
      });
      expect(alts.alternatives.length).toBeGreaterThanOrEqual(3);

      const events: WrongSongPreviewState[] = [];
      const off = api.on("wrong_song_preview", (p) => events.push(p));
      const picked = alts.alternatives.find((a) => a.tag !== "current") ?? alts.alternatives[0];

      await api.call("wrong_song_preview", { track: picked.track, preview_url: picked.preview_url ?? null });
      expect(events.some((e) => e.status === "playing" && e.duration === 30)).toBe(true);

      await api.call("wrong_song_preview_stop");
      expect(events[events.length - 1].status).toBe("stopped");
      off();

      const dlKeys: string[] = [];
      const offDl = api.on("download", (d) => dlKeys.push(d.key));

      // First vote: recorded, NOT promoted globally yet!
      const r1 = await api.call<WrongMatchReportResponse>("wrong_song_select", {
        original: song,
        chosen: picked.track,
        wrong_provider: alts.current_source ?? null,
        wrong_id: alts.current_id ?? null,
      });
      expect(r1.recorded).toBe(true);
      expect(r1.promoted).toBe(false);
      expect(r1.votes).toBe(1);
      expect(r1.threshold).toBe(3);

      // Selecting triggers downloading the chosen version right away.
      for (let i = 0; i < 20 && !dlKeys.includes(trackKey(song)); i++) {
        await new Promise((r) => setTimeout(r, 100));
      }
      expect(dlKeys.includes(trackKey(song))).toBe(true);
      offDl();
    } finally {
      api.destroy();
    }
  });
});
