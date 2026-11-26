import { describe, expect, it } from "vitest";
import { registerListNav, currentListNav, stepRowIndex } from "../state/list-nav.svelte";
import { compact, fmtMs, fmtTime, fmtTotal, listMeta, uniqBy } from "../fmt";
import { listId, primaryArtist, rating, sameList, splitArtists, trackKey, type Track } from "../api/types";
import { filterTrackByEnabled } from "../settings/sources";
import { pruneTrackedKeys, sameTrackKeys } from "../state/cache";

describe("list keyboard navigation", () => {
  it("steps row index down/up with clean clamping", () => {
    expect(stepRowIndex(null, 1, 5)).toBe(0);
    expect(stepRowIndex(null, -1, 5)).toBe(4);
    expect(stepRowIndex(0, -1, 5)).toBe(0);
    expect(stepRowIndex(2, 1, 5)).toBe(3);
    expect(stepRowIndex(4, 1, 5)).toBe(4);
    expect(stepRowIndex(0, 1, 0)).toBeNull();
  });

  it("registers and unregisters active list handlers cleanly", () => {
    const h = { count: () => 3, play: () => {} };
    const off = registerListNav(h);
    expect(currentListNav()).toBe(h);
    off();
    expect(currentListNav()).toBeNull();
  });
});

describe("formatters, wire types, and cache helpers", () => {
  it("formats durations and metadata cleanly", () => {
    expect(fmtTime(0)).toBe("0:00");
    expect(fmtTime(185)).toBe("3:05");
    expect(fmtTime(-5)).toBe("0:00");
    expect(fmtMs(null)).toBe("-:--");
    expect(fmtMs(65_000)).toBe("1:05");
    expect(fmtTotal(3_661_000)).toBe("1:01:01");
    expect(listMeta(1, 180_000, "Song", "1 Songs")).toBe("1 Song · 3:00");
    expect(compact(1_250_000)).toBe("1.3M");
    expect(compact(2_400)).toBe("2.4K");
  });

  it("deduplicates and identifies tracks and lists", () => {
    const a: Track = { id: "1", source: "deezer", title: "  Space Song ", artist: "Beach House" };
    const b: Track = { id: "2", source: "youtube", title: "space song", artist: "beach house" };
    expect(trackKey(a)).toBe(trackKey(b));
    expect(sameTrackKeys([a], [b])).toBe(true);
    expect(uniqBy([a, b], trackKey)).toHaveLength(1);
    expect(splitArtists("A, B feat. C & B")).toEqual(["A", "B", "C"]);
    expect(primaryArtist("Daft Punk feat. Pharrell Williams")).toBe("Daft Punk");
    expect(sameList({ kind: "playlist", id: "pl-1" }, { kind: "playlist", id: "pl-1" })).toBe(true);
    expect(listId({ kind: "liked" })).toBe("liked");
  });

  it("prunes oldest tracked cache keys beyond limit", () => {
    const asked = new Set(["a", "b", "c", "d"]);
    const dict: Record<string, unknown> = { a: 1, b: 2, c: 3, d: 4 };
    pruneTrackedKeys(asked, dict, 2);
    expect([...asked]).toEqual(["c", "d"]);
    expect(Object.keys(dict)).toEqual(["c", "d"]);
  });

  it("evaluates tri-state explicit ratings and preserves them across source filtering", () => {
    const yt: Track = { id: "v1", source: "youtube", title: "Song", artist: "Artist" };
    const sc: Track = { id: "s1", source: "soundcloud", title: "Song", artist: "Artist" };
    const dzUnknown: Track = { id: "d1", source: "deezer", title: "Song", artist: "Artist", explicit_rating: "unknown" };
    const ytClean: Track = { id: "v2", source: "youtube", title: "Song", artist: "Artist", explicit_rating: "clean" };
    const ytExp: Track = { id: "v3", source: "youtube", title: "Song", artist: "Artist", explicit: true };
    expect(rating(yt)).toBe("unknown");
    expect(rating(sc)).toBe("unknown");
    expect(rating(dzUnknown)).toBe("unknown");
    expect(rating(ytClean)).toBe("clean");
    expect(rating(ytExp)).toBe("explicit");

    const dzRatedYt: Track = { id: "v4", source: "youtube", title: "Song", artist: "Artist", external_ids: { deezer: "99" } };
    const filtered = filterTrackByEnabled(dzRatedYt, ["youtube"]);
    expect(filtered).not.toBeNull();
    expect(rating(filtered!)).toBe("clean");
  });
});
