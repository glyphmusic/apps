// App state for the UI (the Svelte side of src/app.rs `Glyph`). Data that
// lives in the Rust core (config, library, queue, player) is mirrored from
// backend events; everything else (which page, open menus, dialogs) is UI
// state kept here.
import { admin } from "./admin/admin.svelte";
import { backend, connect, type Backend } from "./api/backend";
import type {
  Account, ArtistPage, BackendState, Config, Device, HomeFeeds, Library, ListKey, Lyrics, PlayerState, Playlist,
  QueueState, RecentItem, ReleaseKind, RepeatMode, SearchResults, Snapshot, Track, UpdateInfo,
} from "./api/types";
import { listId, primaryArtist, sameList, trackKey } from "./api/types";
import { setLang, t, tf } from "./i18n.svelte";
import { initPluginEvents } from "./settings/plugin-events.svelte";
import { openPasted } from "./settings/migrate-state.svelte";
import { trimReveals } from "./ui.svelte";
import { inMiniWindow, inSideWindow } from "./window";
import { applyTheme } from "./theme";
import { gpuProbe, recommendVisuals } from "./glow";
import {
  albumCacheGet, albumCachePut, fetchInitialSnapshot, homeCacheLoad, homeCacheSave,
  loadArtistPhotos, loadWidth, pruneTrackedKeys, sameTrackKeys, saveArtistPhotos, saveWidth,
} from "./state/cache";
import { pickRangeKeys, selectedTracksOf, toggleAllKeys, togglePickedKey } from "./state/pick";

export type View =
  | "home" | "search" | "library" | "now_playing" | "list" | "artist" | "profile" | "section" | "releases" | "similar" | "stats" | "local" | "server";

export type SettingsPage =
  | "overview" | "playback" | "audio" | "appearance" | "server" | "account" | "storage" | "transfer" | "migrate" | "integrations" | "plugins" | "about" | "system" | "notifications";

export type SearchFilter = "all" | "tracks" | "albums" | "artists";

export type ControlMenu = "sleep" | "sort" | "mini" | "repeat" | "shuffle" | "back" | "forward" | "volume" | "account";

export interface Route { view: View; list: ListKey | null; artist: string | null; profile: string | null; section: Section | null }
export interface Section { title: string; icon: string; tracks: Track[] }

export interface CtxMenu {
  x: number; y: number;
  track: Track | null;
  /** List and row the song was opened from ("remove from …"). */
  from?: { list: ListKey; index: number } | null;
  artist?: string | null;
  /** Multiple credited artists when opening the desktop/mobile artist picker. */
  artists?: string[] | null;
  list?: ListKey | null;
  sidebar?: string | null;
  /** Queue row. */
  queueIndex?: number | null;
  /** Right-click on a heart: only the "Save to" picker (Liked Songs and playlists). */
  save?: boolean;
}

/** app.rs Toast: "Title · Sub" (sub shown in caps), check or info icon, optional undo. */
export interface Toast { id: number; title: string; sub?: string; kind: "ok" | "info"; at: number; life: number; undo?: { label?: string; run: () => void }; gone?: number }

export interface NameDialog {
  tracks: Track[];
  rename?: string | null;
  eqPreset?: boolean;
  initial?: string;
  title?: string;
  okLabel?: string;
  fallback?: string;
  nonEmpty?: boolean;
  onSubmit?: (name: string) => void | Promise<void>;
}
/** What the phone share menu shares: one song, a list, loose songs (recap),
 *  an artist, or a ready link (a Jam invite). */
export interface ShareTarget {
  track?: Track; list?: ListKey; songs?: { title: string; tracks: Track[]; kind?: "song" | "playlist" | "album" };
  artist?: string; link?: { url: string; title: string; sub?: string; icon?: string };
}

export type SideItem = { kind: "list"; list: ListKey } | { kind: "artist"; name: string; track: Track | null };

const HISTORY_CAP = 20;
const isArtistView = (v: View) => v === "artist" || v === "releases" || v === "similar";
const emptyLibrary = (): Library => ({ liked: [], recent: [], playlists: [], saved_albums: [], pinned: [], recent_lists: [], followed: [], downloads: [], search_history: [], hidden_sidebar: [] });
const emptyPlayer = (): PlayerState => ({ status: "idle", current: null, position: 0, duration: null, volume: 0.8, muted: false, shuffle: false, repeat: "off", source: null, preview: false });
const emptySearch = (): SearchResults => ({ query: "", tracks: [], artists: [], albums: [], suggestions: [], lyric_hits: [], loading: false, more: false, errors: [] });

/** Rust omits empty strings/maps/vecs via `skip_serializing_if`; fill defaults so UI bindings never see `undefined`. */
function normalizeConfig(c: Config): Config {
  c.lists.sort ??= {};
  c.mpd.address ??= "";
  c.ui.keys ??= {};
  c.local.folders ??= [];
  c.plugins.disabled ??= [];
  c.mcp.token ??= "";
  c.visuals.glow_shape ??= "";
  c.audio.eq_user_presets ??= [];
  c.audio.output_device ??= null;
  c.storage.download_dir ??= null;
  return c;
}

function sameTrackData(a: Track | null | undefined, b: Track | null | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  if (
    a.id !== b.id || a.source !== b.source || a.title !== b.title || a.artist !== b.artist
    || a.album !== b.album || a.duration_ms !== b.duration_ms || a.cover_url !== b.cover_url
    || a.isrc !== b.isrc || a.permalink !== b.permalink || Boolean(a.explicit) !== Boolean(b.explicit)
  ) return false;
  const ae = a.external_ids ?? {}, be = b.external_ids ?? {};
  const ak = Object.keys(ae), bk = Object.keys(be);
  return ak.length === bk.length && ak.every((k) => (ae as Record<string, string | undefined>)[k] === (be as Record<string, string | undefined>)[k]);
}

/** Songs or a playlist shared or opened with Glyph (overlays/AddShared.svelte). */
export type PickSongs = {
  tracks: Track[];
  file?: boolean;
  /** A shared playlist: offered as "Save as playlist". `code` for a glyph.my playlist. */
  list?: { name: string; origin?: string | null; code?: string; link?: string; author?: string | null } | null;
  /** Android share sheet over another app (share-sheet.svelte.ts). */
  sheet?: boolean;
  /** Still reading the link / files: what was shared, shown meanwhile. */
  loading?: boolean;
  files?: boolean;
  label?: string;
  error?: string;
};

/** How long hearts wait to be sent together (see App.flushLikes). */
const LIKE_BATCH_MS = 700;

class App {
  ready = $state(false);
  platform = $state<Snapshot["platform"]>("web");
  version = $state("0.0.0");
  config = $state<Config>(null as unknown as Config);
  library = $state<Library>(emptyLibrary());
  /** Keys of liked / downloaded songs: every song row asks, so a set instead of a walk through the whole list. */
  private likedKeys = $derived(new Set(this.library.liked.map(trackKey)));
  private downloadedKeys = $derived(new Set(this.library.downloads.map(trackKey)));
  queue = $state<QueueState>({ items: [], index: null, user: 0 });
  player = $state<PlayerState>(emptyPlayer());
  backendState = $state<BackendState>({ kind: "connecting" });
  account = $state<Account | null>(null);
  update = $state<UpdateInfo | null>(null);
  /** The server takes this version no more: "Update Glyph" covers the app
   *  (overlays/Outdated.svelte). `min`: the oldest version it takes, if known. */
  outdated = $state<{ min: string | null; current: string } | null>(null);
  /** Phones: open the settings right at this page, not at the list. */
  settingsDirect = $state<SettingsPage | null>(null);
  outputDevices = $state<string[]>([]);
  devices = $state<Device[]>([]);
  firstRun = $state(false);

  // ---- navigation (app/navigation.rs) ----
  view = $state<View>("home");
  openList = $state<ListKey | null>(null);
  openArtist = $state<string | null>(null);
  /** The list or artist page the queue was last started from (`play`), for the "Playing from" link. */
  playOrigin = $state<{ context: string | null; list: ListKey | null; artist: string | null } | null>(null);
  openProfile = $state<string | null>(null);
  section = $state<Section | null>(null);
  history = $state<Route[]>([]);
  forwardHistory = $state<Route[]>([]);
  settingsOpen = $state(false);
  settingsPage = $state<SettingsPage>("overview");

