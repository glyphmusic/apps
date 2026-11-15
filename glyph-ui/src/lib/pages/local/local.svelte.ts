// State and actions of the "Local files" page (app/local.rs LocalView +
// Glyph::open_local / rescan_local / add_local_folder / remove_local_folder /
// set_local_tab / open_local_* / local_up / play_local). The music folders
// are config.local.folders; what the page shows is built by the backend
// (`local_page`), the UI only keeps where in the folders / artists / albums
// it is and the filter. Kept in a module so the page stays where it was when
// the user comes back.
import { backend } from "../../api/backend";
import { app } from "../../state.svelte";
import { t, tf } from "../../i18n.svelte";
import type { Track } from "../../api/types";

export const LOCAL_FOLDERS = 0, LOCAL_ARTISTS = 1, LOCAL_ALBUMS = 2, LOCAL_SONGS = 3;

export interface LocalPage {
  /** Songs in the library (all folders), and the scan progress while one runs. */
  total: number;
  progress: [number, number] | null;
  /** Subfolders (folders tab) or the music folders themselves, with their song count. */
  dirs: { path: string; name: string; count: number }[];
  /** Artists (artists tab, none open). */
  artists: { name: string; count: number }[];
  /** Albums (albums tab, none open); `cover` is the first song. */
  albums: { title: string; artist: string; count: number; cover: Track | null }[];
  /** Songs to list. */
  tracks: Track[];
}

export const emptyPage = (): LocalPage => ({ total: 0, progress: null, dirs: [], artists: [], albums: [], tracks: [] });

export const local = $state({
  tab: LOCAL_FOLDERS,
  dir: null as string | null,
  artist: null as string | null,
  album: null as [string, string] | null,
  /** Filter typed into the search field (lowercase). */
  query: "",
  scanning: false,
  scanned: false,
  /** Bumped when the library changed (scan done). */
  gen: 0,
});

export const roots = () => app.config?.local.folders ?? [];

/** Last part of a path ("Music"; a drive root keeps its whole path). */
export function displayName(p: string): string {
  const parts = p.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts[parts.length - 1] || p;
}

/** Same arguments the backend gets for the page and for "play". */
export const pageArgs = () => ({ tab: local.tab, dir: local.dir, artist: local.artist, album: local.album, query: local.query.trim().toLowerCase() });

export function openLocal() {
  if (!local.scanned) rescan();
}

/** Reads the music folders again. */
export async function rescan() {
  if (local.scanning) return;
  local.scanning = true;
  local.scanned = true;
  const folders = [...roots()];
  const quiet = folders.length === 0;
  try {
    const r = await backend().call<{ files: number; read: number; failed: number }>("local_scan", { folders });
    if (r && r.failed > 0) app.toastInfo(tf("local.scan.failed", { n: r.failed }));
    else if (r && r.read > 0 && !quiet) app.toast(tf("local.scan.done", { n: r.files }));
  } catch (e) {
    app.toastInfo(String((e as Error)?.message ?? e));
  }
  local.scanning = false;
  local.gen++;
  if (JSON.stringify(roots()) !== JSON.stringify(folders)) rescan();
}

/** Asks for a folder and adds it to the music folders. */
export async function addFolder() {
  const paths = await backend().call<string[] | null>("pick_folders", { prompt: t("local.add") });
  if (!paths?.length) return;
  let added = false;
  app.setConfig((c) => {
    for (const p of paths) {
      if (!c.local.folders.some((f) => f.toLowerCase() === p.toLowerCase())) { c.local.folders.push(p); added = true; }
    }
  });
  if (added) { local.dir = null; rescan(); }
}

/** Removes a music folder (its files stay where they are). */
export function removeFolder(folder: string) {
  app.setConfig((c) => { c.local.folders = c.local.folders.filter((f) => f !== folder); });
  if (local.dir && local.dir.startsWith(folder)) local.dir = null;
  rescan();
}

export function setTab(tab: number) { local.tab = tab; }
export function openDir(dir: string | null) { local.dir = dir; }
export function openArtist(name: string | null) { local.artist = name; }
export function openAlbum(a: [string, string] | null) { local.album = a; }

/** One step up inside the page (folder, artist, album); false when there is nothing to go up to. */
export function up(): boolean {
  const rs = roots();
  if (local.tab === LOCAL_FOLDERS) {
    const dir = local.dir;
    if (!dir) return false;
    if (rs.includes(dir)) openDir(null);
    else openDir(dir.replace(/[\\/]+[^\\/]*$/, "") || null);
    return true;
  }
  if (local.tab === LOCAL_ARTISTS && local.artist) { openArtist(null); return true; }
  if (local.tab === LOCAL_ALBUMS && local.album) { openAlbum(null); return true; }
  return false;
}

/** Plays what the page lists (everything while it lists folders, artists or albums). */
export function playLocal(shuffle: boolean) {
  return backend().call("local_play", { ...pageArgs(), shuffle });
}
