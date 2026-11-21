import { describe, expect, it } from "vitest";
import { filterSearchHistory } from "../pages/search/history-filter";
import type { ListKey, SearchItem, Track } from "../api/types";

const tr = (id: string, title: string, artist: string, album?: string): Track => ({
  id,
  source: "deezer",
  title,
  artist,
  album,
});

describe("recent search history filtering", () => {
  const history: SearchItem[] = [
    { kind: "query", text: "foo fighters" },
    { kind: "song", track: tr("1", "Everlong", "Foo Fighters", "The Colour and the Shape") },
    { kind: "artist", name: "mgk", track: null },
    { kind: "song", track: tr("2", "Bad Things", "mgk", "Bloom") },
  ];
  const recentLists: ListKey[] = [
    { kind: "playlist", id: "pl-1" },
    { kind: "album", title: "Currents", artist: "Tame Impala", tracks: [tr("3", "Let It Happen", "Tame Impala", "Currents")] },
  ];

  it("keeps all history entries with original indices under 'all'", () => {
    const out = filterSearchHistory(history, recentLists, "all");
    expect(out).toHaveLength(4);
    expect(out.map((e) => (e.kind === "history" ? e.origIndex : -1))).toEqual([0, 1, 2, 3]);
  });

  it("filters to songs only under 'tracks' and preserves origIndex for deletion", () => {
    const out = filterSearchHistory(history, recentLists, "tracks");
    expect(out).toHaveLength(2);
    expect(out.map((e) => (e.kind === "history" ? e.origIndex : -1))).toEqual([1, 3]);
  });

  it("filters to artists only under 'artists' and preserves origIndex", () => {
    const out = filterSearchHistory(history, recentLists, "artists");
    expect(out).toHaveLength(1);
    expect(out[0]).toMatchObject({ kind: "history", origIndex: 2, item: { kind: "artist", name: "mgk" } });
  });

  it("surfaces recent opened albums and song albums deduplicated under 'albums'", () => {
    const out = filterSearchHistory(history, recentLists, "albums");
    expect(out.every((e) => e.kind === "album")).toBe(true);
    expect(out.map((e) => (e.kind === "album" ? e.list.title : ""))).toEqual([
      "Currents",
      "The Colour and the Shape",
      "Bloom",
    ]);
  });
});