  // ---- panels ----
  showQueue = $state(false);
  showDevices = $state(false);
  showLyrics = $state(true);
  lyricsOnly = $state(false);
  zen = $state(false);
  miniShown = $state(inMiniWindow());
  compact = $state(false);
  leftSidebarW = $state(loadWidth("glyph.sidebar.left", 64, 64, 280));
  rightSidebarW = $state(loadWidth("glyph.sidebar.right", 320, 260, 580));

  // ---- menus, dialogs, toasts ----
  ctx = $state<CtxMenu | null>(null);
  controlMenu = $state<{ menu: ControlMenu; x: number; y: number } | null>(null);
  nameDialog = $state<NameDialog | null>(null);
  addSongs = $state<{ id: string; tab: number } | null>(null);
  toasts = $state<Toast[]>([]);
  confirm = $state<{ title: string; body: string; ok: string; danger?: boolean; run: () => void } | null>(null);
  /** A shared playlist link, shown as a preview until "Add to library" (GPUI shared_preview). */
  /** A shared playlist shown before it is saved; `id` of its glyph.my/p/<id>
   *  link when the web player opened one (its page address). */
  sharedPreview = $state<{ code: string; link: string; author: string | null; id?: string } | null>(null);
  /** Phone share menu (compact/ShareSheet.svelte): what is being shared. */
  shareSheet = $state<ShareTarget | null>(null);
  /** Songs shared or opened with Glyph: where they go (overlays/AddShared.svelte). */
  pickSongs = $state<PickSongs | null>(null);
  /** "Not the right one" alternative picker modal (overlays/WrongSongModal.svelte). */
  wrongSong = $state<Track | null>(null);
  onboarding = $state(false);
  splash = $state(false);
  jamOpen = $state(false);
  sleep = $state<{ until: number | null; mins: number | null; endOfSong: boolean } | null>(null);

  // ---- data pages ----
  search = $state<SearchResults>({ query: "", tracks: [], artists: [], albums: [], suggestions: [], lyric_hits: [], loading: false, more: false, errors: [] });
  searchQuery = $state("");
  searchFilter = $state<SearchFilter>("all");
  searchFocused = $state(false);
  home = $state<HomeFeeds>({ recs: [], trending: [], new_releases: [], loading: true });
  artists = $state<Record<string, ArtistPage>>({});
  lyrics = $state<{ key: string | null; lyrics: Lyrics | null; loading: boolean }>({ key: null, lyrics: null, loading: false });
  downloading = $state<Record<string, number | null>>({});
  selectedRow = $state<number | null>(null);
  libraryTab = $state(0);

  /** Seconds, eased between backend reports (app.rs pos_anchor). */
  private posAnchor = { pos: 0, at: performance.now() };
  clock = $state(0);
  private clockUntil = 0;
  private clockTimer = 0;
  private artistLru: string[] = [];

  private searchTimer = 0;
  private toastSeq = 0;
  private toastTimers = new Map<number, ReturnType<typeof setTimeout>>();
  private toastsPaused = false;
  ownAvatarUrl = $state<string | null>(null);
  private ownProfileHandle: string | null = null;
  private ownProfileLoading = false;

  async init() {
    const api = await connect();
    const snap = await fetchInitialSnapshot(api);
    this.config = normalizeConfig(snap.config);
    if (typeof document !== "undefined") {
      applyTheme(this.config.visuals.theme, this.config.ui.text_scale);
    }
    this.library = snap.library;
    this.queue = snap.queue;
    this.setPlayer(snap.player);
    this.backendState = snap.backend;
    this.account = snap.account;
    if (this.account && !inSideWindow()) this.ensureOwnProfile();
    this.version = snap.version;
    this.platform = snap.platform;
    this.outputDevices = snap.output_devices;
    this.update = snap.update;
    this.firstRun = snap.first_run;
    this.showQueue = snap.config.player.show_queue;
    setLang(snap.config.language || navigator.language);
    this.splash = snap.config.visuals.startup_animation;
    this.onboarding = snap.first_run && !inSideWindow();
    if (snap.first_run && !inSideWindow()) this.tuneVisuals();

    api.on("player", (p) => this.setPlayer({ ...this.player, ...p }));
    api.on("queue", (q) => (this.queue = q));
    // The core sends only the parts that changed (`{ __patch: {..} }`); the
    // rest stays the same object, so lists that did not change do not redraw.
    api.on("library", (l) => {
      const patch = (l as { __patch?: Partial<Library> }).__patch;
      this.library = patch ? { ...this.library, ...patch } : l;
      // Hearts still waiting to be sent stay as tapped (no flicker back).
      if (this.likeBatch.size && (!patch || patch.liked)) this.library.liked = this.withPendingLikes(this.library.liked);
      this.pruneAddSongs();
    });
    api.on("config", (c) => (this.config = normalizeConfig(c)));
    api.on("backend", (b) => {
      const was = this.backendState.kind;
      this.backendState = b;
      if (b.kind === "ready" && was !== "ready" && !inSideWindow()) this.onBackendReady();
    });
    api.on("search", (s) => {
      if (this.view === "search" && s.query === this.search.query) {
        this.search = s;
        this.scheduleClock();
      }
    });
    api.on("home", (h) => {
      this.home = h;
      if (!h.loading && (h.recs.length || h.trending.length)) homeCacheSave($state.snapshot(h) as HomeFeeds);
    });
    api.on("artist", (a) => this.cacheArtist(a));
    api.on("lyrics", (l) => { if (l.key === this.lyrics.key || l.key === this.currentKey) this.lyrics = l; });
    api.on("toast", (m) => {
      const text = m.key ? (m.vars ? tf(m.key, m.vars) : t(m.key)) : m.text;
      const a = m.action;
      this.toast(m.sub ? `${text} · ${m.sub}` : text, m.kind === "ok" || !m.kind ? "ok" : "info",
        a ? { label: t(a.key), run: () => { void backend().call(a.command, a.args ?? {}); } } : undefined);
    });
    api.on("download", (d) => { if (d.done) delete this.downloading[d.key]; else this.downloading[d.key] = d.progress; });
    api.on("devices", (d) => (this.devices = d));
    api.on("account", (a) => {
      const prev = this.account?.handle;
      this.account = a;
      if (!a) {
        this.ownAvatarUrl = null;
        this.ownProfileHandle = null;
      } else if (a.handle !== prev || !this.ownAvatarUrl) {
        this.ownProfileHandle = null;
        if (!inSideWindow()) this.ensureOwnProfile();
      }
    });
    api.on("update", (u) => (this.update = u));
    const setOutdated = (o: { outdated: boolean; min: string | null; current: string } | null | undefined) => {
      this.outdated = o?.outdated ? { min: o.min ?? null, current: o.current } : null;
    };
    api.on("app_outdated", setOutdated);
    // A tapped notification: Settings > About for a new version, the album for a release.
    if (!inSideWindow()) api.on("open_page", (p) => {
      if (p?.settings) { this.settingsDirect = p.settings as SettingsPage; this.openSettings(p.settings as SettingsPage); }
      else if (p?.album?.title && p.album.artist) this.openAlbum(p.album.title, p.album.artist);
    });
    // It may have been noticed before this page listened.
    void api.call<{ outdated: boolean; min: string | null; current: string } | undefined>("app_outdated").then(setOutdated).catch(() => {});
    api.on("mini", (v) => (this.miniShown = v));
    api.on("memory_pressure", ({ level }) => this.trimMemory(level === "hard"));
    if (inMiniWindow()) this.miniShown = true;
    // Plugins, MCP access questions, finished imports (settings/plugin-events.svelte.ts).
    if (!inSideWindow()) void initPluginEvents();
    // The mini player window only mirrors the player.
    // Shared to Glyph (share sheet, "Send to", `glyph <link>`): one song asks where it goes.
    if (!inSideWindow()) api.on("shared", (text) => this.importText(text, true));
    if (!inSideWindow()) api.on("pick_songs", (p) => { if (p.tracks?.length) { this.ctx = null; this.pickSongs = p; } });
    // A music link copied in another app: a toast offers to add it.
    if (!inSideWindow()) api.on("clipboard_link", (link) => {
      this.toast(`${t("share_in.copied")} · ${link}`, "info", { label: t("share_in.add"), run: () => { void this.importText(link, true); } });
    });

    if (typeof document !== "undefined") {
      // Leaving the app (phone home button, window hidden): send waiting hearts now.
      addEventListener("pagehide", () => this.flushLikes());
      document.addEventListener("visibilitychange", () => {
        if (document.hidden) this.flushLikes();
        this.scheduleClock();
        // Back from the background: only the player and the queue can be
        // behind (the whole snapshot also rebuilt the library and listed
        // the audio outputs, which froze phones for a moment on return).
        if (!document.hidden && this.ready && backend().kind === "tauri") {
          void backend().call<Pick<Snapshot, "player" | "queue">>("resume_state").then((s) => {
            if (!s) return;
            if (s.player) this.setPlayer(s.player);
            if (s.queue) this.queue = s.queue;
          }).catch(() => {});
        }
      });
    }
    this.clock = performance.now();
    this.scheduleClock();
    this.ready = true;
    if (!inSideWindow()) this.loadHome();
  }

