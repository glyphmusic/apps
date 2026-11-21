import { describe, expect, it } from "vitest";
import { filterAndSortTiles, filterArtists, type LibraryTile } from "../pages/lists/library-filter";

describe("library filter & sort", () => {
  const tiles: LibraryTile[] = [
    { l: { kind: "liked" }, kind: "Playlist", title: "Liked Songs", count: 42, pinRank: Infinity, origIndex: 0 },
    { l: { kind: "playlist", id: "1" }, kind: "Local", title: "Sunday Morning", count: 6, pinRank: 0, origIndex: 1 },
    { l: { kind: "playlist", id: "2" }, kind: "Local", title: "Ambient Focus", count: 19, pinRank: Infinity, origIndex: 2 },
    { l: { kind: "album", title: "Currents", artist: "Tame Impala", tracks: [] }, kind: "Album · Tame Impala", title: "Currents", count: 13, pinRank: 1, origIndex: 3 },
  ];

  it("sorts pinned first in default mode", () => {
    const out = filterAndSortTiles(tiles, "", "default");
    expect(out.map((x) => x.title)).toEqual(["Sunday Morning", "Currents", "Liked Songs", "Ambient Focus"]);
  });

  it("sorts alphabetically in name mode", () => {
    const out = filterAndSortTiles(tiles, "", "name");
    expect(out.map((x) => x.title)).toEqual(["Ambient Focus", "Currents", "Liked Songs", "Sunday Morning"]);
  });

  it("sorts by song count descending in songs mode", () => {
    const out = filterAndSortTiles(tiles, "", "songs");
    expect(out.map((x) => x.title)).toEqual(["Liked Songs", "Ambient Focus", "Currents", "Sunday Morning"]);
  });

  it("filters tiles by title or subtitle (artist/kind)", () => {
    expect(filterAndSortTiles(tiles, "tame", "default").map((x) => x.title)).toEqual(["Currents"]);
    expect(filterAndSortTiles(tiles, "sun", "default").map((x) => x.title)).toEqual(["Sunday Morning"]);
  });

  it("filters and optionally sorts followed artists", () => {
    const artists = ["Tame Impala", "Beach House", "Mac DeMarco"];
    expect(filterArtists(artists, "", "name")).toEqual(["Beach House", "Mac DeMarco", "Tame Impala"]);
    expect(filterArtists(artists, "mac", "default")).toEqual(["Mac DeMarco"]);
  });
});
