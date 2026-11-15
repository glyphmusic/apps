// Page addresses in the browser (web player and `npm run dev`; the Tauri app
// has no address bar): every page has its own URL under the base path, so a
// page can be shared and a reload stays on it.
//
//   /                     home            /search?q=…&filter=…   search
//   /library?filter=…     library         /playing?mode=…        now playing
//   /liked  /recent  /downloads           /liked/<artist>        liked songs of an artist
//   /playlist/<id>?add=…  playlist        /album/<artist>/<title> album
//   /artist/<name>        artist          /artist/<name>/releases|similar
//   /profile/<handle>     profile         /stats?range=…&recap=… stats
//   /local?tab=…&q=…      local files
//
// The browser's back and forward buttons move through the pages; the app's
// own back button does the same. Each history entry keeps the page and the
// app's back list, so a reload or "back" restores it as it was.
import { untrack } from "svelte";
import { app, type Route, type SearchFilter, type View } from "./state.svelte";
import type { ListKey } from "./api/types";
import { phone } from "./compact/cstate.svelte";
import { local, LOCAL_ALBUMS, LOCAL_ARTISTS, LOCAL_FOLDERS } from "./pages/local/local.svelte";
import { statsState } from "./pages/stats/stats.svelte";
import type { StatsRange } from "./pages/stats/types";
import { applySettingsHash, isSettingsHash, settingsHash } from "./settings/hash.svelte";

const BASE = import.meta.env.BASE_URL.replace(/\/$/, "");
const enc = encodeURIComponent;

const LOCAL_TABS = ["folders", "artists", "albums", "songs"] as const;
const LIB_FILTERS = ["all", "playlists", "albums", "artists"] as const;
const SEARCH_FILTERS: SearchFilter[] = ["all", "tracks", "albums", "artists"];
const STATS_RANGES: StatsRange[] = ["month", "last_month", "year", "all"];
const ADD_TABS = ["suggested", "liked", "recent", "import"] as const;

interface Entry { path: string; route: Route; history: Route[]; prev: string | null }

const withQuery = (base: string, sp: URLSearchParams) => {
  const qs = sp.toString();
  return qs ? `${base}?${qs}` : base;
};

/** Address of a page (without the base path), or null for pages that have none of their own. */
function pathOf(r: Route, query: string): string | null {
  switch (r.view) {
    case "home": return "/";
    case "search": {
      const sp = new URLSearchParams();
      if (query) sp.set("q", query);
      if (app.searchFilter !== "all") sp.set("filter", app.searchFilter);
      return withQuery("/search", sp);
    }
    case "library": {
      const f = app.compact ? (phone.libFilter || app.libraryTab) : 0;
      return f > 0 && LIB_FILTERS[f] ? `/library?filter=${LIB_FILTERS[f]}` : "/library";
    }
    case "now_playing": {
      const mode = app.zen ? "zen" : app.lyricsOnly ? "lyrics" : null;
      return mode ? `/playing?mode=${mode}` : "/playing";
    }
    case "stats": {
      const sp = new URLSearchParams();
      if (statsState.range !== "month") sp.set("range", statsState.range);
      if (statsState.recap) sp.set("recap", statsState.recap);
      return withQuery("/stats", sp);
    }
    case "local": {
      const sp = new URLSearchParams();
      const tabSlug = LOCAL_TABS[local.tab];
      if (local.tab !== LOCAL_FOLDERS && tabSlug) sp.set("tab", tabSlug);
      if (local.tab === LOCAL_FOLDERS && local.dir) sp.set("dir", local.dir);
      if (local.tab === LOCAL_ARTISTS && local.artist) sp.set("artist", local.artist);
      if (local.tab === LOCAL_ALBUMS && local.album) {
        sp.set("album", local.album[0]);
        sp.set("artist", local.album[1]);
      }
      if (local.query.trim()) sp.set("q", local.query.trim());
      return withQuery("/local", sp);
    }
    case "releases": return r.artist ? `/artist/${enc(r.artist)}/releases` : "/releases";
    case "similar": return r.artist ? `/artist/${enc(r.artist)}/similar` : null;
    case "artist": return r.artist ? `/artist/${enc(r.artist)}` : null;
    case "profile": return r.profile ? `/profile/${enc(r.profile.replace(/^@/, ""))}` : null;
    case "list": {
      const base = r.list ? listPath(r.list) : null;
      if (base && r.list?.kind === "playlist" && app.addSongs?.id === r.list.id) {
        return `${base}?add=${ADD_TABS[app.addSongs.tab] ?? "suggested"}`;
      }
      return base;
    }
    default: return null;
  }
}