  /** Keeps reactive `clock` ticking only when active UI needs smooth updates. */
  pulseClock(ms = 1000) {
    this.clockUntil = Math.max(this.clockUntil, performance.now() + ms);
    this.scheduleClock();
  }

  private clockIntervalMs(): number {
    if (typeof document !== "undefined" && document.hidden) return 0;
    const now = performance.now();
    // Short animations (spinners, the follow button) want smooth frames.
    if (this.loading || this.search.loading || this.jamOpen || now < this.clockUntil) {
      return 33;
    }
    // A playing song only moves the time and the progress bar: ten steps a
    // second look the same and cost a third of the work.
    if (this.playing) return 100;
    if (this.sleep?.until != null) return 1000;
    return 0;
  }

  private scheduleClock() {
    clearTimeout(this.clockTimer);
    this.clockTimer = 0;
    const ms = this.clockIntervalMs();
    if (!ms) return;
    this.clockTimer = window.setTimeout(() => {
      this.clockTimer = 0;
      const next = this.clockIntervalMs();
      if (!next) return;
      this.clock = performance.now();
      this.scheduleClock();
    }, ms);
  }

  private cacheArtist(a: ArtistPage) {
    const k = a.name.toLowerCase();
    this.artists[k] = a;
    const idx = this.artistLru.indexOf(k);
    if (idx !== -1) this.artistLru.splice(idx, 1);
    this.artistLru.push(k);
    this.trimArtistCache(12, k);
  }

  private trimArtistCache(limit: number, extraKeep?: string) {
    const keep = this.openArtist?.toLowerCase();
    while (this.artistLru.length > limit) {
      const dropIdx = this.artistLru.findIndex((x) => x !== keep && x !== extraKeep);
      if (dropIdx === -1) break;
      const victim = this.artistLru.splice(dropIdx, 1)[0];
      delete this.artists[victim];
    }
  }

  /** Evicts cached covers, artist pages, and badge/photo lookups under RAM pressure. */
  trimMemory(hard: boolean) {
    backend().trimCovers?.(hard ? 12 : 48);
    this.trimArtistCache(hard && !isArtistView(this.view) ? 0 : hard ? 1 : 2);
    const cap = hard ? 0 : 48;
    pruneTrackedKeys(this.photoAsked, this.artistPhotos, cap);
    pruneTrackedKeys(this.verifiedAsked, this.verified, cap);
    trimReveals(hard ? 0 : 64);
    if (hard && this.view !== "search") this.clearSearchState();
    if (this.view !== "list") this.openList = null;
    if (this.view !== "profile") this.openProfile = null;
    if (this.view !== "section") this.section = null;
    if (!isArtistView(this.view)) this.openArtist = null;
  }

  // ---- player ----
  private setPlayer(p: PlayerState) {
    const prev = this.player.current;
    // Backend player snapshots create fresh `current` JSON objects on every
    // position tick; keep the object reference when metadata is unchanged so
    // effects keyed on `player.current` do not restart every 100 ms.
    if (sameTrackData(prev, p.current)) p = { ...p, current: prev };
    const prevKey = prev ? trackKey(prev) : null;
    this.player = p;
    this.posAnchor = { pos: p.position, at: performance.now() };
    const key = p.current ? trackKey(p.current) : null;
    if (key && key !== prevKey) this.loadLyrics();
    this.scheduleClock();
  }

  get currentKey() { return this.player.current ? trackKey(this.player.current) : null; }
  get playing() { return this.player.status === "playing"; }
  get loading() { return this.player.status === "loading"; }

  /** Non-reactive interpolated playback position (ms) for rAF loops like synced lyrics. */
  positionMsNow(): number {
    const { pos, at } = this.posAnchor;
    if (this.player.status !== "playing") return pos * 1000;
    const d = this.player.duration ?? Infinity;
    return Math.min(d, pos + (performance.now() - at) / 1000) * 1000;
  }

  /** Smooth position between reports. */
  get position(): number {
    void this.clock;
    const { pos, at } = this.posAnchor;
    if (this.player.status !== "playing") return pos;
    const d = this.player.duration ?? Infinity;
    return Math.min(d, pos + (performance.now() - at) / 1000);
  }

  isCurrent(t: Track) { return this.currentKey === trackKey(t); }

  play(tracks: Track[], index = 0, context?: string) {
    const t0 = tracks[index];
    if (!t0) return;
    this.optimistic({ current: t0, status: "loading", position: 0, duration: t0.duration_ms ? t0.duration_ms / 1000 : null });
    // Started from an open list or artist page: counts for Home Recently played & sidebar.
    const l = this.view === "list" ? this.openList : null;
    const fromList = l && l.kind === "album" ? { ...l, tracks: [] } : l;
    backend().call("set_playing_from", { list: fromList });
    const from = {
      list: fromList,
      artist: this.view === "artist" ? this.openArtist : null,
      search: this.view === "search",
    };
    this.playOrigin = { context: context ?? null, list: fromList, artist: from.artist };
    return backend().call("play", { tracks, index, context: context ?? null, from });
  }

  /**
   * Where "Playing from <context>" leads: the list or artist page the queue
   * was started from when its name matches, else a list or artist with that
   * name (Liked Songs, a playlist, the playing song's artist), else the page
   * it was started from. null when nothing fits (radio, search results).
   */
  playingFromTarget(): { list: ListKey } | { artist: string } | null {
    const ctx = this.queue.context?.trim();
    if (!ctx) return null;
    const same = (a: string | null | undefined) => !!a && a.trim().toLowerCase() === ctx.toLowerCase();
    const o = this.playOrigin;
    if (o?.list && same(this.listTitle(o.list))) return { list: o.list };
    if (o?.artist && same(o.artist)) return { artist: o.artist };
    if (same(t("library.liked"))) return { list: { kind: "liked" } };
    const pl = this.library.playlists.find((p) => same(p.name));
    if (pl) return { list: { kind: "playlist", id: pl.id } };
    const cur = this.player.current;
    if (cur && (same(cur.artist) || same(primaryArtist(cur.artist)))) return { artist: primaryArtist(cur.artist) };
    if (cur?.album && same(cur.album)) return { list: { kind: "album", title: cur.album, artist: primaryArtist(cur.artist), tracks: [] } };
    if (o && (!o.context || same(o.context))) {
      if (o.list) return { list: o.list };
      if (o.artist) return { artist: o.artist };
    }
    return null;
  }

