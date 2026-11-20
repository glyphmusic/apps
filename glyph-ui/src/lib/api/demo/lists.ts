// Demo handlers for the list / artist pages, the song menu and the name
// dialog (helper 1). Commands the Rust / Tauri side implements:
//
//   play_shuffled            { tracks: Track[], context: string | null }      -> void   (menus.rs play_shuffled: shuffle on, whole list dealt)
//   list_radio               { list: ListKey }                                -> void   (menus.rs list_radio)
//   artist_radio             { artist: string }                               -> void   (artist_radio)
//   song_radio               { track: Track, title: string }                  -> void   (start_radio_from)
//   recent_remove_track      { track: Track }                                 -> void   (remove_recent_track)
//   list_insert              { list: ListKey, index: number, track: Track }   -> void   (undo of a removal: insert_liked / insert_into_playlist / insert_recent_song)
//   playlist_delete          { id }                                           -> void   (overrides the core demo: moves the playlist to the trash)
//   playlist_restore         { id }                                           -> void   (restore_playlist from the trash)
//   playlist_purge           { id }                                           -> void   (purge_playlist: delete for good)
//   playlist_restore_version { revision }                                     -> void   (restore_playlist_version)
//   playlist_storage         { id, mode: 0 local | 1 account | 2 offline }    -> void   (set_playlist_storage)
//   playlist_auto_download   { id, on: boolean }                              -> void   (toggle_auto_download)
//   liked_auto_download      { on: boolean }                                  -> void   (set_liked_auto_download)
//   playlist_set_cover       { id, data }                                     -> void   (data = 512 px WebP/JPEG data URL from lib/picture.ts)
//   playlist_remove_cover    { id }                                           -> void
//   hide_artist              { artist, on: boolean }                          -> void   (toggle_hidden_artist; library.hidden_artists)
//   copy_artist_link         { artist, copy?: false }                         -> string (copies it, toasts; copy false: only returns it)
//   recommend_feedback       { artist, preference: 1 | -1 | -2 | null }      -> void   (recommendation_feedback)
//   downloaded_bytes         { tracks: Track[] }                              -> number (bytes on disk of those songs)
//   album_meta               { title, artist }                                -> { kind: "album"|"ep"|"single"|"compilation", year?: number } | null
//   import_parse_url         { url }                                          -> string | null (what the link is, e.g. "Spotify playlist")
//   import_link              { url, name: string | null }                     -> { error?: string | null } (imports into a new playlist)
//   import_playlist_file     {}                                               -> void   (file dialog, then import)
//   eq_preset_save           { name }                                         -> void   (add_eq_preset with the current gains)
//
// Library fields read by these screens that the shared types do not list yet:
// playlist.local_only / auto_download / shared_by, library.playlist_trash,
// playlist_versions, hidden_artists, liked_auto_download.
import type { DemoCtx, DemoHandler } from "../demo";
import type { ListKey, Playlist, Track } from "../types";
import { trackKey } from "../types";
import { TRACKS, byArtist, findAlbumDef, hash } from "../demo-data";

type Lib = DemoCtx["library"] & {
  playlist_trash?: { id: string; name: string; playlist?: Playlist }[];
  hidden_artists?: string[];
  liked_auto_download?: boolean;
};
type Pl = Playlist & { auto_download?: boolean; local_only?: boolean };

const call = (ctx: DemoCtx, cmd: string, args: Record<string, unknown>) =>
  (ctx as unknown as { call: (c: string, a: Record<string, unknown>) => Promise<unknown> }).call(cmd, args);
const lib = (ctx: DemoCtx) => ctx.library as Lib;
const pl = (ctx: DemoCtx, id: string) => ctx.library.playlists.find((p) => p.id === id) as Pl | undefined;
const shuffle = <T>(a: T[]) => { const b = [...a]; for (let i = b.length - 1; i > 0; i--) { const j = Math.floor(Math.random() * (i + 1)); [b[i], b[j]] = [b[j], b[i]]; } return b; };
const radio = (ctx: DemoCtx, seed: Track[], context: string) => {
  const artists = new Set(seed.map((x) => x.artist));
  const more = shuffle(TRACKS.filter((x) => !artists.has(x.artist))).slice(0, 12);
  return call(ctx, "play", { tracks: [...shuffle(seed).slice(0, 6), ...more], index: 0, context });
};
const toast = (ctx: DemoCtx, text: string, sub?: string) => ctx.emit("toast", { text, sub });