function listPath(l: ListKey): string | null {
  switch (l.kind) {
    case "liked": return "/liked";
    case "recent": return "/recent";
    case "downloads": return "/downloads";
    case "playlist": return `/playlist/${enc(l.id)}`;
    case "artist_liked": return `/liked/${enc(l.artist)}`;
    // Profile playlists open as albums by "@handle"; they have no address of their own.
    case "album": return l.artist.startsWith("@") ? null : `/album/${enc(l.artist)}/${enc(l.title)}`;
  }
}

/** Restores in-page tabs, filters, ranges, and sub-views from URL query parameters. */
function applyParams(view: View, list: ListKey | null, search: string) {
  const sp = new URLSearchParams(search);

  if (view === "search") {
    const f = sp.get("filter") as SearchFilter | null;
    app.searchFilter = f && SEARCH_FILTERS.includes(f) ? f : "all";
  } else if (view === "library") {
    const f = sp.get("filter");
    const idx = f ? LIB_FILTERS.indexOf(f as (typeof LIB_FILTERS)[number]) : 0;
    const val = idx > 0 ? idx : 0;
    app.libraryTab = val;
    phone.libFilter = val;
  } else if (view === "now_playing") {
    const mode = sp.get("mode");
    app.zen = mode === "zen";
    app.lyricsOnly = mode === "lyrics";
  } else if (view === "stats") {
    const r = sp.get("range") as StatsRange | null;
    statsState.range = r && STATS_RANGES.includes(r) ? r : "month";
    statsState.recap = sp.get("recap") || null;
  } else if (view === "local") {
    const tab = sp.get("tab");
    const idx = tab ? LOCAL_TABS.indexOf(tab as (typeof LOCAL_TABS)[number]) : 0;
    local.tab = idx >= 0 ? idx : LOCAL_FOLDERS;
    local.dir = local.tab === LOCAL_FOLDERS ? sp.get("dir") || null : null;
    local.artist = local.tab === LOCAL_ARTISTS ? sp.get("artist") || null : null;
    const alb = sp.get("album"), art = sp.get("artist");
    local.album = local.tab === LOCAL_ALBUMS && alb != null && art != null ? [alb, art] : null;
    local.query = sp.get("q") ?? "";
  } else if (view === "list" && list?.kind === "playlist") {
    const add = sp.get("add");
    if (add) {
      const idx = ADD_TABS.indexOf(add as (typeof ADD_TABS)[number]);
      app.addSongs = { id: list.id, tab: idx >= 0 ? idx : 0 };
    } else if (app.addSongs?.id === list.id) {
      app.addSongs = null;
    }
  }
}

/** Opens the page at `path` (a shared link or a reload without saved state). */
function open(path: string, search: string) {
  const parts = path.split("/").filter(Boolean).map((s) => { try { return decodeURIComponent(s); } catch { return s; } });
  const [head, a, b] = parts;
  const simple: Record<string, View> = { library: "library", playing: "now_playing", stats: "stats", local: "local", releases: "releases" };
  if (!head) return;
  if (head === "search") {
    const q = new URLSearchParams(search).get("q") ?? "";
    if (q.trim()) app.searchFor(q); else app.navigate("search");
  } else if (simple[head]) app.navigate(simple[head]);
  else if (head === "artist" && a) {
    app.openArtistPage(a);
    if (b === "releases" || b === "similar") app.navigate(b);
  }
  else if (head === "album" && a && b) app.openAlbum(b, a);
  else if (head === "playlist" && a && app.playlist(a)) app.openListView({ kind: "playlist", id: a });
  else if (head === "liked") app.openListView(a ? { kind: "artist_liked", artist: a } : { kind: "liked" });
  else if (head === "recent") app.openListView({ kind: "recent" });
  else if (head === "downloads") app.openListView({ kind: "downloads" });
  else if (head === "profile" && a) app.openProfilePage(a);
  applyParams(app.view, app.openList, search);
}