  /** Opens what "Playing from" names; false when there is nothing to open. */
  openPlayingFrom(): boolean {
    const to = this.playingFromTarget();
    if (!to) return false;
    if ("artist" in to) this.openArtistPage(to.artist);
    else this.openListView(to.list);
    return true;
  }
  playOne(t: Track) { return this.play([t], 0); }
  togglePlay() {
    if (!this.player.current) return backend().call("play_recent");
    this.optimistic({ status: this.playing ? "paused" : "playing" });
    return backend().call("toggle_play");
  }
  next() { return backend().call("next"); }
  previous() { return backend().call("previous"); }
  seek(sec: number) { this.optimistic({ position: sec }); return backend().call("seek", { position: sec }); }
  seekRelative(d: number) { return this.seek(Math.max(0, Math.min(this.player.duration ?? 0, this.position + d))); }
  setVolume(v: number) { this.optimistic({ volume: v, muted: false }); return backend().call("set_volume", { volume: v }); }
  toggleMute() { this.optimistic({ muted: !this.player.muted }); return backend().call("toggle_mute"); }
  toggleShuffle() { this.optimistic({ shuffle: !this.player.shuffle }); return backend().call("set_shuffle", { on: this.player.shuffle }); }
  setShuffle(on: boolean) { this.optimistic({ shuffle: on }); return backend().call("set_shuffle", { on }); }
  cycleRepeat() {
    const next: Record<RepeatMode, RepeatMode> = { off: "all", all: "one", one: "off" };
    return this.setRepeat(next[this.player.repeat]);
  }
  setRepeat(mode: RepeatMode) { this.optimistic({ repeat: mode }); return backend().call("set_repeat", { mode }); }
  setLoop(a: number | null, b: number | null) { this.optimistic({ loop_a: a, loop_b: b }); return backend().call("set_loop", { a, b }); }
  get volume() { return this.player.muted ? 0 : this.player.volume; }

  private optimistic(p: Partial<PlayerState>) { this.setPlayer({ ...this.player, ...p }); }

  // ---- queue ----
  queueAdd(tracks: Track[], next: boolean) {
    this.toast(tracks.length === 1 ? tf(next ? "toast.play_next" : "toast.queued", { title: tracks[0].title }) : tf("toast.queued_n", { n: tracks.length }));
    return backend().call("queue_add", { tracks, next });
  }
  queueRemove(index: number) { return backend().call("queue_remove", { index }); }
  queueMove(from: number, to: number) {
    const items = [...this.queue.items];
    const [it] = items.splice(from, 1);
    items.splice(to, 0, it);
    this.queue = { ...this.queue, items };
    return backend().call("queue_move", { from, to });
  }
  queueClear() { return backend().call("queue_clear"); }
  queueJump(index: number) { return backend().call("queue_jump", { index }); }
  /** Songs after the current one. */
  get upNext(): { track: Track; index: number }[] {
    const i = this.queue.index ?? -1;
    return this.queue.items.map((track, index) => ({ track, index })).filter((x) => x.index > i);
  }

  // ---- library ----
  isLiked(t: Track) { return this.likedKeys.has(trackKey(t)); }
  toggleLike(t: Track) {
    const on = !this.isLiked(t);
    this.library.liked = on ? [t, ...this.library.liked] : this.library.liked.filter((x) => trackKey(x) !== trackKey(t));
    this.likeBatch.set(trackKey(t), { track: t, on });
    clearTimeout(this.likeTimer);
    this.likeTimer = window.setTimeout(() => this.flushLikes(), LIKE_BATCH_MS);
    return Promise.resolve();
  }
  /** Likes or unlikes many songs at once (a selection, "Clear Liked Songs"):
   *  the list changes at once, the backend gets one call. */
  setLiked(tracks: Track[], on: boolean) {
    if (!tracks.length) return Promise.resolve();
    const ks = new Set(tracks.map(trackKey));
    for (const k of ks) this.likeBatch.delete(k);
    const rest = this.library.liked.filter((x) => !ks.has(trackKey(x)));
    this.library.liked = on ? [...tracks.filter((x) => !this.likedKeys.has(trackKey(x))), ...this.library.liked] : rest;
    return backend().call("like_many", { tracks, on });
  }
  // ---- select mode (multi-select in track lists) ----
  /** The list in select mode, picked song keys, and last clicked index for Shift+Click ranges. */
  picking = $state<{ list: string; keys: string[]; anchor?: number } | null>(null);
  private pickedSet = $derived(new Set(this.picking?.keys ?? []));
  /** Only Liked Songs (also per artist) and own playlists can drop songs in bulk. */
  canPick(l: ListKey | null | undefined) { return !!l && (l.kind === "liked" || l.kind === "artist_liked" || (l.kind === "playlist" && !!this.playlist(l.id))); }
  isPicking(l: ListKey | null | undefined) { return !!l && this.picking?.list === listId(l); }
  isPicked(t: Track) { return this.pickedSet.has(trackKey(t)); }
  startPicking(l: ListKey, first?: Track, anchor?: number) { this.picking = { list: listId(l), keys: first ? [trackKey(first)] : [], anchor }; }
  stopPicking() { this.picking = null; }
  togglePicked(t: Track) {
    const p = this.picking;
    if (!p) return;
    this.picking = { ...p, keys: togglePickedKey(p.keys, trackKey(t)) };
  }
  /** Toggles row `index` (or selects the inclusive range from `anchor` when `shift` is held). */
  togglePickedAt(tracks: readonly Track[], index: number, shift = false) {
    const p = this.picking;
    const tr = tracks[index];
    if (!p || !tr) return;
    if (shift && p.anchor != null) {
      this.picking = { ...p, keys: pickRangeKeys(tracks, p.keys, p.anchor, index, true), anchor: index };
      return;
    }
    this.picking = { ...p, keys: togglePickedKey(p.keys, trackKey(tr)), anchor: index };
  }
  /** Picks all `tracks`, or none when all are picked already. */
  pickAll(tracks: Track[]) {
    const p = this.picking;
    if (!p) return;
    this.picking = { ...p, keys: toggleAllKeys(tracks, p.keys), anchor: undefined };
  }
  pickedTracksOf(tracks: readonly Track[]): Track[] {
    return selectedTracksOf(tracks, this.picking?.keys ?? []);
  }
  /** Removes the picked songs of `l` in one go and leaves select mode. */
  removePicked(l: ListKey) {
    if (!this.isPicking(l)) return;
    const ks = this.pickedSet;
    const tracks = this.listTracks(l).filter((x) => ks.has(trackKey(x)));
    this.picking = null;
    this.removeTracks(l, tracks);
  }
  /** Removes `tracks` from Liked Songs or an own playlist (one backend call). */
  removeTracks(l: ListKey, tracks: Track[]) {
    if (!tracks.length) return;
    const n = tracks.length;
    if (l.kind === "liked" || l.kind === "artist_liked") {
      const ks = new Set(tracks.map(trackKey));
      const clearingAll = l.kind === "liked" && this.library.liked.length > 0 && this.library.liked.every((x) => ks.has(trackKey(x)));
      if (clearingAll) {
        this.likeBatch.clear();
        clearTimeout(this.likeTimer);
        this.likeTimer = 0;
        this.library.liked = [];
        void backend().call("clear_liked", {});
      } else {
        void this.setLiked(tracks, false);
      }
      this.toast(tf("toast.removed_n_liked", { n }), "ok", { label: t("toast.undo"), run: () => { void this.setLiked(tracks, true); } });
    } else if (l.kind === "playlist") {
      const ks = new Set(tracks.map(trackKey));
      const p = this.playlist(l.id);
      if (!p) return;
      const indices = p.tracks.flatMap((x, i) => (ks.has(trackKey(x)) ? [i] : []));
      const before = [...p.tracks];
      p.tracks = p.tracks.filter((x) => !ks.has(trackKey(x)));
      void backend().call("playlist_remove_many", { id: l.id, indices });
      this.toast(tf("toast.removed_n_from", { n, name: p.name }), "ok", {
        label: t("toast.undo"),
        // Back in their old places.
        run: () => { for (const i of [...indices].sort((a, b) => a - b)) void backend().call("list_insert", { list: l, index: i, track: before[i] }); },
      });
    }
  }
  /** "Clear Liked Songs": asks first, then removes all of them at once. */
  clearLiked() {
    const all = [...this.library.liked];
    if (!all.length) return;
    this.ask({
      title: t("liked.clear.title"),
      body: tf("liked.clear.body", { n: all.length }),
      ok: t("liked.clear.ok"),
      danger: true,
      run: () => { this.picking = null; this.removeTracks({ kind: "liked" }, all); },
    });
  }

