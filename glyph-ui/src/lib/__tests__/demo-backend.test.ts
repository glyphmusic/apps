import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { DemoBackend } from "../api/demo";
import { trackKey, type Library, type QueueState } from "../api/types";

describe("demo backend core flows", () => {
  const prevWindow = (globalThis as { window?: unknown }).window;
  beforeAll(() => {
    if (typeof (globalThis as { window?: unknown }).window === "undefined") {
      (globalThis as { window?: unknown }).window = globalThis;
    }
  });
  afterAll(() => {
    if (prevWindow === undefined) delete (globalThis as { window?: unknown }).window;
  });

  it("loads snapshot, creates playlists, adds/removes tracks and updates queue", async () => {
    const api = new DemoBackend();
    try {
      let latestLib: Library | null = null;
      let latestQueue: QueueState | null = null;
      api.on("library", (p) => (latestLib = p as Library));
      api.on("queue", (p) => (latestQueue = p as QueueState));

      const snap = await api.snapshot();
      expect(snap.library.playlists.length).toBeGreaterThan(0);
      const sampleTracks = snap.library.liked.slice(0, 2);
      expect(sampleTracks.length).toBe(2);

      const id = await api.call<string>("playlist_create", { name: "Test Mix", tracks: sampleTracks });
      expect(typeof id).toBe("string");
      expect(latestLib).not.toBeNull();
      const created = latestLib!.playlists.find((p) => p.id === id);
      expect(created?.name).toBe("Test Mix");
      expect(created?.tracks.length).toBe(2);

      await api.call("playlist_move", { id, from: 0, to: 1 });
      expect(latestLib!.playlists.find((p) => p.id === id)?.tracks[0].id).toBe(sampleTracks[1].id);

      await api.call("playlist_remove_many", { id, indices: [0] });
      expect(latestLib!.playlists.find((p) => p.id === id)?.tracks.length).toBe(1);

      await api.call("playlist_delete", { id });
      expect(latestLib!.playlists.some((p) => p.id === id)).toBe(false);
      await api.call("playlist_restore", { id });
      expect(latestLib!.playlists.some((p) => p.id === id)).toBe(true);

      await api.call("queue_add", { tracks: sampleTracks, next: false });
      expect(latestQueue).not.toBeNull();
      expect(latestQueue!.items.length).toBeGreaterThanOrEqual(2);
    } finally {
      api.destroy();
    }
  });

  it("cancels individual and playlist downloads without completing cancelled tracks", async () => {
    const api = new DemoBackend();
    try {
      const snap = await api.snapshot();
      const downloaded = new Set(snap.library.downloads.map(trackKey));
      const candidates = snap.library.liked.filter((t) => !downloaded.has(trackKey(t))).slice(0, 3);
      expect(candidates.length).toBe(3);

      const events: { key: string; done: boolean }[] = [];
      api.on("download", (e) => events.push({ key: e.key, done: e.done }));

      vi.useFakeTimers();
      const start = api.call("download", { tracks: candidates });
      await vi.advanceTimersByTimeAsync(350);
      await start;

      // Cancel one individual song while the rest keep going.
      const cancelOne = api.call("download_cancel", { tracks: [candidates[0]] });
      await vi.advanceTimersByTimeAsync(200);
      await cancelOne;
      expect(events.some((e) => e.key === trackKey(candidates[0]) && e.done)).toBe(true);

      // Cancel a second song as part of a batch, leaving candidates[2] to finish.
      const cancelBatch = api.call("download_cancel", { tracks: [candidates[1]] });
      await vi.advanceTimersByTimeAsync(200);
      await cancelBatch;

      await vi.advanceTimersByTimeAsync(2500);
      vi.useRealTimers();

      const after = await api.snapshot();
      const afterKeys = new Set(after.library.downloads.map(trackKey));
      expect(afterKeys.has(trackKey(candidates[0]))).toBe(false);
      expect(afterKeys.has(trackKey(candidates[1]))).toBe(false);
      expect(afterKeys.has(trackKey(candidates[2]))).toBe(true);
    } finally {
      vi.useRealTimers();
      api.destroy();
    }
  });
});