/** Puts a saved page back (history entry of a reload or of back / forward). */
function restore(e: Entry) {
  app.history = e.history;
  app.forwardHistory = [];
  app.settingsOpen = false;
  const r = e.route;
  app.view = r.view; app.openList = r.list; app.openArtist = r.artist; app.openProfile = r.profile; app.section = r.section;
  const search = e.path.split("?")[1] ?? "";
  applyParams(r.view, r.list, search);
  if (r.view === "search") {
    const q = new URLSearchParams(search).get("q") ?? "";
    if (q && q !== app.search.query) { app.searchQuery = q; app.runSearch(q); }
  }
  if ((r.view === "artist" || r.view === "releases" || r.view === "similar") && r.artist && !app.artists[r.artist.toLowerCase()]) {
    // Loads the page's data again (after a reload), then keeps the restored back list.
    const h = app.history;
    app.openArtistPage(r.artist);
    app.view = r.view;
    app.history = h;
  }
}

const current = (): Route => ({ view: app.view, list: app.openList, artist: app.openArtist, profile: app.openProfile, section: app.section });
const baseOf = (p: string) => p.split("?")[0];

/** Keeps entries small: long song lists stay out of the browser's history. */
function slim(r: Route): Route {
  const cut = <T>(a: T[]) => (a.length > 200 ? a.slice(0, 200) : a);
  return {
    ...r,
    list: r.list?.kind === "album" ? { ...r.list, tracks: cut(r.list.tracks) } : r.list,
    section: r.section ? { ...r.section, tracks: cut(r.section.tracks) } : null,
  };
}

function title(r: Route) {
  const name = r.view === "artist" || r.view === "releases" || r.view === "similar" ? r.artist : r.view === "search" ? app.search.query : r.view === "list" && r.list ? app.listTitle(r.list) : r.view === "profile" ? r.profile : null;
  document.title = name ? `${name} · Glyph` : "Glyph";
}

/** Starts the address sync. Call once, after `app.init()`. */
export function installRoutes(): () => void {
  let popping = 0;
  let prev: string | null = (history.state as Entry | null)?.prev ?? null;
  const here = () => location.pathname.slice(BASE.length) + location.search;

  const saved = history.state as Entry | null;
  if (saved?.route && saved.path === here()) restore(saved);
  else open(location.pathname.slice(BASE.length), location.search);

  const onPop = (ev: PopStateEvent) => {
    const e = ev.state as Entry | null;
    if (popping > 0) { popping--; prev = e?.prev ?? null; return; }
    if (e?.route) { prev = e.prev; restore(e); }
    else { prev = null; app.history = []; app.forwardHistory = []; app.view = "home"; open(location.pathname.slice(BASE.length), location.search); }
    applySettingsHash(location.hash);
  };
  addEventListener("popstate", onPop);

  const stop = $effect.root(() => {
    $effect(() => {
      const r = current();
      const path = pathOf(r, app.search.query);
      const back = $state.snapshot(app.history) as Route[];
      untrack(() => {
        title(r);
        if (path == null) return;
        const cur = here();
        const hash = settingsHash() || (isSettingsHash(location.hash) ? "" : location.hash);
        const entry = (p: string | null): Entry => ({ path, route: slim($state.snapshot(r) as Route), history: back.slice(-20).map(slim), prev: p });
        if (path === cur) { history.replaceState(entry(prev), "", BASE + path + hash); return; }
        // The app's back button: the same as the browser's.
        if (prev != null && path === prev) { popping++; history.back(); return; }
        // Changing tabs, filters, queries, or sub-views on the same page updates the current entry.
        if (baseOf(path) === baseOf(cur)) { history.replaceState(entry(prev), "", BASE + path + hash); return; }
        prev = cur;
        history.pushState(entry(cur), "", BASE + path + hash);
      });
    });
  });
  return () => { stop(); removeEventListener("popstate", onPop); };
}