  // Hearts tapped close together (unliking a lot of songs one by one) are
  // sent as one call after a short pause: each call saved and redrew the
  // whole library, and many in a row could leave a black screen.
  private likeBatch = new Map<string, { track: Track; on: boolean }>();
  private likeTimer = 0;
  flushLikes() {
    clearTimeout(this.likeTimer);
    this.likeTimer = 0;
    if (!this.likeBatch.size) return;
    const all = [...this.likeBatch.values()];
    this.likeBatch.clear();
    for (const on of [true, false]) {
      const tracks = all.filter((x) => x.on === on).map((x) => x.track);
      if (tracks.length === 1) void backend().call("like", { track: tracks[0], on });
      else if (tracks.length) void backend().call("like_many", { tracks, on });
    }
  }
  private withPendingLikes(liked: Track[]): Track[] {
    const add: Track[] = [];
    const drop = new Set<string>();
    for (const [k, x] of this.likeBatch) (x.on ? add.push(x.track) : drop.add(k));
    const have = new Set(liked.map(trackKey));
    return [...add.filter((x) => !have.has(trackKey(x))), ...liked.filter((x) => !drop.has(trackKey(x)))];
  }
  allowed(t: Track) { return !(this.config?.audio.hide_explicit && (t.explicit || (t.source === "youtube" && !t.external_ids?.deezer && !t.external_ids?.spotify))); }
  playlist(id: string): Playlist | undefined { return this.library.playlists.find((p) => p.id === id); }
  get addSongsOpen(): boolean {
    return (
      this.addSongs != null &&
      this.view === "list" &&
      this.openList?.kind === "playlist" &&
      this.openList.id === this.addSongs.id &&
      !!this.playlist(this.addSongs.id)
    );
  }
  private pruneAddSongs() {
    if (!this.addSongsOpen) this.addSongs = null;
  }
  async createPlaylist(name: string, tracks: Track[] = []) {
    return backend().call<string>("playlist_create", { name, tracks });
  }
  renamePlaylist(id: string, name: string) { return backend().call("playlist_rename", { id, name }); }
  deletePlaylist(id: string) {
    if (this.addSongs?.id === id) this.addSongs = null;
    const isDeleted = (r: Route) => r.view === "list" && r.list?.kind === "playlist" && r.list.id === id;
    this.history = this.history.filter((r) => !isDeleted(r));
    if (this.view === "list" && this.openList?.kind === "playlist" && this.openList.id === id) this.back();
    this.forwardHistory = this.forwardHistory.filter((r) => !isDeleted(r));
    return backend().call("playlist_delete", { id });
  }
  /** `quiet`: the place you tapped already shows it (a tick in a menu, a
   *  row in a panel), so no toast on top. */
  addToPlaylist(id: string, tracks: Track[], quiet = false) {
    const p = this.playlist(id);
    const viewing = this.view === "list" && this.openList?.kind === "playlist" && this.openList.id === id;
    if (!viewing && !quiet) {
      this.toast(tracks.length === 1 ? tf("toast.added_to_playlist", { name: p?.name ?? "" }) : tf("toast.added_n_to_playlist", { n: tracks.length, name: p?.name ?? "" }));
    }
    return backend().call("playlist_add", { id, tracks });
  }
  removeFromList(list: ListKey, index: number) {
    if (list.kind === "playlist") return backend().call("playlist_remove", { id: list.id, index });
    if (list.kind === "liked") { const tr = this.library.liked[index]; if (tr) return this.toggleLike(tr); }
    if (list.kind === "downloads") { const tr = this.library.downloads[index]; if (tr) return backend().call("download_remove", { track: tr }); }
    if (list.kind === "recent") return backend().call("recent_remove", { index });
  }
  movePlaylistItem(id: string, from: number, to: number) {
    const p = this.playlist(id);
    if (p) { const [it] = p.tracks.splice(from, 1); p.tracks.splice(to, 0, it); }
    return backend().call("playlist_move", { id, from, to });
  }
  isPinned(l: ListKey) { return this.library.pinned.some((p) => sameList(p, l)); }
  togglePin(l: ListKey) { return backend().call("pin", { list: l, on: !this.isPinned(l) }); }
  isSavedAlbum(l: ListKey) { return this.library.saved_albums.some((p) => sameList(p, l)); }
  toggleSaveAlbum(l: ListKey) { return backend().call("save_album", { list: l, on: !this.isSavedAlbum(l) }); }
  follows(a: string) { return this.library.followed.some((x) => x.toLowerCase() === a.toLowerCase()); }
  toggleFollow(a: string) {
    const on = !this.follows(a);
    this.library.followed = on ? [...this.library.followed, a] : this.library.followed.filter((x) => x.toLowerCase() !== a.toLowerCase());
    return backend().call("follow", { artist: a, on });
  }
  isDownloaded(tr: Track) { return this.downloadedKeys.has(trackKey(tr)); }
  download(tracks: Track[]) {
    for (const tr of tracks) {
      const k = trackKey(tr);
      if (!this.isDownloaded(tr) && !(k in this.downloading)) this.downloading[k] = null;
    }
    return backend().call("download", { tracks });
  }
  cancelDownloads(tracks: Track[]) {
    const busy = tracks.filter((tr) => trackKey(tr) in this.downloading);
    const list = busy.length ? busy : tracks;
    for (const tr of list) delete this.downloading[trackKey(tr)];
    if (!list.length) return Promise.resolve();
    return backend().call("download_cancel", { tracks: list });
  }
  /** Shares a song or list: on phones the share menu opens, on the desktop the link is copied. */
  share(what: { track?: Track; list?: ListKey }) {
    if (this.compact) { this.shareSheet = what; return Promise.resolve(null); }
    return backend().call<string>("share", what);
  }
  /** Shares an artist: share menu on phones, on the desktop the link is copied. */
  shareArtist(artist: string) {
    if (this.compact) { this.shareSheet = { artist }; return Promise.resolve(null); }
    return backend().call<string>("copy_artist_link", { artist });
  }
  /** Shares loose songs (recap): share menu on phones, else copied. */
  shareSongs(what: { title: string; tracks: Track[]; kind?: "song" | "playlist" | "album" }) {
    if (!what.tracks.length) return Promise.resolve(null);
    if (this.compact) { this.shareSheet = { songs: what }; return Promise.resolve(null); }
    return backend().call<string>("share_songs", { ...what, kind: what.kind ?? "playlist" });
  }

  /** Songs of a list. */
  listTracks(l: ListKey): Track[] {
    switch (l.kind) {
      case "liked": return this.library.liked;
      case "recent": return this.library.recent.flatMap((r) => (r.kind === "song" ? [r.track] : []));
      case "playlist": return this.playlist(l.id)?.tracks ?? [];
      case "album": return l.tracks;
      case "artist_liked": return this.library.liked.filter((x) => x.artist.toLowerCase().includes(l.artist.toLowerCase()));
      case "downloads": return this.library.downloads;
    }
  }
  listTitle(l: ListKey): string {
    switch (l.kind) {
      case "liked": return t("library.liked");
      case "recent": return t("library.recent");
      case "playlist": return this.playlist(l.id)?.name ?? "";
      case "album": return l.title;
      case "artist_liked": return t("library.liked");
      case "downloads": return t("library.downloads");
    }
  }

  /** Sidebar entries (app sidebar_items): pinned lists first (they count
   *  towards `n`), then the most used recent lists and artist pages (listening
   *  from them counts most, ties by recency). */
  sidebarItems(n: number): SideItem[] {
    const lib = this.library;
    const hidden = new Set(lib.hidden_sidebar);
    const exists = (k: ListKey) => k.kind !== "playlist" || lib.playlists.some((p) => p.id === k.id);
    const weights = lib.sidebar_weights ?? {};
    const pinned = lib.pinned.filter(exists);
    const room = Math.max(0, n - pinned.length);
    const recent: { w: number; i: number; item: SideItem }[] = [];
    lib.recent_lists.filter(exists).filter((k) => !lib.pinned.some((p) => sameList(p, k))).forEach((k, i) => {
      if (!hidden.has(listId(k))) recent.push({ w: weights[listId(k)] ?? 0, i, item: { kind: "list", list: k } });
    });
    (lib.recent_artists ?? []).forEach((a, i) => {
      const id = `artist:${a.name.trim().toLowerCase()}`;
      if (!hidden.has(id)) recent.push({ w: weights[id] ?? 0, i, item: { kind: "artist", name: a.name, track: a.track ?? null } });
    });
    recent.sort((a, b) => b.w - a.w || a.i - b.i);
    return [...pinned.map((l): SideItem => ({ kind: "list", list: l })), ...recent.slice(0, room).map((r) => r.item)].slice(0, n);
  }

