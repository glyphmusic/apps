// What the list / artist pages and the song menu do (src/app/lists.rs,
// app/menus.rs, app/library.rs, app/artist.rs). Things the shared `app`
// state has no method for live here and go straight to the backend.
import { app } from "../../state.svelte";
import { listSortDirs } from "../../ui.svelte";
import { backend } from "../../api/backend";
import { listId, trackKey, type ArtistPage, type Library, type ListKey, type Playlist, type Track } from "../../api/types";
import { t, tf } from "../../i18n.svelte";

export type PlaylistX = Playlist;
export type LibraryX = Library;
export const lib = () => app.library as LibraryX;
export const playlistX = (id: string) => app.playlist(id) as PlaylistX | undefined;

// ---- sorting (app/lists.rs + UI asc/desc direction) ----
export type ListSort = "custom" | "title" | "artist" | "album" | "duration";
export type SortDir = "asc" | "desc";

/** ListRef::sort_id: stable key for per-list sorting. */
export function sortId(l: ListKey): string | null {
  return listId(l);
}
export function listSort(l: ListKey): ListSort {
  const id = sortId(l);
  return ((id && (app.config?.lists.sort[id] as ListSort)) || "custom");
}
export function listSortDir(l: ListKey): SortDir {
  const id = sortId(l);
  return id && listSortDirs[id] === "desc" && listSort(l) !== "custom" ? "desc" : "asc";
}
export function setListSort(l: ListKey, s: ListSort, dir: SortDir = "asc") {
  const id = sortId(l);
  if (!id) return;
  if (s === "custom" || dir === "asc") delete listSortDirs[id];
  else listSortDirs[id] = "desc";
  try {
    if (typeof localStorage !== "undefined") localStorage.setItem("glyph:list-sort-dir", JSON.stringify($state.snapshot(listSortDirs)));
  } catch { /* ignore storage quota errors */ }
  app.selectedRow = null;
  return app.setConfig((c) => { if (s === "custom") delete c.lists.sort[id]; else c.lists.sort[id] = s; });
}
/** sort_order: stable, missing albums / lengths always last regardless of direction. */
export function sortOrder(tracks: Track[], s: ListSort, desc = false): number[] {
  const order = tracks.map((_, i) => i);
  const tx = (v?: string | null) => (v ?? "").trim().toLowerCase();
  const cmpTuple = (a: (string | number)[], b: (string | number)[]) => {
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
    return 0;
  };
  const missing = (i: number): boolean => {
    const tr = tracks[i];
    return (s === "album" && tr.album == null) || (s === "duration" && tr.duration_ms == null);
  };
  const key = (i: number): (string | number)[] => {
    const tr = tracks[i];
    switch (s) {
      case "title": return [tx(tr.title), tx(tr.artist)];
      case "artist": return [tx(tr.artist), tx(tr.album), tx(tr.title)];
      case "album": return [tx(tr.album), tx(tr.artist), tx(tr.title)];
      case "duration": return [tr.duration_ms ?? 0];
      default: return [];
    }
  };
  if (s !== "custom") {
    const sign = desc ? -1 : 1;
    order.sort((a, b) => {
      const ma = missing(a), mb = missing(b);
      if (ma !== mb) return ma ? 1 : -1;
      return cmpTuple(key(a), key(b)) * sign || a - b;
    });
  }
  return order;
}
/** The list's songs in the order the list view shows (and plays) them. */
export function shownTracks(l: ListKey): Track[] {
  const tracks = app.listTracks(l);
  const s = listSort(l);
  return s === "custom" ? tracks : sortOrder(tracks, s, listSortDir(l) === "desc").map((i) => tracks[i]);
}

