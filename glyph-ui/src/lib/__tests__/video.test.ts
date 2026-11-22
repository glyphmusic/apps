import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { DemoBackend } from "../api/demo";
import type { VideoResolveResponse, VideoSearchResponse } from "../api/types";

describe("music video demo backend", () => {
  const prevWindow = (globalThis as { window?: unknown }).window;
  beforeAll(() => {
    if (typeof (globalThis as { window?: unknown }).window === "undefined") {
      (globalThis as { window?: unknown }).window = globalThis;
    }
  });
  afterAll(() => {
    if (prevWindow === undefined) delete (globalThis as { window?: unknown }).window;
  });

  it("searches videos for a song with unique IDs, supports custom queries, and resolves playable qualities", async () => {
    const api = new DemoBackend();
    const snap = await api.snapshot();
    const track = snap.player.current!;
    expect(track).toBeTruthy();

    const search = await api.call<VideoSearchResponse>("video_search", { track });
    expect(search.results.length).toBeGreaterThanOrEqual(4);
    expect(new Set(search.results.map((v) => v.id)).size).toBe(search.results.length);
    expect(search.results[0].kind).toBe("official");
    expect(search.results.some((v) => v.kind === "live")).toBe(true);
    expect(search.results.some((v) => v.kind === "lyric")).toBe(true);

    const custom = await api.call<VideoSearchResponse>("video_search", { track, query: "live acoustic" });
    expect(new Set(custom.results.map((v) => v.id)).size).toBe(custom.results.length);
    expect(custom.results[0].title.toLowerCase()).toContain("live acoustic");

    const resolved = await api.call<VideoResolveResponse>("video_resolve", { id: search.results[0].id });
    expect(resolved.id).toBe(search.results[0].id);
    expect(resolved.url).toContain("demo://video/");
    expect(resolved.qualities?.map((q) => q.label)).toEqual(["1080p", "720p", "360p"]);
  });
});