  // ---- navigation (app/navigation.rs) ----
  private albumCoverSeed(title: string, artist: string): Track {
    return { id: `album:${artist}:${title}`, source: "deezer", title, artist, album: title };
  }
  private albumReleaseMeta(title: string, artist: string): { kind?: ReleaseKind; tracks?: number | null } | null {
    const tl = title.trim().toLowerCase();
    const al = primaryArtist(artist).trim().toLowerCase();
    if (!tl) return null;
    const match = (r: { title: string; artist: string; kind: ReleaseKind; tracks?: number | null }) =>
      r.title.trim().toLowerCase() === tl && (!al || primaryArtist(r.artist).trim().toLowerCase() === al);
    const hit =
      this.artists[al]?.releases.find((r) => r.title.trim().toLowerCase() === tl)
      ?? this.home.new_releases.find(match)
      ?? this.search.albums.find(match);
    return hit ? { kind: hit.kind, tracks: hit.tracks } : null;
  }
  private route(): Route {
    const artistView = isArtistView(this.view);
    const list = this.view === "list" && this.openList
      ? (this.openList.kind === "album" && !this.openList.artist.startsWith("@")
          ? {
              ...this.openList,
              tracks: [],
              cover: this.openList.cover ?? this.openList.tracks[0] ?? null,
              loading: true,
              ...(this.openList.expectedTracks == null && this.openList.tracks.length ? { expectedTracks: this.openList.tracks.length } : {}),
            }
          : this.openList)
      : null;
    return {
      view: this.view,
      list,
      artist: artistView ? this.openArtist : null,
      profile: this.view === "profile" ? this.openProfile : null,
      section: this.view === "section" ? this.section : null,
    };
  }
  private clearSearchState() {
    clearTimeout(this.searchTimer);
    if (
      this.search.query || this.search.tracks.length || this.search.artists.length
      || this.search.albums.length || this.search.suggestions.length || this.search.lyric_hits.length
      || this.search.errors.length || this.search.loading || this.search.error
    ) {
      this.search = emptySearch();
    }
    backend().call("release_search").catch(() => {});
  }
  private trimOffscreen(prevView: View, sameTargetChanged = false) {
    if (this.view !== "list") this.openList = null;
    if (!isArtistView(this.view)) this.openArtist = null;
    if (this.view !== "profile") this.openProfile = null;
    if (this.view !== "section") this.section = null;
    // Artist pages opened lately stay, so going back to one is instant
    // (memory pressure still empties them: trimMemory).
    this.trimArtistCache(12);
    if (prevView === "search" && this.view !== "search") this.clearSearchState();
    if (prevView !== this.view || sameTargetChanged) {
      backend().trimCovers?.(200);
      pruneTrackedKeys(this.photoAsked, this.artistPhotos, 48);
      pruneTrackedKeys(this.verifiedAsked, this.verified, 48);
      trimReveals(128);
    }
  }
  private pushView(v: View): View {
    const prev = this.view;
    this.settingsOpen = false;
    this.history.push(this.route());
    if (this.history.length > HISTORY_CAP) this.history.shift();
    this.forwardHistory = [];
    this.view = v;
    this.selectedRow = null;
    this.pruneAddSongs();
    return prev;
  }
  private fetchAlbumTracks(l: Extract<ListKey, { kind: "album" }>) {
    // Seen before: show the cached songs at once, refresh quietly behind them.
    const cached = albumCacheGet(l.title, l.artist);
    if (cached?.length && this.view === "list" && this.openList?.kind === "album" && sameList(this.openList, l) && !this.openList.tracks.length) {
      this.openList = { ...this.openList, tracks: cached, cover: this.openList.cover ?? cached[0] ?? null, loading: false };
    }
    backend().call<Track[]>("album", { title: l.title, artist: l.artist }).then((full) => {
      if (full?.length) albumCachePut(l.title, l.artist, $state.snapshot(full) as Track[]);
      if (this.view === "list" && this.openList?.kind === "album" && sameList(this.openList, l)) {
        // A failed refresh never empties a list that already shows songs.
        const tracks = full?.length ? full : this.openList.tracks;
        if (cached?.length && full?.length && sameTrackKeys(full, this.openList.tracks)) {
          if (this.openList.loading) this.openList = { ...this.openList, loading: false };
          return;
        }
        this.openList = {
          ...this.openList,
          tracks,
          cover: this.openList.cover ?? l.cover ?? tracks[0] ?? null,
          loading: false,
        };
      }
    }).catch(() => {
      if (this.view === "list" && this.openList?.kind === "album" && sameList(this.openList, l)) {
        this.openList = { ...this.openList, loading: false };
      }
    });
  }
  private fetchArtist(name: string) {
    const k = name.toLowerCase();
    if (!this.artists[k]) this.artists[k] = { name, tracks: [], loading: true, info: null, similar: [], releases: [] };
    this.scheduleClock();
    backend().call<ArtistPage>("artist", { name }).then((a) => {
      if (!a) return;
      if (isArtistView(this.view) && this.openArtist?.toLowerCase() === k) {
        if (a.name.toLowerCase() !== k) delete this.artists[k];
        this.cacheArtist(a);
      }
    }).catch(() => {
      if (this.artists[k] && isArtistView(this.view) && this.openArtist?.toLowerCase() === k) {
        this.artists[k] = { ...this.artists[k], loading: false };
      }
    });
  }
  private applyRoute(r: Route) {
    const prev = this.view;
    this.view = r.view;
    this.openList = r.list;
    this.openArtist = r.artist;
    this.openProfile = r.profile;
    this.section = r.section;
    this.selectedRow = null;
    this.pruneAddSongs();
    this.trimOffscreen(prev);
    if (r.view === "list" && r.list?.kind === "album" && !r.list.tracks.length && !r.list.artist.startsWith("@")) {
      this.fetchAlbumTracks(r.list);
    } else if (isArtistView(r.view) && r.artist && !this.artists[r.artist.toLowerCase()]) {
      this.fetchArtist(r.artist);
    } else if (r.view === "search" && this.searchQuery.trim() && !this.search.loading && this.search.query !== this.searchQuery.trim()) {
      this.runSearch(this.searchQuery);
    }
  }
  navigate(v: View | "settings") {
    if (v === "settings") { this.settingsOpen = !this.settingsOpen; return; }
    this.settingsOpen = false;
    if (v === this.view && v !== "list" && v !== "artist" && v !== "profile" && v !== "section") {
      if (v === "now_playing" && this.history.length) return this.back();
      return;
    }
    const prev = this.pushView(v);
    this.trimOffscreen(prev);
  }
  back() {
    if (this.view === "server" && admin.sub && !this.settingsOpen) { admin.sub = ""; return; }
    this.sharedPreview = null;
    const r = this.history.pop();
    this.settingsOpen = false;
    if (!r) {
      const prev = this.view;
      this.view = "home";
      this.pruneAddSongs();
      this.trimOffscreen(prev);
      return;
    }
    this.forwardHistory.push(this.route());
    if (this.forwardHistory.length > HISTORY_CAP) this.forwardHistory.shift();
    this.applyRoute(r);
  }
  forward() {
    this.sharedPreview = null;
    const r = this.forwardHistory.pop();
    if (!r) return;
    this.settingsOpen = false;
    this.history.push(this.route());
    if (this.history.length > HISTORY_CAP) this.history.shift();
    this.applyRoute(r);
  }
  openListView(l: ListKey, fetchAlbum = true) {
    const albumEmpty = l.kind === "album" && !l.tracks.length && !l.artist.startsWith("@");
    const cached = albumEmpty ? albumCacheGet(l.title, l.artist) : null;
    const meta = l.kind === "album" && !l.artist.startsWith("@") && (!l.releaseKind || l.expectedTracks == null)
      ? this.albumReleaseMeta(l.title, l.artist)
      : null;
    const releaseKind = l.kind === "album" ? (l.releaseKind ?? meta?.kind) : undefined;
    const expectedTracks = l.kind === "album" ? (l.expectedTracks ?? meta?.tracks) : undefined;
    const target: ListKey = l.kind === "album" && !l.artist.startsWith("@")
      ? {
          ...l,
          tracks: cached?.length ? cached : l.tracks,
          cover: l.cover ?? cached?.[0] ?? this.albumCoverSeed(l.title, l.artist),
          loading: cached?.length ? false : (l.loading ?? albumEmpty),
          ...(releaseKind ? { releaseKind } : {}),
          ...(expectedTracks != null ? { expectedTracks } : {}),
        }
      : l;
    this.sharedPreview = null;
    const prev = this.pushView("list");
    this.openList = target;
    this.trimOffscreen(prev, true);
    backend().call("list_opened", { list: target });
    if (fetchAlbum && target.kind === "album" && !target.artist.startsWith("@") && (albumEmpty || !target.tracks.length || target.loading)) {
      this.fetchAlbumTracks(target);
    }
  }
  openAlbum(
    title: string,
    artist: string,
    tracks: Track[] = [],
    cover: Track | null = null,
    meta?: { kind?: ReleaseKind; tracks?: number | null },
  ) {
    const inferred = !artist.startsWith("@") && (!meta?.kind || meta?.tracks == null) ? this.albumReleaseMeta(title, artist) : null;
    const releaseKind = meta?.kind ?? inferred?.kind;
    const expectedTracks = meta?.tracks ?? inferred?.tracks;
    const l: Extract<ListKey, { kind: "album" }> = {
      kind: "album",
      title,
      artist,
      tracks,
      cover: cover ?? tracks[0] ?? this.albumCoverSeed(title, artist),
      loading: !tracks.length && !artist.startsWith("@"),
      ...(releaseKind ? { releaseKind } : {}),
      ...(expectedTracks != null ? { expectedTracks } : {}),
    };
    this.openListView(l, false);
    this.fetchAlbumTracks(l);
  }
  openArtistPage(name: string) {
    name = primaryArtist(name);
    if (!name) return;
    const prev = this.pushView("artist");
    this.openArtist = name;
    this.trimOffscreen(prev, true);
    this.fetchArtist(name);
  }
  openProfilePage(handle: string) {
    const prev = this.pushView("profile");
    this.openProfile = handle;
    this.trimOffscreen(prev, true);
  }
  openSection(s: Section) {
    const prev = this.pushView("section");
    this.section = s;
    this.trimOffscreen(prev, true);
  }
  openSettings(page?: SettingsPage) { if (page) this.settingsPage = page; this.settingsOpen = true; }

