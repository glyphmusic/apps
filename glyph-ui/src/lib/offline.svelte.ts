// No connection for a while (OfflineBar sets `shown` after OFFLINE_AFTER_MS):
// the bar under the player, and on phones once per drop a sheet with the
// saved playlists (OfflineSheet).
import { app } from "./state.svelte";
import type { ListKey, Track } from "./api/types";

export const OFFLINE_AFTER_MS = 6000;

export const offline = $state({ shown: false, sheetOpen: false });

export type SavedList = { list: ListKey; name: string; tracks: Track[]; total: number };

/** Downloads first, then every playlist with songs saved on this device. */
export function savedLists(): SavedList[] {
  const out: SavedList[] = [];
  const dl = app.library.downloads;
  if (dl.length) out.push({ list: { kind: "downloads" }, name: "", tracks: dl, total: dl.length });
  for (const p of app.library.playlists) {
    const tracks = p.tracks.filter((x) => app.isDownloaded(x));
    if (tracks.length) out.push({ list: { kind: "playlist", id: p.id }, name: p.name, tracks, total: p.tracks.length });
  }
  return out;
}