// ---- playing ----
/** "Liked Songs" etc.: where the queue came from. */
export function contextOf(l: ListKey | null): string | undefined {
  return l ? app.listTitle(l) : undefined;
}
/** menus.rs play_shuffled: the whole list dealt with shuffle on. */
export function playShuffled(tracks: Track[], context?: string) {
  const ok = tracks.filter((x) => app.allowed(x));
  if (!ok.length) return;
  const l = app.view === "list" ? app.openList : null;
  const fromList = l && l.kind === "album" ? { ...l, tracks: [] } : l;
  backend().call("set_playing_from", { list: fromList });
  const from = { list: fromList, artist: app.view === "artist" ? app.openArtist : null, search: app.view === "search" };
  return backend().call("play_shuffled", { tracks: ok, context: context ?? null, from });
}
/** menus.rs queue_all. */
export function queueAll(tracks: Track[]) {
  const ok = tracks.filter((x) => app.allowed(x));
  if (!ok.length) return;
  return app.queueAdd(ok, false);
}
/** Every song "play next", keeping their order. */
export function playNextAll(tracks: Track[]) {
  return app.queueAdd(tracks.filter((x) => app.allowed(x)), true);
}
export const listRadio = (l: ListKey) => backend().call("list_radio", { list: l });
export const artistRadio = (artist: string) => backend().call("artist_radio", { artist, page_tracks: app.artists[artist.toLowerCase()]?.tracks?.slice(0, 5) ?? null });
export const songRadio = (track: Track) => backend().call("song_radio", { track, title: track.title });
/** One of `tracks` is the current song. */
export function playingFrom(tracks: Track[]): boolean {
  const k = app.currentKey;
  return !!k && tracks.some((x) => trackKey(x) === k);
}

// ---- downloads (app/downloads.rs) ----
export const isDownloading = (tr: Track) => trackKey(tr) in app.downloading;
export function removeDownloads(tracks: Track[]) {
  for (const tr of tracks) backend().call("download_remove", { track: tr });
  app.toast(tracks.length === 1 ? t("toast.dl_removed_one") : tf("toast.dl_removed", { n: tracks.length }));
}
/** downloads_view: ring while any runs (average progress), check when all are saved. */
export function downloadsView(tracks: Track[]): { state: "none" | "busy" | "done"; progress: number | null } {
  if (!tracks.length) return { state: "none", progress: null };
  let busy = false, sum = 0, done = 0;
  for (const tr of tracks) {
    const k = trackKey(tr);
    if (k in app.downloading) { busy = true; sum += app.downloading[k] ?? 0; }
    else if (app.isDownloaded(tr)) { sum += 1; done++; }
  }
  if (busy) return { state: "busy", progress: sum / tracks.length };
  return { state: done === tracks.length ? "done" : "none", progress: null };
}
export function downloadTip(tracks: Track[]): string {
  const total = tracks.length;
  if (!total) return t("tip.download");
  const saved = tracks.filter((x) => app.isDownloaded(x)).length;
  if (tracks.some(isDownloading)) return tf("tip.dl_busy", { done: saved, total });
  return saved === 0 ? t("tip.download") : tf("tip.dl_saved", { done: saved, total });
}

// ---- lists ----
/** Kind label of an own playlist ("Local playlist" ...). */
export function scopeKey(p: PlaylistX | undefined): string {
  if (!p || p.local_only || !app.account) return "playlist.scope.local";
  return p.auto_download ? "playlist.scope.offline" : "playlist.scope.account";
}
/** AlbumKind::from_count: 1-3 single, 4-6 EP, more an album. */
export function kindFromCount(n: number): string {
  return t(n <= 3 ? "album.kind.single" : n <= 6 ? "album.kind.ep" : "album.kind.album");
}
export function releaseKind(kind: string): string {
  return t(`album.kind.${kind}`);
}
export function openNameDialog(tracks: Track[] = [], rename: string | null = null) {
  app.ctx = null;
  app.nameDialog = { tracks, rename, initial: rename ? app.playlist(rename)?.name ?? "" : "" };
}
/** lists.rs remove_shown: removes the row shown at `i` (the rows may be
 *  sorted); the toast offers to put it back. */