  // ---- search (app/search.rs) ----
  setQuery(q: string) {
    this.searchQuery = q;
    clearTimeout(this.searchTimer);
    if (!q.trim()) { this.search = { ...this.search, query: "", tracks: [], artists: [], albums: [], lyric_hits: [], loading: false }; return; }
    if (/^(https?:\/\/|glyph:)/i.test(q.trim())) return;
    // Short pause after typing: searches start soon, without one per key.
    this.searchTimer = window.setTimeout(() => this.runSearch(q), 300);
  }
  runSearch(q = this.searchQuery) {
    clearTimeout(this.searchTimer);
    q = q.trim();
    if (!q) return;
    if (/^(https?:\/\/|glyph:)/i.test(q)) return this.importText(q);
    if (this.view !== "search") this.navigate("search");
    this.search = { query: q, tracks: [], artists: [], albums: [], suggestions: [], lyric_hits: [], loading: true, more: false, errors: [] };
    this.scheduleClock();
    backend().call<SearchResults & { paste?: string; text?: string }>("search", { query: q, page: 0 }).then((r) => {
      if (r?.paste) return this.pasted(r.paste, r.text ?? q);
      if (r && this.view === "search" && r.query === this.search.query) { this.search = r; this.scheduleClock(); }
    }).catch(() => { if (this.view === "search" && this.search.query === q) { this.search = { ...this.search, loading: false, error: t("status.backend_offline") }; this.scheduleClock(); } });
  }
  /** A code or link was pasted into the search field: it is imported instead. */
  private async pasted(kind: string, text: string) {
    this.searchQuery = "";
    this.search = { ...this.search, query: "", tracks: [], artists: [], albums: [], lyric_hits: [], loading: false, error: null };
    if (kind === "settings") { this.pendingSettingsCode = text; this.openSettings("transfer"); }
    else if (kind === "eq") { this.pendingEqCode = text; this.openSettings("audio"); }
    else await this.importText(text);
  }
  /** The backend came up (or came back): what failed meanwhile loads again. */
  private onBackendReady() {
    this.loadHome();
    if (this.account) this.ensureOwnProfile();
    if (this.player.current) this.loadLyrics();
    if (this.view === "search" && (this.search.error || this.search.errors.length) && this.search.query.trim()) this.runSearch(this.search.query);
    for (const [k, a] of Object.entries(this.artists)) if (a.loading) delete this.artists[k];
    if (this.view === "artist" && this.openArtist) this.openArtistPage(this.openArtist);
  }
  searchMore() {
    if (!this.search.more || this.search.loading) return;
    backend().call<SearchResults>("search_more", { query: this.search.query }).then((r) => { if (r && this.view === "search" && r.query === this.search.query) this.search = r; });
  }
  searchFor(text: string) { this.searchQuery = text; this.runSearch(text); }
  removeSearchHistory(i: number) { this.library.search_history.splice(i, 1); return backend().call("search_history_remove", { index: i }); }
  pickSearch(item: { kind: "song"; track: Track } | { kind: "artist"; name: string; track?: Track | null }) {
    return backend().call("search_history_add", { item });
  }
  /** A glyph.my/p/<id> link opened in the web player (the share page sends
   *  people here): the songs open as a list to play, save or open in the
   *  app; an artist link opens the artist. */
  async openShared(id: string) {
    let r: { share_kind?: string; name?: string; code?: string; link?: string; author?: string | null; tracks?: Track[] } | null = null;
    try { r = await backend().call("share_open", { id }); } catch { r = null; }
    if (!r?.code) { this.toastInfo(t("share.gone")); return; }
    if (r.share_kind === "artist" && r.name) { this.openArtistPage(r.name); return; }
    this.openListView({ kind: "album", title: r.name ?? "", artist: "", tracks: r.tracks ?? [] });
    this.sharedPreview = { code: r.code, link: r.link ?? "", author: r.author ?? null, id };
  }
  /** Links or codes pasted / shared to Glyph. Several service links open
   *  the "move from another app" screen (Rust answers `{ kind: "migrate" }`). */
  async importText(text: string, ask = false) {
    const r = await backend().call<{ kind?: string; text?: string; id?: string; name?: string; code?: string; link?: string; author?: string | null; tracks?: Track[] } | undefined | void>("import_text", { text, ask });
    if (!r || typeof r !== "object") return;
    if (r.kind === "pick" && r.tracks?.length) { this.ctx = null; this.pickSongs = { tracks: r.tracks }; return; }
    if (r.kind === "settings") { this.pendingSettingsCode = r.text ?? text; this.openSettings("transfer"); return; }
    if (r.kind === "migrate") {
      await openPasted(r.text ?? text);
    } else if (r.kind === "playlist" && r.id) this.openListView({ kind: "playlist", id: r.id });
    else if (r.kind === "artist" && r.name) this.openArtistPage(r.name);
    else if (r.kind === "share_preview" && r.code) {
      // A shared playlist: shown first, saved with "Add to library".
      this.openListView({ kind: "album", title: r.name ?? "", artist: "", tracks: r.tracks ?? [] });
      this.sharedPreview = { code: r.code, link: r.link ?? "", author: r.author ?? null };
    }
  }

