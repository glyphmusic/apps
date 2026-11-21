// Pure filter and sort logic for Library tiles and followed artists.
import type { ListKey } from "../../api/types";

export type LibrarySort = "default" | "name" | "songs";

export interface LibraryTile {
  l: ListKey;
  kind: string;
  title: string;
  count: number;
  pinRank: number;
  origIndex: number;
}

export function filterAndSortTiles(
  tiles: readonly LibraryTile[],
  query: string,
  sort: LibrarySort,
): LibraryTile[] {
  const q = query.trim().toLowerCase();
  const filtered = q
    ? tiles.filter((x) => x.title.toLowerCase().includes(q) || x.kind.toLowerCase().includes(q))
    : [...tiles];

  filtered.sort((a, b) => {
    if (sort === "name") {
      return a.title.localeCompare(b.title, undefined, { sensitivity: "base" }) || a.origIndex - b.origIndex;
    }
    if (sort === "songs") {
      return b.count - a.count || a.title.localeCompare(b.title, undefined, { sensitivity: "base" });
    }
    // default: pinned first in pin order, then original order
    return a.pinRank - b.pinRank || a.origIndex - b.origIndex;
  });
  return filtered;
}

export function filterArtists(artists: readonly string[], query: string, sort: LibrarySort): string[] {
  const q = query.trim().toLowerCase();
  const filtered = q ? artists.filter((a) => a.toLowerCase().includes(q)) : [...artists];
  if (sort === "name") {
    filtered.sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));
  }
  return filtered;
}