export function removeShown(l: ListKey, i: number) {
  const tracks = app.listTracks(l);
  const s = listSort(l);
  const at = s === "custom" ? i : sortOrder(tracks, s, listSortDir(l) === "desc")[i];
  const tr = at == null ? undefined : tracks[at];
  if (!tr) return;
  const name = app.listTitle(l);
  if (l.kind === "recent") {
    backend().call("recent_remove_track", { track: tr });
    return;
  }
  const index = l.kind === "liked" || l.kind === "artist_liked" ? app.library.liked.findIndex((x) => x.id === tr.id && x.source === tr.source) : at;
  app.removeFromList(l.kind === "artist_liked" ? { kind: "liked" } : l, index);
  if (l.kind === "album") return;
  if (l.kind === "downloads") {
    app.toast(t("toast.dl_removed_one"), "ok", { label: t("toast.undo"), run: () => app.download([tr]) });
    return;
  }
  const target: ListKey = l.kind === "artist_liked" ? { kind: "liked" } : l;
  app.toast(`${tf("toast.removed_from", { name })} · ${tr.title}`, "ok", { label: t("toast.undo"), run: () => backend().call("list_insert", { list: target, index, track: tr }) });
}
/** library.rs delete_playlist: confirm first, then delete with an undo in the toast. */
export function deletePlaylist(id: string) {
  const name = app.playlist(id)?.name;
  if (name == null) return;
  app.ask({
    title: t("list.delete.confirm.title"),
    body: tf("list.delete.confirm.body", { name }),
    ok: t("list.delete.confirm.ok"),
    danger: true,
    run: () => {
      app.deletePlaylist(id);
      app.toast(`${t("toast.playlist_deleted")} · ${name}`, "ok", { label: t("toast.undo"), run: () => backend().call("playlist_restore", { id }) });
    },
  });
}
export const restorePlaylist = (id: string) => backend().call("playlist_restore", { id });
export const purgePlaylist = (id: string) => backend().call("playlist_purge", { id });
export const restoreVersion = (revision: string) => backend().call("playlist_restore_version", { revision });
export const setPlaylistStorage = (id: string, mode: 0 | 1 | 2) => backend().call("playlist_storage", { id, mode });
export const toggleAutoDownload = (id: string) => backend().call("playlist_auto_download", { id, on: !playlistX(id)?.auto_download });
export const setLikedAutoDownload = (on: boolean) => backend().call("liked_auto_download", { on });
export const pickCover = (id: string) => backend().call("playlist_pick_cover", { id });
export const removeCover = (id: string) => backend().call("playlist_remove_cover", { id });
export const shareList = (l: ListKey) => app.share({ list: l });
export const hideSidebar = (id: string) => backend().call("hide_sidebar", { id });

// ---- artists ----
export function artistPage(name: string): ArtistPage | undefined {
  return app.artists[name.toLowerCase()];
}
export const isVerified = (name: string) => app.isVerifiedArtist(name);
/** similar_artist_track: a song already known for `name` (its page, liked
 *  or recent songs) for the round picture. */
export function artistTrack(name: string): Track | null {
  const n = name.toLowerCase();
  const own = artistPage(name)?.tracks[0];
  if (own) return own;
  for (const r of app.library.recent) {
    if (r.kind === "song" && r.track.artist.toLowerCase().includes(n)) return r.track;
    if (r.kind === "artist" && r.name.toLowerCase() === n && r.track) return r.track;
  }
  return app.library.liked.find((x) => x.artist.toLowerCase().includes(n)) ?? null;
}
export function isHiddenArtist(name: string): boolean {
  const n = name.toLowerCase();
  return (lib().hidden_artists ?? []).some((x) => x.toLowerCase() === n);
}
export function toggleHiddenArtist(name: string) {
  const hidden = !isHiddenArtist(name);
  backend().call("hide_artist", { artist: name, on: hidden });
  app.toast(tf(hidden ? "toast.artist_hidden" : "toast.artist_shown", { name }));
}
export const copyArtistLink = (artist: string) => backend().call("copy_artist_link", { artist });
export const recommendFeedback = (artist: string, preference: 1 | -1 | -2 | null) => backend().call("recommend_feedback", { artist, preference });

/** lists.rs artist_releases: the songs found, grouped by album name. */
export function groupAlbums(tracks: Track[]): { title: string; tracks: Track[] }[] {
  const out: { title: string; tracks: Track[] }[] = [];
  for (const tr of tracks) {
    const a = tr.album?.trim();
    if (!a) continue;
    const g = out.find((x) => x.title.toLowerCase() === a.toLowerCase());
    if (g) g.tracks.push(tr); else out.push({ title: tr.album!, tracks: [tr] });
  }
  return out;
}
/** compact_count: 1234 -> "1.2K", 3_400_000 -> "3.4M". */
export function compactCount(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1e3).toFixed(1)}K`.replace(".0K", "K");
  return `${(n / 1e6).toFixed(1)}M`.replace(".0M", "M");
}