export const handlers: Record<string, DemoHandler> = {
  play_shuffled: (ctx, a) => {
    ctx.player.shuffle = true;
    ctx.emit("player", { shuffle: true });
    return call(ctx, "play", { tracks: shuffle(a.tracks as Track[]), index: 0, context: a.context ?? null });
  },
  list_radio: (ctx, a) => {
    const l = a.list as ListKey;
    const seed = l.kind === "playlist" ? pl(ctx, l.id)?.tracks ?? [] : l.kind === "album" ? l.tracks : ctx.library.liked;
    return radio(ctx, seed, "Radio");
  },
  artist_radio: (ctx, a) => radio(ctx, byArtist(a.artist), `${a.artist} Radio`),
  song_radio: (ctx, a) => radio(ctx, [a.track], `Radio · ${a.title}`),
  recent_remove_track: (ctx, a) => {
    const k = trackKey(a.track);
    ctx.library.recent = ctx.library.recent.filter((r) => !(r.kind === "song" && trackKey(r.track) === k));
    ctx.saveLibrary();
  },
  list_insert: (ctx, a) => {
    const l = a.list as ListKey, tr = a.track as Track, i = a.index as number;
    if (l.kind === "playlist") pl(ctx, l.id)?.tracks.splice(i, 0, tr);
    else if (l.kind === "liked") ctx.library.liked.splice(i, 0, tr);
    else if (l.kind === "recent") ctx.library.recent.splice(i, 0, { kind: "song", track: tr });
    ctx.saveLibrary();
  },
  playlist_delete: (ctx, a) => {
    const p = pl(ctx, a.id);
    if (!p) return;
    ctx.library.playlists = ctx.library.playlists.filter((x) => x.id !== a.id);
    lib(ctx).playlist_trash = [{ id: p.id, name: p.name, playlist: p }, ...(lib(ctx).playlist_trash ?? [])];
    ctx.saveLibrary();
  },
  playlist_restore: (ctx, a) => {
    const tr = lib(ctx).playlist_trash ?? [];
    const it = tr.find((x) => x.id === a.id);
    if (!it?.playlist) return;
    lib(ctx).playlist_trash = tr.filter((x) => x.id !== a.id);
    ctx.library.playlists.unshift(it.playlist);
    ctx.saveLibrary();
  },
  playlist_purge: (ctx, a) => {
    lib(ctx).playlist_trash = (lib(ctx).playlist_trash ?? []).filter((x) => x.id !== a.id);
    ctx.saveLibrary();
  },
  playlist_restore_version: () => {},
  playlist_storage: (ctx, a) => {
    const p = pl(ctx, a.id);
    if (!p) return;
    p.local_only = a.mode === 0;
    p.auto_download = a.mode === 2;
    ctx.saveLibrary();
  },
  playlist_auto_download: (ctx, a) => {
    const p = pl(ctx, a.id);
    if (p) { p.auto_download = !!a.on; ctx.saveLibrary(); }
  },
  liked_auto_download: (ctx, a) => { lib(ctx).liked_auto_download = !!a.on; ctx.saveLibrary(); },
  playlist_set_cover: (ctx, a) => {
    const p = pl(ctx, a.id);
    if (p && typeof a.data === "string") { p.cover = a.data; ctx.saveLibrary(); }
  },
  playlist_remove_cover: (ctx, a) => {
    const p = pl(ctx, a.id);
    if (p) { p.cover = null; ctx.saveLibrary(); }
  },
  hide_artist: (ctx, a) => {
    const n = String(a.artist).toLowerCase();
    const cur = (lib(ctx).hidden_artists ?? []).filter((x) => x.toLowerCase() !== n);
    lib(ctx).hidden_artists = a.on ? [...cur, a.artist] : cur;
    ctx.saveLibrary();
  },
  copy_artist_link: async (ctx, a) => {
    const link = `https://glyph.my/a/${encodeURIComponent(a.artist)}`;
    if (a.copy === false) return link;
    try { await navigator.clipboard.writeText(link); } catch { /* preview without clipboard access */ }
    toast(ctx, "Glyph link copied");
    return link;
  },
  recommend_feedback: () => {},
  downloaded_bytes: (ctx, a) => {
    const have = new Set(ctx.library.downloads.map(trackKey));
    return (a.tracks as Track[]).filter((x) => have.has(trackKey(x))).reduce((s, x) => s + Math.round((x.duration_ms ?? 180_000) * 24), 0);
  },
  album_meta: (_, a) => {
    const def = findAlbumDef(String(a.artist ?? ""), String(a.title ?? ""));
    if (def) return { kind: def.kind ?? "album", year: def.year };
    const n = TRACKS.filter((x) => x.album?.toLowerCase() === String(a.title).toLowerCase()).length;
    if (!n) return null;
    return { kind: n <= 3 ? "single" : n <= 6 ? "ep" : "album", year: 2016 + (hash(String(a.title)) % 10) };
  },
  import_parse_url: (_, a) => {
    const u = String(a.url).toLowerCase();
    const known: [string, string][] = [["spotify", "Spotify playlist"], ["soundcloud", "SoundCloud playlist"], ["deezer", "Deezer playlist"], ["youtube", "YouTube playlist"], ["music.apple", "Apple Music playlist"], ["bandcamp", "Bandcamp album"], ["last.fm", "Last.fm profile"]];
    return known.find(([d]) => u.includes(d))?.[1] ?? null;
  },
  import_link: async (ctx, a) => {
    await new Promise((r) => setTimeout(r, 1200));
    const tracks = shuffle(TRACKS).slice(0, 9);
    const pid = a.playlist_id as string | undefined;
    if (pid) {
      const p = ctx.library.playlists.find((x) => x.id === pid);
      if (p) {
        const have = new Set(p.tracks.map((t) => `${t.source}:${t.id}`));
        p.tracks.push(...tracks.filter((t) => !have.has(`${t.source}:${t.id}`)));
        ctx.saveLibrary();
        toast(ctx, "Playlist imported", p.name);
        return { error: null, playlist: p.id };
      }
    }
    const id = `pl-${Date.now()}`;
    const name = (a.name as string | undefined) || "Imported playlist";
    ctx.library.playlists.unshift({ id, name, tracks, created_at: Date.now() });
    ctx.saveLibrary();
    toast(ctx, "Playlist imported", name);
    return { error: null, playlist: id };
  },
  import_playlist_file: () => {},
  eq_preset_save: (ctx, a) => {
    ctx.config.audio.eq_user_presets = [...ctx.config.audio.eq_user_presets, { name: a.name, gains: [...ctx.config.audio.eq_gains] }];
    ctx.emit("config", ctx.config);
  },
};
