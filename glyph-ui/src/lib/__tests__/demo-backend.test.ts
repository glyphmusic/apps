import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { DemoBackend } from "../api/demo";
import type { Library, QueueState } from "../api/types";

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
});
