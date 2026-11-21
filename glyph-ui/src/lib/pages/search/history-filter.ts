// Filters Search page recent-history entries according to the active filter chip.
import type { ListKey, SearchItem, Track } from "../../api/types";
import type { SearchFilter } from "../../state.svelte";

export type FilteredHistoryEntry =
  | { kind: "history"; item: SearchItem; origIndex: number }
  | { kind: "album"; list: Extract<ListKey, { kind: "album" }>; coverTrack: Track | null };

export function filterSearchHistory(
  history: readonly SearchItem[],
  recentLists: readonly ListKey[],
  filter: SearchFilter,
): FilteredHistoryEntry[] {
  if (filter === "albums") {
    const out: FilteredHistoryEntry[] = [];
    const seen = new Set<string>();
    for (const l of recentLists) {
      if (l.kind !== "album") continue;
      const k = `${l.title.trim().toLowerCase()}|${l.artist.trim().toLowerCase()}`;
      if (seen.has(k)) continue;
      seen.add(k);
      out.push({ kind: "album", list: l, coverTrack: l.cover ?? l.tracks[0] ?? null });
    }
    for (const it of history) {
      if (it.kind !== "song" || !it.track.album?.trim()) continue;
      const title = it.track.album.trim();
      const artist = it.track.artist;
      const k = `${title.toLowerCase()}|${artist.trim().toLowerCase()}`;
      if (seen.has(k)) continue;
      seen.add(k);
      out.push({
        kind: "album",
        list: { kind: "album", title, artist, tracks: [it.track], cover: it.track },
        coverTrack: it.track,
      });
    }
    return out;
  }

  const out: FilteredHistoryEntry[] = [];
  for (let i = 0; i < history.length; i++) {
    const it = history[i];
    if (
      filter === "all"
      || (filter === "tracks" && it.kind === "song")
      || (filter === "artists" && it.kind === "artist")
    ) {
      out.push({ kind: "history", item: it, origIndex: i });
    }
  }
  return out;
}