  // ---- home, lyrics ----
  loadHome() {
    // Last Home first (no empty skeleton page on every start), fresh feeds replace it.
    if (!this.home.recs.length && !this.home.trending.length) {
      const c = homeCacheLoad();
      if (c) this.home = { ...c, loading: true };
    }
    backend().call<HomeFeeds>("home").then((h) => {
      if (!h) return;
      const empty = !h.recs.length && !h.trending.length && !h.new_releases.length;
      // A failed refresh (offline, server busy) keeps the cached feeds.
      if (empty && (this.home.recs.length || this.home.trending.length)) { this.home = { ...this.home, loading: false }; return; }
      this.home = h;
      if (!empty) homeCacheSave($state.snapshot(h) as HomeFeeds);
    }).catch(() => { this.home = { ...this.home, loading: false }; });
  }
  /** Code pasted into the search field, waiting for its settings page. */
  pendingSettingsCode = $state<string | null>(null);
  pendingEqCode = $state<string | null>(null);
  /** Pictures of artists whose page was not opened (tiles), and verified badges. */
  artistPhotos = $state<Record<string, string | null>>(loadArtistPhotos());
  private photoAsked = new Map<string, number>();
  verified = $state<Record<string, boolean>>({});
  private verifiedAsked = new Set<string>();
  /** The artist's picture; looked up in the background the first time. */
  /** A cached artist photo failed to load (file gone): forget it and ask again. */
  dropArtistPhoto(name: string) {
    const k = primaryArtist(name).toLowerCase();
    if (!this.artistPhotos[k]) return;
    delete this.artistPhotos[k];
    this.photoAsked.delete(k);
    saveArtistPhotos($state.snapshot(this.artistPhotos) as Record<string, string | null>);
  }
  artistPhoto(name: string): string | null {
    const k = primaryArtist(name).toLowerCase();
    const page = this.artists[k]?.info?.photo;
    if (page) return page;
    if (this.artistPhotos[k]) return this.artistPhotos[k];
    const at = this.photoAsked.get(k);
    if (this.backendState.kind === "ready" && (at === undefined || performance.now() - at > 30_000)) {
      pruneTrackedKeys(this.photoAsked, this.artistPhotos, 299);
      this.photoAsked.set(k, performance.now());
      backend().call<string | null>("artist_photo", { name: k }).then((p) => { if (p) { this.artistPhotos[k] = p; saveArtistPhotos($state.snapshot(this.artistPhotos) as Record<string, string | null>); } }).catch(() => {});
    }
    return null;
  }
  /** Is the artist verified? Looked up once per name. */
  isVerifiedArtist(name: string): boolean {
    const k = primaryArtist(name).toLowerCase();
    const page = this.artists[k]?.info;
    if (page) return !!page.verified;
    if (this.verified[k]) return true;
    if (k && this.backendState.kind === "ready" && !this.verifiedAsked.has(k)) {
      pruneTrackedKeys(this.verifiedAsked, this.verified, 299);
      this.verifiedAsked.add(k);
      backend().call<boolean>("artist_verified", { name: k }).then((v) => { if (v) this.verified[k] = true; }).catch(() => {});
    }
    return false;
  }
  loadLyrics() {
    const cur = this.player.current;
    if (!cur) return;
    const key = trackKey(cur);
    this.lyrics = { key, lyrics: null, loading: true };
    backend().call<Lyrics | null>("lyrics", { track: cur }).then(async (l) => {
      // Nothing found: lyrics a plugin published for this song, if any.
      if (!l) l = (await backend().call<Lyrics[] | undefined>("plugin_lyrics", { key }))?.[0] ?? null;
      if (this.lyrics.key === key) this.lyrics = { key, lyrics: l ?? null, loading: false };
    }).catch(() => { if (this.lyrics.key === key) this.lyrics = { key, lyrics: null, loading: false }; });
  }

  /** Preloads the signed-in user's own avatar so topbar and account overview show it right away. */
  async ensureOwnProfile(force = false) {
    const handle = this.account?.handle;
    if (!handle || this.ownProfileLoading) return;
    if (!force && this.ownProfileHandle === handle) return;
    this.ownProfileLoading = true;
    try {
      const s = await backend().call<{ avatar_url?: string | null }>("profile_settings");
      if (this.account?.handle === handle) {
        this.ownAvatarUrl = s?.avatar_url ?? null;
        this.ownProfileHandle = handle;
      }
    } catch { /* retry next time if offline or backend not ready */ }
    this.ownProfileLoading = false;
  }

  // ---- settings ----
  /**
   * First start: checks the device before the background starts and picks
   * who draws it (graphics card or processor) and whether performance mode
   * helps (glow.ts recommendVisuals). A graphics card that fails the check
   * is locked right away. The onboarding's speed page shows the pick and
   * lets the user change it.
   */
  private tuneVisuals() {
    try {
      const pick = recommendVisuals();
      const broken = gpuProbe().broken;
      const render = pick.render === "cpu" ? "cpu" : "";
      const v = this.config.visuals;
      if ((v.render_mode ?? "") === render && !!v.performance_mode === pick.perf && !!v.gpu_blocked === broken) return;
      void this.setConfig((c) => {
        c.visuals.render_mode = render;
        c.visuals.performance_mode = pick.perf;
        c.visuals.gpu_blocked = broken;
      });
    } catch (e) {
      console.warn("[visuals] device check failed, keeping the defaults:", e);
    }
  }

  setConfig(patch: (c: Config) => void) {
    patch(this.config);
    if (this.config.language) setLang(this.config.language);
    return backend().call("set_config", { config: $state.snapshot(this.config) });
  }

  // ---- menus, toasts ----
  openCtx(m: CtxMenu) { this.controlMenu = null; this.ctx = m; }
  closeMenus() { this.ctx = null; this.controlMenu = null; }
  openWrongSong(track: Track) { this.closeMenus(); this.wrongSong = track; }
  toast(msg: string, kind: Toast["kind"] = "ok", undo?: Toast["undo"]) {
    const id = ++this.toastSeq;
    const cut = msg.indexOf(" · ");
    const [title, sub] = cut > 0 ? [msg.slice(0, cut), msg.slice(cut + 3)] : [msg, undefined];
    const life = undo ? 6000 : 3500;
    // The same message again: bring the old toast to the front instead of stacking copies.
    const old = this.toasts.find((x) => !x.gone && x.title === title && x.sub === sub);
    if (old) {
      old.at = performance.now();
      old.life = life;
      if (undo) old.undo = undo;
      this.toasts = [old, ...this.toasts.filter((x) => x !== old)];
      clearTimeout(this.toastTimers.get(old.id));
      this.toastTimers.delete(old.id);
      if (!this.toastsPaused) this.toastTimers.set(old.id, setTimeout(() => this.dismissToast(old.id), life));
      return;
    }
    this.toasts = [{ id, title, sub, kind, at: performance.now(), life, undo }, ...this.toasts].slice(0, 6);
    if (!this.toastsPaused) this.toastTimers.set(id, setTimeout(() => this.dismissToast(id), life));
  }
  toastInfo(msg: string) { this.toast(msg, "info"); }
  /** "Copied": a toast on the computer; Android already shows its own copy
   *  note, so phones get no second one. */
  copied(msg: string) { if (this.platform !== "android") this.toast(msg); }
  pauseToasts(paused: boolean) {
    if (this.toastsPaused === paused) return;
    this.toastsPaused = paused;
    const now = performance.now();
    for (const x of this.toasts) {
      if (x.gone) continue;
      if (paused) {
        x.life = Math.max(800, x.life - (now - x.at));
        clearTimeout(this.toastTimers.get(x.id));
        this.toastTimers.delete(x.id);
      } else {
        x.at = now;
        clearTimeout(this.toastTimers.get(x.id));
        this.toastTimers.set(x.id, setTimeout(() => this.dismissToast(x.id), x.life));
      }
    }
  }
  dismissToast(id: number) {
    const tmr = this.toastTimers.get(id);
    if (tmr !== undefined) { clearTimeout(tmr); this.toastTimers.delete(id); }
    const x = this.toasts.find((x) => x.id === id);
    if (!x || x.gone) return;
    x.gone = performance.now();
    setTimeout(() => (this.toasts = this.toasts.filter((y) => y.id !== id)), 180);
  }
  ask(c: NonNullable<App["confirm"]>) { this.confirm = c; }

  recentPlayed(): RecentItem[] { return this.library.recent; }

  setLeftSidebarW(w: number, commit = false) {
    let next = Math.round(Math.max(64, Math.min(280, w)));
    if (commit && next < 92) next = 64;
    this.leftSidebarW = next;
    if (commit) saveWidth("glyph.sidebar.left", next);
  }
  setRightSidebarW(w: number, commit = false) {
    const next = Math.round(Math.max(260, Math.min(580, w)));
    this.rightSidebarW = next;
    if (commit) saveWidth("glyph.sidebar.right", next);
  }
}

export const app = new App();
