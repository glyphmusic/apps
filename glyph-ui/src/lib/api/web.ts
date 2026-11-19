// Web backend: Glyph in a plain browser at glyph.my/web, no install. Search,
// resolve, lyrics, radio and the chart come from glyph-server's public /v1
// API (web-server.ts); sound plays straight from the provider (web-player.ts),
// never through the server. Settings, library, queue and recent lists stay in
// this browser (web-store.ts). What a browser cannot do (downloads, local
// files, yt-dlp, Cast / AirPlay / DLNA, MPD, Discord, plugins, autostart,
// tray) is hidden in the UI (lib/web.ts `appOnly`); its commands answer
// harmlessly here. Commands follow the lists at the top of demo.ts and
// demo/*.ts.
import type { Backend } from "./backend";
import type { StatsRange } from "../pages/stats/types";
import type {
  ArtistInfo, ArtistPage, BackendEvents, BackendState, Config, HomeFeeds, HomeTop, Library, ListKey, Lyrics, PlayerState, ProviderId, QueueState,
  RecentItem, Release, SearchResults, Snapshot, Track,
} from "./types";
import { PROVIDER_NAME, isYoutubeCover, primaryArtist, sameList, trackKey } from "./types";
import { WebAccountManager } from "./web-account";
import { WebConnectManager } from "./web-connect";
import { WebMcpManager } from "./web-mcp";
import { ApiError, server, type PlaybackCommandWire, type PlaybackSource, type SearchEvent } from "./web-server";
import { WebEngine, serverAudio } from "./web-player";
import { WebStats } from "./web-stats";
import { cancelSoon, load, loadBig, save, saveSoon } from "./web-store";
import { filterTrackByEnabled } from "../settings/sources";

interface WebPlaybackSession {
  queue: QueueState;
  position: number;
  duration: number | null;
  track_key: string | null;
  listened_ms: number;
  counted_play: boolean;
  saved_at: number;
}

const playbackStorageKey = (accountId: string | null) =>
  accountId ? `playback.${accountId}` : "playback.guest";

type Handler = (a: Record<string, any>) => unknown;

/** Sources a browser can play: streams and previews, YouTube in its own player. */
const WEB_SOURCES: ProviderId[] = ["audius", "jamendo", "archive", "youtube"];
const VERSION = String(import.meta.env.VITE_GLYPH_VERSION ?? "0.0.0");
const HOME_TTL = 10 * 60_000;

export function webConfig(): Config {
  return {
    server_url: "", language: "", keys: {},
    providers: { local_search: false, youtube_mode: "server", youtube_only: false, enabled: ["audius", "jamendo", "archive", "soundcloud", "youtube", "deezer"], priority: ["audius", "jamendo", "archive", "youtube"], extractor: "none" },
    visuals: { intensity: 0.8, music_reactivity: 1, ascii_covers: false, startup_animation: false, ascii_effects: true, theme: "glyph", throttle_unfocused: true, glow_shape: "", performance_mode: false, render_mode: "" },
    updates: { repo: "glyphmusic/apps", check_on_start: false, auto_install: false, channel: "stable" },
    audio: { crossfade_s: 0, quality: "auto", download_quality: "high", download_parallelism: 3, download_adaptive: true, level: "normal", normalize: false, hide_explicit: false, channels: "stereo", eq_enabled: false, eq_preset: "flat", eq_gains: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0], eq_user_presets: [], eq_bass_db: 0, eq_headroom_db: 0, output_device: null },
    storage: { cache_limit_mb: 0, data_saver: false, download_dir: null },
    integrations: { discord_presence: false, discord_client_id: "" },
    sharing: { glyph_links: true, profile_sections: ["playlists", "recent"] },
    player: { volume: 0.8, shuffle: false, repeat: "off", show_queue: false, mini_on_top: false, mini_large: false },
    playback: { shuffle: "smart", skip_unplayable: true, patient_search: false },
    lists: { sort: {} },
    stats: { enabled: false },
    mcp: { enabled: false, mode: "server", port: 0, token: "", access: { playback: "write", library: "read", stats: "none", settings: "write" } },
    mpd: { enabled: false, address: "" },
    devices: { discovery: false },
    plugins: { auto_update: false, skip_install_warning: false, sort: "stars", disabled: [] },
    desktop: { autostart: false, start_in_tray: false },
    ui: { wheel_seek_s: 5, wheel_volume_pct: 5, np_download: false, np_lyrics: true, keys: {}, reveal_downloads: false, text_scale: 1 },
    local: { folders: [] },
  };
}

// Found cover addresses, kept across reloads (Home and lists show them at
// once instead of loading each cover again through the server).
const COVER_KEEP_MS = 7 * 24 * 3600_000;
function loadCoverUrls(): [string, { url: string | null; expiresAt: number }][] {
  const saved = load<[string, string, number][]>("cover_urls") ?? [];
  const now = Date.now();
  return Array.isArray(saved) ? saved.filter((e) => Array.isArray(e) && e[2] > now).map(([k, url, at]) => [k, { url, expiresAt: at }]) : [];
}
let coverSaveTimer = 0;
function saveCoverUrlsSoon(m: Map<string, { url: string | null; expiresAt: number }>) {
  clearTimeout(coverSaveTimer);
  coverSaveTimer = window.setTimeout(() => {
    const out: [string, string, number][] = [];
    for (const [k, v] of m) if (v.url) out.push([k, v.url, v.expiresAt]);
    save("cover_urls", out.slice(-400));
  }, 1500);
}

const emptyLibrary = (): Library => ({ liked: [], recent: [], playlists: [], saved_albums: [], pinned: [], recent_lists: [], followed: [], downloads: [], search_history: [], hidden_sidebar: [] });

/** Deep copy, like Tauri's JSON bridge: the UI never holds our objects. */
const wire = <T>(v: T): T => (v === undefined ? v : (JSON.parse(JSON.stringify(v)) as T));
const norm = (s: string) => s.trim().toLowerCase();
const shuffled = <T>(a: T[]) => { const b = [...a]; for (let i = b.length - 1; i > 0; i--) { const j = Math.floor(Math.random() * (i + 1)); [b[i], b[j]] = [b[j], b[i]]; } return b; };

/** Same song from several providers: one row, all ids kept (dedupe::merge). */
function mergeInto(list: Track[], add: Track[]): Track[] {
  const at = new Map(list.map((t, i) => [trackKey(t), i]));
  for (const t of add) {
    const k = trackKey(t);
    const i = at.get(k);
    if (i == null) { at.set(k, list.length); list.push(t); continue; }
    const cur = list[i];
    const ids = { ...(t.external_ids ?? {}), ...(cur.external_ids ?? {}) };
    if (t.source !== cur.source) ids[t.source] = t.id;
    const cover_url = (!cur.cover_url || (isYoutubeCover(cur.cover_url) && t.cover_url && !isYoutubeCover(t.cover_url)))
      ? (t.cover_url ?? cur.cover_url)
      : cur.cover_url;
    list[i] = { ...cur, external_ids: ids, cover_url, album: cur.album ?? t.album, duration_ms: cur.duration_ms ?? t.duration_ms, isrc: cur.isrc ?? t.isrc, explicit: cur.explicit || t.explicit };
  }
  return list;
}

/** A server that does not know `direct=1` yet answers with its own audio path
 *  (`/v1/audio/<provider>/<id>`). Never play that: YouTube becomes its official
 *  embed, Audius its public stream URL; anything else counts as no source. */
function direct(src: PlaybackSource | null): PlaybackSource | null {
  if (!src || src.kind === "embed" || !serverAudio(src.url)) return src;
  const m = /\/v1\/(?:audio|download)\/([a-z]+)\/([A-Za-z0-9_.-]+)$/.exec(new URL(src.url, location.href).pathname);
  if (m?.[1] === "youtube" && /^[A-Za-z0-9_-]{11}$/.test(m[2])) return { kind: "embed", provider: "youtube", embed_id: m[2] };
  if (m?.[1] === "audius") return { kind: "stream", provider: "audius", url: `https://api.audius.co/v1/tracks/${encodeURIComponent(m[2])}/stream?app_name=glyph` };
  return null;
}

const kindOf = (n: number): Release["kind"] => (n <= 3 ? "single" : n <= 6 ? "ep" : "album");
const isRateLimit = (e: unknown): e is ApiError =>
  e instanceof ApiError && (e.status === 429 || e.code === "rate_limited" || e.code === "temporarily_blocked");

function albumsOf(tracks: Track[], artist?: string): Release[] {
  const groups = new Map<string, Track[]>();
  for (const t of tracks) {
    if (!t.album || norm(t.album) === norm(t.title)) continue;
    const a = primaryArtist(t.artist);
    if (artist && norm(a) !== norm(artist)) continue;
    const k = `${norm(a)}|${norm(t.album)}`;
    groups.set(k, [...(groups.get(k) ?? []), t]);
  }
  return [...groups.values()].map((g) => ({
    id: `${primaryArtist(g[0].artist)}|${g[0].album}`, title: g[0].album!, artist: primaryArtist(g[0].artist),
    kind: kindOf(g.length), year: null, cover_url: g.find((t) => t.cover_url)?.cover_url ?? null, tracks: null,
  }));
}

export class WebBackend implements Backend {
  readonly kind = "web" as const;
  private config: Config;
  private library: Library;
  private queue: QueueState;
  private player: PlayerState;
  private state: BackendState = { kind: "connecting" };
  private subs = new Map<string, Set<(p: any) => void>>();
  private handlers: Record<string, Handler>;
  private engine: WebEngine;
  /** The current song's resolved source is loaded in the engine. */
  private loaded = false;
  /** Bumped on every new song; late answers for an old one are dropped. */
  private gen = 0;
  private skips = 0;
  private radio = false;
  private search: { query: string; page: number; results: SearchResults; abort: AbortController } | null = null;
  private home: { at: number; feeds: HomeFeeds } | null = null;
  private artists = new Map<string, ArtistPage>();
  private lyricsOffsets: Record<string, number>;
  private stats: WebStats;
  private account: WebAccountManager;
  private connect: WebConnectManager;
  private mcp: WebMcpManager;
  private playTrackState: { key: string; listenedMs: number; countedPlay: boolean; lastPos: number } | null = null;
  private activeAccountId: string | null = null;
  private lastSavedPos = 0;
  private resolvedCache = new Map<string, { src: PlaybackSource | null; expiresAt: number }>();
  private resolving = new Map<string, Promise<PlaybackSource | null>>();
  private warmedCovers = new Set<string>();
  private coverCache = new Map<string, { url: string | null; expiresAt: number }>(loadCoverUrls());
  private coverInflight = new Map<string, { promise: Promise<string | null>; signals: (AbortSignal | undefined)[] }>();
  private coverQueue: { t: Track; key: string; signals: (AbortSignal | undefined)[]; resolve: (u: string | null) => void }[] = [];
  private coverActive = 0;
  private lastCoverReqAt = 0;
  private coversAvailable = true;
  private preloadTimer = 0;
  private preloadCooldownUntil = 0;
  private preloadSeq = 0;
  private playingFrom: ListKey | null = null;
  private lastMediaPosAt = 0;
  private lastMediaDur: number | null = null;

  /** localStorage had no library (too big for it): read it from IndexedDB first. */
  private libraryFromIdb = false;

  constructor() {
    this.config = { ...webConfig(), ...(load<Config>("config") ?? {}) };
    this.config.mcp = {
      enabled: Boolean(this.config.mcp?.enabled),
      mode: "server",
      port: this.config.mcp?.port ?? 0,
      token: this.config.mcp?.token ?? "",
      access: {
        playback: this.config.mcp?.access?.playback ?? "write",
        library: this.config.mcp?.access?.library ?? "read",
        stats: this.config.mcp?.access?.stats ?? "none",
        settings: this.config.mcp?.access?.settings ?? "write",
      },
    };
    const cachedLib = load<Library>("library");
    this.libraryFromIdb = !cachedLib;
    this.library = { ...emptyLibrary(), ...(cachedLib ?? {}) };
    this.lyricsOffsets = load<Record<string, number>>("lyrics_offsets") ?? {};
    this.activeAccountId = load<{ account?: { id?: string } }>("account")?.account?.id ?? null;
    this.queue = { items: [], index: null, user: 0 };
    const pc = this.config.player;
    this.player = {
      status: "idle", current: null, position: 0, duration: null,
      volume: pc.volume, muted: false, shuffle: pc.shuffle, repeat: (pc.repeat as PlayerState["repeat"]) ?? "off", source: null, preview: false,
    };
    this.loadSessionData(this.activeAccountId);
    this.stats = new WebStats(() => this.account?.scheduleSync(4000));
    this.account = new WebAccountManager({
      getConfig: () => this.config,
      setConfig: (c) => { this.config = c; save("config", this.config); this.emit("config", this.config); },
      getLibrary: () => this.library,
      setLibrary: (l) => { this.library = l; saveSoon("library", () => this.library); this.emit("library", this.library); },
      getLyricsOffsets: () => this.lyricsOffsets,
      setLyricsOffsets: (o) => { this.lyricsOffsets = o; save("lyrics_offsets", this.lyricsOffsets); },
      stats: this.stats,
      emit: (ev, p) => this.emit(ev, p),
      toast: (key, kind, vars) => this.toast(key, kind, vars),
      onSessionChange: () => { this.mcp?.sync(); this.connect?.sync(); },
      onBeforeAccountSwitch: (prevId) => {
        this.activeAccountId = prevId;
        this.saveSessionNow(prevId);
      },
      onAfterAccountSwitch: (nextId, signOut) => {
        this.activeAccountId = nextId;
        if (signOut || !this.player.current) {
          this.loaded = false;
          this.gen++;
          this.engine.stop();
          this.loadSessionData(nextId);
          this.emit("queue", this.queue);
          this.emit("player", { ...this.player });
        } else {
          this.saveSessionNow(nextId);
        }
      },
    });
    this.mcp = new WebMcpManager({
      account: this.account,
      stats: this.stats,
      getConfig: () => this.config,
      getLibrary: () => this.library,
      getQueue: () => this.queue,
      getPlayer: () => this.player,
      callHandler: async (cmd, args = {}) => this.handlers[cmd]?.(args),
      searchTracks: (q) => this.searchAll(q),
      getLyrics: (t) => this.lyrics(t),
      emit: (ev, p) => this.emit(ev, p),
      toast: (key, kind) => this.toast(key, kind),
    });
    this.connect = new WebConnectManager({
      isSignedIn: () => this.account.isSignedIn(),
      withToken: (fn) => this.account.withToken(fn),
      getLocalPlayer: () => this.player,
      isLocalPlaying: () => this.player.status === "playing" || this.player.status === "loading",
      getUpcoming: (limit) => this.upcomingQueueTracks(limit),
      emit: (ev, p) => this.emit(ev, p),
      onApplyCommand: (cmd) => this.applyRemoteCommand(cmd),
      onResumeLocal: (track, posSec, playing) => {
        this.queue = { items: [track], index: 0, user: 0, context: null };
        this.saveQueue();
        void this.start(track, playing, posSec);
      },
      pauseLocal: () => this.pauseAction(),
    });
    this.engine = new WebEngine({
      time: (position, duration) => {
        if (this.connect.isControlling()) return;
        const p = this.player;
        const prevPos = this.playTrackState?.lastPos ?? p.position;
        p.position = position;
        if (duration != null) p.duration = duration;
        if (this.playTrackState) this.playTrackState.lastPos = position;
        if (p.status === "playing" && p.current && this.playTrackState) {
          const deltaSec = position - prevPos;
          if (deltaSec > 0 && deltaSec < 5) {
            const addMs = deltaSec * 1000;
            this.playTrackState.listenedMs += addMs;
            if (this.config.stats.enabled) {
              this.stats.addTime(p.current, addMs);
              const durSec = p.duration ?? (p.current.duration_ms ? p.current.duration_ms / 1000 : 60);
              const threshMs = Math.max(5, Math.min(30, durSec / 2)) * 1000;
              if (!this.playTrackState.countedPlay && this.playTrackState.listenedMs >= threshMs) {
                this.playTrackState.countedPlay = true;
                this.stats.addPlay(p.current);
              }
            }
          }
        }
        if (p.loop_a != null && p.loop_b != null && position >= p.loop_b) this.engine.seek(p.loop_a);
        this.emit("player", { position, duration: p.duration });
        this.syncMediaPositionState(duration != null && duration !== this.lastMediaDur);
        if (
          p.status === "playing"
          && p.duration != null
          && p.duration - position <= 25
          && Date.now() >= this.preloadCooldownUntil
        ) {
          const next = this.upcomingQueueTracks(1)[0];
          if (next) {
            const cached = this.peekResolvedTrack(next);
            if (cached === undefined) {
              void this.preloadQueue(++this.preloadSeq);
            } else if (cached) {
              this.engine.preload(cached);
            }
          }
        }
        if (p.current && Math.abs(position - this.lastSavedPos) >= 5) {
          this.saveSessionNow();
        }
      },
      playing: (on) => {
        if (this.connect.isControlling()) return;
        this.setStatus(on ? "playing" : "paused");
        if (on) this.skips = 0;
        else this.saveSessionNow();
      },
      waiting: () => { if (!this.connect.isControlling() && this.player.status === "playing") this.setStatus("loading"); },
      ended: () => { if (!this.connect.isControlling()) this.advance(false, true); },
      error: (m) => { if (!this.connect.isControlling()) this.unplayable(m); },
    });
    this.engine.setVolume(this.player.volume, false);
    this.handlers = this.makeHandlers();
    this.mediaSession();
    if (this.player.current) this.metadata(this.player.current);
    if (typeof window !== "undefined") {
      const flush = () => this.saveSessionNow();
      window.addEventListener("pagehide", flush);
      window.addEventListener("beforeunload", flush);
      if (typeof document !== "undefined") {
        document.addEventListener("visibilitychange", () => {
          if (document.visibilityState === "hidden") {
            this.saveSessionNow();
          } else {
            this.emit("player", { ...this.player });
            this.syncMediaPlaybackState();
            this.syncMediaPositionState(true);
          }
        });
      }
    }
  }

  // ---- Backend ----
  async snapshot(): Promise<Snapshot> {
    if (this.libraryFromIdb) {
      this.libraryFromIdb = false;
      const lib = await loadBig<Library>("library");
      if (lib) this.library = { ...emptyLibrary(), ...lib };
    }
    try {
      const h = await server.health();
      this.state = h.protocol_version === 1 ? { kind: "ready", mode: "server" } : { kind: "incompatible", version: h.protocol_version };
      this.account.setAccountsAvailable(Boolean(h.accounts ?? h.capabilities?.includes("accounts") ?? true));
      this.coversAvailable = Boolean(h.covers ?? h.capabilities?.includes("covers") ?? true);
    } catch (e) {
      this.state = { kind: "offline", reason: String((e as Error)?.message ?? e) };
    }
    this.connect.sync();
    return wire({
      config: this.config, library: this.libraryWithTop(), queue: this.queue, player: this.player, backend: this.state,
      account: this.account.uiAccount(), version: VERSION, platform: "web", first_run: false, output_devices: [], update: null,
    } satisfies Snapshot);
  }

  async call<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
    const h = this.handlers[cmd];
    if (!h) { console.debug("[web] not in the browser:", cmd); return undefined as T; }
    return wire(await h(wire(args))) as T;
  }

  on<K extends keyof BackendEvents>(ev: K, cb: (p: BackendEvents[K]) => void) {
    if (!this.subs.has(ev)) this.subs.set(ev, new Set());
    this.subs.get(ev)!.add(cb);
    return () => this.subs.get(ev)?.delete(cb);
  }

  async cover(t: Track, signal?: AbortSignal): Promise<string | null> {
    if (signal?.aborted) return null;
    if (t.cover_url && !isYoutubeCover(t.cover_url)) return t.cover_url;
    if (!this.coversAvailable || Date.now() < this.preloadCooldownUntil || !t.title?.trim() || !t.artist?.trim()) {
      return t.cover_url ?? null;
    }
    const key = trackKey(t);
    const now = Date.now();
    const hit = this.coverCache.get(key);
    if (hit && hit.expiresAt > now) {
      this.coverCache.delete(key);
      this.coverCache.set(key, hit);
      return hit.url;
    }
    const pending = this.coverInflight.get(key);
    if (pending) {
      pending.signals.push(signal);
      return pending.promise;
    }

    const signals: (AbortSignal | undefined)[] = [signal];
    const promise = new Promise<string | null>((resolve) => {
      this.coverQueue.push({ t, key, signals, resolve });
      this.drainCoverQueue();
    }).finally(() => {
      this.coverInflight.delete(key);
    });
    this.coverInflight.set(key, { promise, signals });
    return promise;
  }

  trimCovers(keep = 48) {
    const cap = Math.max(0, keep);
    // Addresses are a few bytes each: keep enough of them that a reload or a
    // long scroll back does not ask the server again.
    while (this.coverCache.size > Math.max(400, cap)) {
      const oldest = this.coverCache.keys().next().value;
      if (oldest === undefined) break;
      this.coverCache.delete(oldest);
    }
    if (this.warmedCovers.size > cap) {
      const arr = [...this.warmedCovers];
      this.warmedCovers = new Set(arr.slice(Math.max(0, arr.length - cap)));
    }
  }

  private noteRateLimit(e: ApiError) {
    this.preloadCooldownUntil = Math.max(
      this.preloadCooldownUntil,
      Date.now() + Math.min(300, Math.max(5, e.retryAfter ?? 30)) * 1000,
    );
  }

  private drainCoverQueue() {
    while (this.coverActive < 2 && this.coverQueue.length > 0) {
      // Newest first: what is on screen now beats rows scrolled past long ago.
      const item = this.coverQueue.pop()!;
      if (item.signals.length > 0 && item.signals.every((s) => s?.aborted)) {
        item.resolve(null);
        continue;
      }
      if (!this.coversAvailable || Date.now() < this.preloadCooldownUntil) {
        item.resolve(item.t.cover_url ?? null);
        continue;
      }
      const gap = Math.max(0, 180 - (Date.now() - this.lastCoverReqAt));
      this.coverActive++;
      const run = () => {
        if (item.signals.length > 0 && item.signals.every((s) => s?.aborted)) {
          this.coverActive--;
          item.resolve(null);
          this.drainCoverQueue();
          return;
        }
        this.lastCoverReqAt = Date.now();
        server.cover(item.t)
          .then((res) => {
            const url = res?.url ?? item.t.cover_url ?? null;
            const album = res?.album?.trim() || null;
            if (album || (res?.url && !isYoutubeCover(res.url))) {
              this.backfillTrackMeta(item.t, album, res?.url ?? null);
            }
            if (this.coverCache.size >= 500) {
              const oldest = this.coverCache.keys().next().value;
              if (oldest !== undefined) this.coverCache.delete(oldest);
            }
            this.coverCache.set(item.key, {
              url,
              expiresAt: Date.now() + (url ? COVER_KEEP_MS : 2 * 60_000),
            });
            if (url) saveCoverUrlsSoon(this.coverCache);
            item.resolve(url);
          })
          .catch((e) => {
            if (isRateLimit(e)) {
              this.noteRateLimit(e);
              while (this.coverQueue.length > 0) {
                const queued = this.coverQueue.shift()!;
                queued.resolve(queued.t.cover_url ?? null);
              }
            }
            item.resolve(item.t.cover_url ?? null);
          })
          .finally(() => {
            this.coverActive--;
            this.drainCoverQueue();
          });
      };
      if (gap > 0) setTimeout(run, gap);
      else run();
    }
  }

  private backfillTrackMeta(target: Track, album?: string | null, coverUrl?: string | null): boolean {
    const k = trackKey(target);
    const cleanAl = album?.trim() || null;
    const cleanCv = coverUrl?.trim() && !isYoutubeCover(coverUrl.trim()) ? coverUrl.trim() : null;
    if (!cleanAl && !cleanCv) return false;
    const patch = (tr: Track) => {
      if (trackKey(tr) !== k) return false;
      let hit = false;
      if (cleanAl && !tr.album?.trim()) { tr.album = cleanAl; hit = true; }
      if (cleanCv && (!tr.cover_url?.trim() || isYoutubeCover(tr.cover_url))) { tr.cover_url = cleanCv; hit = true; }
      return hit;
    };
    let changed = false;
    for (const tr of this.library.liked) changed = patch(tr) || changed;
    for (const pl of this.library.playlists) {
      for (const tr of pl.tracks) changed = patch(tr) || changed;
    }
    for (const r of this.library.recent) {
      if (r.kind === "song") changed = patch(r.track) || changed;
    }
    if (changed) this.saveLibrary();
    return changed;
  }

  private computeHomeTop(): HomeTop {
    const sum = this.stats.summary("all");
    const songMs = new Map<string, { track: Track; ms: number; plays: number }>();
    for (const ts of sum.top_tracks) songMs.set(trackKey(ts.track), { track: ts.track, ms: ts.ms, plays: ts.plays });
    const listUses = new Map<string, number>();
    this.library.recent_lists.forEach((l, idx) => {
      const id = l.kind === "liked" ? "liked" : l.kind === "playlist" ? `pl:${l.id}` : l.kind === "album" ? `al:${norm(l.title)}|${norm(l.artist)}` : "";
      if (id) listUses.set(id, (this.library.recent_lists.length - idx) * 20_000);
    });
    const playlists = this.library.playlists
      .filter((p) => !p.shared_by)
      .map((p) => {
        let ms = 0;
        for (const tr of p.tracks) ms += songMs.get(trackKey(tr))?.ms ?? 0;
        const score = ms + (listUses.get(`pl:${p.id}`) ?? 0);
        return { kind: "list" as const, list: { kind: "playlist" as const, id: p.id }, score, ms };
      })
      .sort((a, b) => b.score - a.score)
      .slice(0, 6);

    const albumMap = new Map<string, { title: string; artist: string; cover: Track | null; ms: number; score: number }>();
    for (const al of this.library.saved_albums) {
      if (al.kind !== "album") continue;
      const k = `${norm(al.title)}|${norm(al.artist)}`;
      albumMap.set(k, { title: al.title, artist: al.artist, cover: al.cover ?? al.tracks[0] ?? null, ms: 0, score: 30_000 + (listUses.get(`al:${k}`) ?? 0) });
    }
    for (const ts of sum.top_tracks) {
      const al = ts.track.album?.trim();
      const ar = primaryArtist(ts.track.artist);
      if (!al || !ar) continue;
      const k = `${norm(al)}|${norm(ar)}`;
      const cur = albumMap.get(k) ?? { title: al, artist: ar, cover: ts.track, ms: 0, score: listUses.get(`al:${k}`) ?? 0 };
      if (!cur.cover) cur.cover = ts.track;
      cur.ms += ts.ms;
      cur.score += ts.ms;
      albumMap.set(k, cur);
    }
    const albums = [...albumMap.values()]
      .filter((a) => a.score > 0 || a.ms > 0)
      .sort((a, b) => b.score - a.score)
      .slice(0, 6)
      .map((a) => ({ kind: "list" as const, list: { kind: "album" as const, title: a.title, artist: a.artist, tracks: [], cover: a.cover }, score: a.score, ms: a.ms }));

    const artists = sum.top_artists.slice(0, 8).map((a) => {
      const pic = a.track ?? this.library.recent_artists?.find((x) => norm(x.name) === norm(a.name))?.track ?? this.library.liked.find((t) => norm(primaryArtist(t.artist)) === norm(a.name)) ?? null;
      return { kind: "artist" as const, name: a.name, track: pic, score: a.ms + a.plays * 30_000, ms: a.ms };
    });
    const songs = sum.top_tracks.slice(0, 8).map((x) => ({
      kind: "song" as const,
      track: x.track,
      score: x.ms + x.plays * 30_000,
      ms: x.ms,
    }));
    return { playlists, albums, artists, songs };
  }

  private libraryWithTop(): Library {
    return { ...this.library, home_top: this.computeHomeTop() };
  }

  private emit<K extends keyof BackendEvents>(ev: K, p: BackendEvents[K]) {
    this.subs.get(ev)?.forEach((cb) => cb(wire(p)));
  }
  private toast(key: string, kind: "ok" | "info" = "ok", vars?: Record<string, string | number>) {
    this.emit("toast", { text: key, key, vars, kind });
  }
  private saveLibrary() {
    saveSoon("library", () => this.library);
    this.emit("library", this.libraryWithTop());
    this.account.scheduleSync();
  }

  private loadSessionData(accountId: string | null) {
    let raw = load<WebPlaybackSession>(playbackStorageKey(accountId));
    if (!raw && !accountId) {
      const legacyQ = load<QueueState>("queue");
      const legacyP = load<{ position?: number }>("player");
      if (legacyQ && Array.isArray(legacyQ.items) && legacyQ.items.length) {
        raw = {
          queue: legacyQ,
          position: Number.isFinite(legacyP?.position) ? Math.max(0, legacyP!.position!) : 0,
          duration: null,
          track_key: null,
          listened_ms: 0,
          counted_play: false,
          saved_at: Date.now(),
        };
      }
    }
    const items = Array.isArray(raw?.queue?.items) ? raw!.queue.items : [];
    const idx = raw?.queue?.index != null && raw.queue.index >= 0 && raw.queue.index < items.length ? raw.queue.index : null;
    const user = idx != null ? Math.max(0, Math.min(raw?.queue?.user ?? 0, Math.max(0, items.length - idx - 1))) : 0;
    this.queue = { items, index: idx, user, context: raw?.queue?.context ?? null };
    const cur = idx != null ? items[idx] ?? null : null;
    const key = cur ? trackKey(cur) : null;
    const rawDur = raw?.duration != null && Number.isFinite(raw.duration) && raw.duration > 0 ? raw.duration : null;
    const dur = rawDur ?? (cur?.duration_ms ? cur.duration_ms / 1000 : null);
    let pos = cur && (!raw?.track_key || raw.track_key === key) && Number.isFinite(raw?.position) ? Math.max(0, raw!.position) : 0;
    if (dur != null && dur > 0 && pos >= Math.max(0, dur - 2)) pos = 0;
    const pc = this.config.player;
    this.player = {
      status: cur ? "paused" : "idle",
      current: cur,
      position: pos,
      duration: dur,
      volume: pc.volume,
      muted: this.player?.muted ?? false,
      shuffle: pc.shuffle,
      repeat: (pc.repeat as PlayerState["repeat"]) ?? "off",
      source: cur?.source ?? null,
      preview: false,
    };
    this.playTrackState = cur && key
      ? {
          key,
          listenedMs: raw?.track_key === key && Number.isFinite(raw.listened_ms) ? Math.max(0, raw.listened_ms) : 0,
          countedPlay: raw?.track_key === key ? Boolean(raw.counted_play) : false,
          lastPos: pos,
        }
      : null;
    this.lastSavedPos = pos;
    if (cur) this.metadata(cur);
  }

  private buildPlaybackSession(): WebPlaybackSession {
    const cur = this.player.current;
    const dur = this.player.duration ?? (cur?.duration_ms ? cur.duration_ms / 1000 : null);
    let pos = cur && Number.isFinite(this.player.position) ? Math.max(0, this.player.position) : 0;
    if (dur != null && dur > 0 && pos >= Math.max(0, dur - 2)) pos = 0;
    return {
      queue: this.queue,
      position: pos,
      duration: dur,
      track_key: cur ? trackKey(cur) : null,
      listened_ms: this.playTrackState?.listenedMs ?? 0,
      counted_play: this.playTrackState?.countedPlay ?? false,
      saved_at: Date.now(),
    };
  }

  private saveSessionNow(accountId = this.activeAccountId) {
    const key = playbackStorageKey(accountId);
    cancelSoon(key);
    const snap = this.buildPlaybackSession();
    this.lastSavedPos = snap.position;
    save(key, snap);
  }

  private saveSessionSoon(ms = 300) {
    const key = playbackStorageKey(this.activeAccountId);
    saveSoon(key, () => {
      const snap = this.buildPlaybackSession();
      this.lastSavedPos = snap.position;
      return snap;
    }, ms);
  }

  private saveQueue() {
    this.saveSessionNow();
    this.emit("queue", this.queue);
    this.scheduleQueuePreload();
  }

  private resolveParams() {
    const enabled = this.config.providers.enabled;
    const priority = this.config.providers.priority.filter((x) => WEB_SOURCES.includes(x) && enabled.includes(x));
    const youtubeOnly = Boolean(this.config.providers.youtube_only);
    return { enabled, priority, youtubeOnly };
  }

  private resolveCacheKey(t: Track, priority: ProviderId[], youtubeOnly: boolean): string {
    return `${trackKey(t)}\0${youtubeOnly ? "1" : "0"}\0${priority.join(",")}`;
  }

  private peekResolvedTrack(t: Track): PlaybackSource | null | undefined {
    const { enabled, priority, youtubeOnly } = this.resolveParams();
    if (!priority.length && !(youtubeOnly && enabled.includes("youtube"))) return null;
    const cacheKey = this.resolveCacheKey(t, priority, youtubeOnly);
    const cached = this.resolvedCache.get(cacheKey);
    if (!cached || cached.expiresAt <= Date.now()) return undefined;
    this.resolvedCache.delete(cacheKey);
    this.resolvedCache.set(cacheKey, cached);
    return cached.src;
  }

  private resolveTrack(t: Track, throwOnRateLimit = false): Promise<PlaybackSource | null> {
    const { enabled, priority, youtubeOnly } = this.resolveParams();
    if (!priority.length && !(youtubeOnly && enabled.includes("youtube"))) return Promise.resolve(null);
    const cacheKey = this.resolveCacheKey(t, priority, youtubeOnly);
    const now = Date.now();
    const cached = this.resolvedCache.get(cacheKey);
    if (cached && cached.expiresAt > now) {
      this.resolvedCache.delete(cacheKey);
      this.resolvedCache.set(cacheKey, cached);
      return Promise.resolve(cached.src);
    }
    const inflight = this.resolving.get(cacheKey);
    if (inflight) return inflight;

    const job = (async () => {
      try {
        const r = await server.resolve(t, priority, youtubeOnly);
        let src = direct(r.source);
        if (src?.kind === "embed" && src.provider !== "youtube") src = null;
        if (src && !enabled.includes(src.provider)) src = null;
        const ttlMs = src ? 15 * 60_000 : 60_000;
        if (this.resolvedCache.size >= 120) {
          const oldest = this.resolvedCache.keys().next().value;
          if (oldest !== undefined) this.resolvedCache.delete(oldest);
        }
        this.resolvedCache.set(cacheKey, { src, expiresAt: Date.now() + ttlMs });
        return src;
      } catch (e) {
        if (isRateLimit(e)) {
          this.noteRateLimit(e);
          if (throwOnRateLimit) throw e;
        } else if (e instanceof ApiError && e.status < 500 && e.status >= 400) {
          this.resolvedCache.set(cacheKey, { src: null, expiresAt: Date.now() + 60_000 });
        }
        return null;
      } finally {
        this.resolving.delete(cacheKey);
      }
    })();

    this.resolving.set(cacheKey, job);
    return job;
  }

  private warmCover(t: Track) {
    if (!t.cover_url || isYoutubeCover(t.cover_url) || typeof Image === "undefined" || this.warmedCovers.has(t.cover_url)) return;
    if (this.warmedCovers.size >= 150) this.warmedCovers.clear();
    this.warmedCovers.add(t.cover_url);
    const img = new Image();
    img.src = t.cover_url;
  }

  private upcomingQueueTracks(count = 3): Track[] {
    const q = this.queue;
    if (q.index == null || q.items.length <= 1) return [];
    const seen = new Set<string>();
    const cur = q.items[q.index];
    if (cur) seen.add(trackKey(cur));
    const out: Track[] = [];
    for (let i = q.index + 1; i < q.items.length && out.length < count; i++) {
      const t = q.items[i];
      const k = trackKey(t);
      if (!seen.has(k)) { seen.add(k); out.push(t); }
    }
    if (this.player.repeat === "all") {
      for (let i = 0; i < q.index && out.length < count; i++) {
        const t = q.items[i];
        const k = trackKey(t);
        if (!seen.has(k)) { seen.add(k); out.push(t); }
      }
    }
    return out;
  }

  private scheduleQueuePreload(delayMs = 450) {
    clearTimeout(this.preloadTimer);
    if (!this.queue.items.length || this.queue.index == null) return;
    const seq = ++this.preloadSeq;
    const effectiveDelay = typeof document !== "undefined" && document.hidden ? 0 : delayMs;
    if (effectiveDelay <= 0) {
      void this.preloadQueue(seq);
      return;
    }
    this.preloadTimer = window.setTimeout(() => {
      void this.preloadQueue(seq);
    }, effectiveDelay);
  }

  private async preloadQueue(seq: number) {
    if (seq !== this.preloadSeq || Date.now() < this.preloadCooldownUntil) return;
    if (typeof navigator !== "undefined" && navigator.onLine === false) return;
    const upcoming = this.upcomingQueueTracks(3);
    const p = this.player;
    const nearEnd = p.duration != null && p.duration - p.position <= 35;
    for (let i = 0; i < upcoming.length; i++) {
      if (seq !== this.preloadSeq || Date.now() < this.preloadCooldownUntil) return;
      const tr = upcoming[i];
      this.warmCover(tr);
      const src = await this.resolveTrack(tr, false);
      if (seq !== this.preloadSeq) return;
      if (i === 0 && src && nearEnd) this.engine.preload(src);
    }
  }

  private prefetchTrack(t: Track | undefined) {
    if (!t || Date.now() < this.preloadCooldownUntil) return;
    this.warmCover(t);
    void this.resolveTrack(t, false);
  }

  // ---- playback ----
  private syncMediaPlaybackState() {
    if (typeof navigator === "undefined" || !("mediaSession" in navigator)) return;
    const s = this.player.status;
    navigator.mediaSession.playbackState =
      s === "playing" || s === "loading" ? "playing" : s === "paused" ? "paused" : "none";
  }

  private syncMediaPositionState(force = false) {
    if (typeof navigator === "undefined" || !("mediaSession" in navigator)) return;
    const ms = navigator.mediaSession;
    if (typeof ms.setPositionState !== "function") return;
    const dur = this.player.duration;
    if (!this.player.current || dur == null || !Number.isFinite(dur) || dur <= 0) {
      if (force) {
        this.lastMediaDur = null;
        try { ms.setPositionState(); } catch { /* ignore */ }
      }
      return;
    }
    const now = typeof performance !== "undefined" ? performance.now() : Date.now();
    if (!force && this.lastMediaDur === dur && now - this.lastMediaPosAt < 1000) return;
    this.lastMediaPosAt = now;
    this.lastMediaDur = dur;
    try {
      ms.setPositionState({
        duration: dur,
        playbackRate: this.player.status === "playing" ? 1 : 0,
        position: Math.max(0, Math.min(this.player.position, dur)),
      });
    } catch { /* ignore */ }
  }

  private setStatus(s: PlayerState["status"]) {
    if (this.player.status === s) return;
    this.player.status = s;
    this.emit("player", { status: s, position: this.player.position });
    this.syncMediaPlaybackState();
    this.syncMediaPositionState(true);
  }

  private async loadResolved(gen: number, src: PlaybackSource, start: number) {
    try {
      await this.engine.load(src, true, start);
      if (gen === this.gen) {
        this.loaded = true;
        this.scheduleQueuePreload(typeof document !== "undefined" && document.hidden ? 0 : 250);
      }
    } catch (e) {
      if (gen === this.gen) this.unplayable(String((e as Error)?.message ?? e));
    }
  }

  /** Make `t` the current song and, with `play`, start it from `start` seconds. */
  private async start(t: Track, play: boolean, start = 0) {
    const gen = ++this.gen;
    const p = this.player;
    const key = trackKey(t);
    const resumingRestored = !this.loaded && this.playTrackState?.key === key && start > 0;
    const cached = play ? this.peekResolvedTrack(t) : undefined;
    this.loaded = false;
    if (!play || cached === null) {
      this.engine.stop();
    } else if (cached === undefined) {
      this.engine.beginTransition();
    }
    this.playTrackState = resumingRestored
      ? { ...this.playTrackState!, lastPos: start }
      : { key, listenedMs: 0, countedPlay: false, lastPos: start };
    Object.assign(p, {
      current: t,
      position: start,
      duration: t.duration_ms ? t.duration_ms / 1000 : (resumingRestored ? p.duration : null),
      source: cached ? cached.provider : t.source,
      preview: cached ? cached.kind === "preview" : false,
      error: null,
      loop_a: null,
      loop_b: null,
      status: play ? "loading" : "paused",
    });
    this.saveSessionNow();
    this.emit("player", { ...p });
    this.syncMediaPlaybackState();
    this.syncMediaPositionState(true);
    this.metadata(t);
    if (!play) return;
    this.addRecent(t);
    this.account.scheduleRecentUpdate();
    if (cached === null) return this.unplayable("no_source");
    if (cached !== undefined) {
      await this.loadResolved(gen, cached, start);
      return;
    }
    let src: PlaybackSource | null = null;
    try {
      src = await this.resolveTrack(t, true);
      if (gen !== this.gen) return;
    } catch (e) {
      if (gen !== this.gen) return;
      if (isRateLimit(e)) {
        this.engine.stop();
        this.toast("toast.server_out.offline", "info");
        this.setStatus("error");
        return;
      }
      src = null;
    }
    if (!src) {
      this.engine.stop();
      return this.unplayable("no_source");
    }
    p.source = src.provider;
    p.preview = src.kind === "preview";
    this.emit("player", { source: p.source, preview: p.preview });
    await this.loadResolved(gen, src, start);
  }

  /** No source, or the source failed: say so, then go on (sp.skip_unplayable) or stop. */
  private unplayable(reason: string) {
    const p = this.player;
    p.error = reason;
    this.loaded = false;
    const many = this.skips >= Math.min(5, this.queue.items.length - 1);
    if (this.config.playback.skip_unplayable && !many && this.hasNext()) {
      this.skips++;
      this.toast("toast.skipped", "info");
      this.advance(false, true);
      return;
    }
    this.skips = 0;
    this.engine.stop();
    this.toast(reason === "no_source" ? "toast.no_source" : "toast.skipped", "info");
    p.position = 0;
    p.status = "error";
    this.emit("player", { status: "error", error: reason, position: 0 });
    this.syncMediaPlaybackState();
    this.syncMediaPositionState(true);
  }

  private hasNext() {
    const q = this.queue;
    return q.index != null && (q.index + 1 < q.items.length || this.player.repeat === "all" || this.player.shuffle);
  }

  private advance(back: boolean, auto = false) {
    const q = this.queue, p = this.player;
    if (q.index == null || !q.items.length) return;
    if (!auto && !back && this.config.stats.enabled && this.playTrackState) {
      if (this.playTrackState.listenedMs > 1000 && this.playTrackState.listenedMs < 30_000 && !this.playTrackState.countedPlay) {
        this.stats.addSkip();
      }
    }
    if (auto && p.repeat === "one" && p.current) {
      if (this.playTrackState) {
        this.playTrackState.listenedMs = 0;
        this.playTrackState.countedPlay = false;
        this.playTrackState.lastPos = 0;
      }
      this.engine.seek(0);
      p.position = 0;
      this.syncMediaPositionState(true);
      this.engine.play();
      return;
    }
    let i = q.index + (back ? -1 : 1);
    if (p.shuffle && !back && q.items.length > 1) { do i = Math.floor(Math.random() * q.items.length); while (i === q.index); }
    if (i >= q.items.length) {
      if (p.repeat === "all") i = 0;
      else if (this.radio && p.current) { void this.extendRadio(p.current); return; }
      else {
        this.engine.pause();
        this.loaded = false;
        p.position = 0;
        this.emit("player", { position: 0 });
        this.setStatus("paused");
        this.saveSessionNow();
        return;
      }
    }
    if (i < 0) i = 0;
    q.index = i;
    q.user = Math.max(0, q.user - 1);
    this.saveQueue();
    void this.start(q.items[i], true);
  }

  /** Queue ran out with "continue with similar songs" on. */
  private async extendRadio(seed: Track) {
    const p = this.player;
    this.engine.beginTransition();
    this.loaded = false;
    p.position = 0;
    p.error = null;
    p.duration = seed.duration_ms ? seed.duration_ms / 1000 : p.duration;
    p.status = "loading";
    this.emit("player", { status: "loading", position: 0, duration: p.duration, error: null });
    this.syncMediaPlaybackState();
    this.syncMediaPositionState(true);
    const more = await server.similar(primaryArtist(seed.artist), seed.title).catch(() => [] as Track[]);
    const have = new Set(this.queue.items.map(trackKey));
    const add = more.filter((t) => !have.has(trackKey(t))).slice(0, 20);
    if (!add.length) {
      this.engine.stop();
      p.position = 0;
      p.error = "radio_empty";
      this.toast("toast.radio_empty", "info");
      this.emit("player", { position: 0, error: p.error });
      this.setStatus("error");
      return;
    }
    this.queue.items.push(...add);
    this.advance(false);
  }

  private playList(tracks: Track[], index: number, context: string | null, startSec = 0, play = true) {
    if (!tracks[index]) return;
    this.queue = { items: [...tracks], index, user: 0, context };
    this.saveQueue();
    void this.start(tracks[index], play, startSec);
  }

  private async applyRemoteCommand(cmd: PlaybackCommandWire) {
    switch (cmd.action) {
      case "transfer":
        this.playList(cmd.tracks, cmd.index ?? 0, null, (cmd.position_ms ?? 0) / 1000, cmd.playing);
        break;
      case "play":
        this.playAction();
        break;
      case "pause":
        this.pauseAction();
        break;
      case "stop":
        this.gen++;
        this.loaded = false;
        this.engine.stop();
        this.player.current = null;
        this.player.position = 0;
        this.player.duration = null;
        this.player.status = "idle";
        this.saveSessionNow();
        this.emit("player", { ...this.player });
        this.syncMediaPlaybackState();
        break;
      case "next":
        this.advance(false);
        break;
      case "previous":
        await this.handlers.previous({});
        break;
      case "seek":
        await this.handlers.seek({ position: (cmd.position_ms ?? 0) / 1000 });
        break;
      case "volume":
        await this.handlers.set_volume({ volume: cmd.volume });
        break;
    }
  }

  private playAction() {
    const p = this.player;
    if (!p.current || p.status === "playing") return;
    if (!this.loaded || p.status === "error") {
      const start = p.duration != null && p.position >= Math.max(0, p.duration - 0.5) ? 0 : p.position;
      void this.start(p.current, true, start);
      return;
    }
    this.engine.play();
  }

  private pauseAction() {
    const p = this.player;
    if (!p.current) return;
    if (!this.loaded) {
      this.gen++;
      this.engine.stop();
    } else {
      this.engine.pause();
    }
    this.setStatus("paused");
    this.saveSessionNow();
  }

  private togglePlay() {
    const p = this.player;
    if (!p.current) return;
    if (p.status === "playing" || p.status === "loading") {
      this.pauseAction();
      return;
    }
    this.playAction();
  }

  private addRecent(t: Track) {
    const k = trackKey(t);
    const from = this.playingFrom && this.playingFrom.kind !== "recent" ? this.playingFrom : undefined;
    const item: RecentItem = from ? { kind: "song", track: t, at: Date.now(), from } : { kind: "song", track: t, at: Date.now() };
    this.library.recent = [item, ...this.library.recent.filter((r) => !(r.kind === "song" && trackKey(r.track) === k))].slice(0, 100);
    this.saveLibrary();
  }

  /** Lock screen, headset buttons and media keys (Media Session API). */
  private mediaSession() {
    const ms = typeof navigator !== "undefined" ? navigator.mediaSession : undefined;
    if (!ms) return;
    const on = (a: MediaSessionAction, f: (d: MediaSessionActionDetails) => void) => { try { ms.setActionHandler(a, f); } catch { /* not supported */ } };
    on("play", () => this.playAction());
    on("pause", () => this.pauseAction());
    on("stop", () => this.pauseAction());
    on("nexttrack", () => this.advance(false));
    on("previoustrack", () => this.handlers.previous({}));
    on("seekto", (d) => d.seekTime != null && this.handlers.seek({ position: d.seekTime }));
    on("seekbackward", (d) => this.handlers.seek({ position: Math.max(0, this.player.position - (d.seekOffset ?? 10)) }));
    on("seekforward", (d) => this.handlers.seek({ position: this.player.position + (d.seekOffset ?? 10) }));
  }
  private metadata(t: Track) {
    if (!("mediaSession" in navigator) || typeof MediaMetadata === "undefined") return;
    const apply = (artworkUrl: string | null) => {
      navigator.mediaSession.metadata = new MediaMetadata({
        title: t.title,
        artist: t.artist,
        album: t.album ?? "",
        artwork: artworkUrl ? [{ src: artworkUrl }] : [],
      });
    };
    const directCover = t.cover_url && !isYoutubeCover(t.cover_url) ? t.cover_url : null;
    apply(directCover);
    if (!directCover) {
      void this.cover(t).then((u) => {
        if (u && this.player.current && trackKey(this.player.current) === trackKey(t)) apply(u);
      });
    }
  }

  // ---- data ----
  private async runSearch(query: string, page: number, progress: boolean): Promise<SearchResults> {
    if (!norm(query)) {
      const empty: SearchResults = { query, tracks: [], artists: [], albums: [], suggestions: [], lyric_hits: [], loading: false, more: false, errors: [] };
      if (progress) this.emit("search", empty);
      return empty;
    }
    const prev = this.search?.query === query ? this.search : null;
    if (!prev || page === 0) this.search?.abort.abort();
    const abort = prev && page > 0 ? prev.abort : new AbortController();
    const base: SearchResults = prev && page > 0 ? prev.results : { query, tracks: [], artists: [], albums: [], suggestions: [], lyric_hits: [], loading: true, more: false, errors: [] };
    const s = { query, page, results: { ...base, loading: true }, abort };
    this.search = s;
    let got = 0;
    const update = () => {
      const r = s.results;
      r.artists = this.artistsOf(r.tracks, query);
      r.albums = albumsOf(r.tracks).slice(0, 10);
      if (progress && this.search === s) this.emit("search", { ...r });
    };
    try {
      const enabled = this.config.providers.enabled;
      await server.search(query, page, (e: SearchEvent) => {
        if (e.event === "results") {
          const tracks = e.tracks.map((t) => filterTrackByEnabled(t, enabled)).filter((t): t is Track => t !== null);
          if (!tracks.length) return;
          got += tracks.length;
          mergeInto(s.results.tracks, tracks);
          update();
        } else if (e.event === "provider_error" && enabled.includes(e.provider)) {
          s.results.errors.push({ source: e.provider, message: e.message });
        }
      }, abort.signal);
      s.results.error = null;
    } catch (e) {
      if ((e as Error)?.name === "AbortError") return s.results;
      s.results.error = String((e as Error)?.message ?? e);
    }
    s.results.loading = false;
    s.results.more = got > 0 && page < 5;
    this.mcp.rememberSearch(s.results.tracks);
    update();
    return s.results;
  }

  /** Artists of the results: the ones named like the query first, then by how often they show up. */
  private artistsOf(tracks: Track[], query: string): SearchResults["artists"] {
    const q = norm(query);
    const count = new Map<string, { name: string; n: number; first: number; track: Track }>();
    tracks.forEach((t, i) => {
      const name = primaryArtist(t.artist);
      const k = norm(name);
      if (!k) return;
      const c = count.get(k);
      if (c) c.n++; else count.set(k, { name, n: 1, first: i, track: t });
    });
    return [...count.values()]
      .sort((a, b) => Number(!!q && q.includes(norm(b.name))) - Number(!!q && q.includes(norm(a.name))) || b.n - a.n || a.first - b.first)
      .slice(0, 8)
      .map((c) => ({ name: c.name, track: c.track, fans: null }));
  }

  /** Every result of one search (pages 0 and 1), without progress events. */
  private async searchAll(query: string): Promise<Track[]> {
    if (!norm(query)) return [];
    const out: Track[] = [];
    const ctl = new AbortController();
    const enabled = this.config.providers.enabled;
    const once = (page: number) => server.search(query, page, (e) => {
      if (e.event !== "results") return;
      const tracks = e.tracks.map((t) => filterTrackByEnabled(t, enabled)).filter((t): t is Track => t !== null);
      if (tracks.length) mergeInto(out, tracks);
    }, ctl.signal).catch(() => {});
    await once(0);
    if (out.length < 25) await once(1);
    return out;
  }

  private async artistPage(name: string): Promise<ArtistPage> {
    const k = norm(name);
    if (!k) return { name, tracks: [], loading: false, similar: [], releases: [], info: null };
    const hit = this.artists.get(k);
    if (hit) return hit;
    const [all, infoRes] = await Promise.all([
      this.searchAll(name),
      server.artist(name, this.config.language || "en").catch(() => null),
    ]);
    const tracks = all.filter((t) => norm(primaryArtist(t.artist)) === k || norm(t.artist).includes(k));
    const similar = tracks[0]
      ? await server.similar(name, tracks[0].title).then((s) => [...new Set(s.map((t) => primaryArtist(t.artist)).filter((a) => norm(a) !== k))].slice(0, 8), () => [] as string[])
      : [];
    const photo = infoRes?.photo ?? infoRes?.banner ?? tracks.find((t) => t.cover_url)?.cover_url ?? null;
    const banner = infoRes?.banner ?? infoRes?.photo ?? photo;
    const info: ArtistInfo = {
      name: infoRes?.name ?? name,
      bio: infoRes?.bio ?? null,
      photo: banner ?? photo,
      fans: infoRes?.fans ?? null,
      verified: Boolean(infoRes?.verified),
      listeners: null,
      tags: infoRes?.tags ?? [],
      links: infoRes?.links ?? (infoRes?.url ? [{ label: "Wikipedia", url: infoRes.url }] : []),
    };
    if (banner) {
      (info as any).banner = banner;
    }
    const page: ArtistPage = {
      name, tracks, loading: false, similar, releases: albumsOf(tracks, name),
      info,
    };
    this.artists.set(k, page);
    return page;
  }

  private async albumTracks(title: string, artist: string): Promise<Track[]> {
    const all = await this.searchAll(`${artist} ${title}`);
    const a = norm(primaryArtist(artist)), al = norm(title);
    return all.filter((t) => norm(t.album ?? "") === al && (norm(primaryArtist(t.artist)) === a || norm(t.artist).includes(a)));
  }

  private async homeFeeds(): Promise<HomeFeeds> {
    if (this.home && Date.now() - this.home.at < HOME_TTL) return this.home.feeds;
    const seeds: Track[] = [];
    for (const r of this.library.recent) if (r.kind === "song" && seeds.length < 2 && !seeds.some((s) => s.artist === r.track.artist)) seeds.push(r.track);
    for (const t of this.library.liked) if (seeds.length < 2 && !seeds.some((s) => s.artist === t.artist)) seeds.push(t);
    const [trending, ...similar] = await Promise.all([
      server.chart(),
      ...seeds.map((s) => server.similar(primaryArtist(s.artist), s.title).catch(() => [] as Track[])),
    ]);
    const known = new Set([...this.library.liked, ...seeds].map(trackKey));
    const per = new Map<string, number>();
    const recs: Track[] = [];
    for (let i = 0; recs.length < 24 && similar.some((l) => i < l.length); i++) {
      for (const l of similar) {
        const t = l[i];
        if (!t || known.has(trackKey(t))) continue;
        const a = norm(primaryArtist(t.artist));
        if ((per.get(a) ?? 0) >= 3) continue;
        per.set(a, (per.get(a) ?? 0) + 1);
        known.add(trackKey(t));
        recs.push(t);
      }
    }
    const feeds: HomeFeeds = { recs, trending, new_releases: [], loading: false };
    if (trending.length || recs.length) this.home = { at: Date.now(), feeds };
    return feeds;
  }

  private async lyrics(t: Track): Promise<Lyrics | null> {
    const l = await server.lyrics(t).catch(() => null);
    if (!l || l.instrumental) return null;
    const offset_ms = this.lyricsOffsets[trackKey(t)] ?? 0;
    if (l.synced.length) return { synced: true, lines: l.synced.map((x) => ({ time_ms: x.time_ms, text: x.text })), source: PROVIDER_NAME[l.provider] ?? l.provider, offset_ms };
    const plain = (l.plain ?? "").split(/\r?\n/);
    return plain.some((x) => x.trim()) ? { synced: false, lines: plain.map((text) => ({ time_ms: null, text })), source: PROVIDER_NAME[l.provider] ?? l.provider, offset_ms } : null;
  }

  private async radioFrom(seed: Track[], context: string) {
    const base = shuffled(seed).slice(0, 6);
    const lists = await Promise.all(base.slice(0, 2).map((t) => server.similar(primaryArtist(t.artist), t.title).catch(() => [] as Track[])));
    const tracks = mergeInto([...base], lists.flat());
    this.playList(tracks, 0, context);
  }

  // ---- links ----
  private pageUrl(path: string) {
    return `${location.origin}${import.meta.env.BASE_URL.replace(/\/$/, "")}/${path}`;
  }
  private async copyLink(link: string) {
    try { await navigator.clipboard.writeText(link); this.toast("toast.link_copied_plain"); } catch { /* no clipboard access */ }
    return link;
  }

  private makeHandlers(): Record<string, Handler> {
    const lib = () => this.library, q = () => this.queue, p = () => this.player;
    const playlist = (id: string) => lib().playlists.find((x) => x.id === id);
    const enc = encodeURIComponent;
    return {
      // ---- player ----
      play: (a) => {
        if ("from" in a && a.from !== undefined) this.playingFrom = a.from?.list ?? null;
        if (!this.connect.transferTracks(a.tracks, a.index ?? 0)) this.playList(a.tracks, a.index ?? 0, a.context ?? null);
      },
      play_recent: () => {
        this.playingFrom = null;
        const songs = lib().recent.flatMap((r) => (r.kind === "song" ? [r.track] : []));
        if (songs.length && !this.connect.transferTracks(songs, 0)) this.playList(songs, 0, null);
      },
      play_shuffled: (a) => {
        if ("from" in a && a.from !== undefined) this.playingFrom = a.from?.list ?? null;
        const list = shuffled(a.tracks as Track[]);
        if (this.connect.transferTracks(list, 0)) return;
        p().shuffle = true; this.emit("player", { shuffle: true }); this.playList(list, 0, a.context ?? null);
      },
      set_playing_from: (a) => {
        this.playingFrom = (a.list as ListKey | null) ?? null;
      },
      toggle_play: () => {
        if (this.connect.isControlling()) {
          const rp = this.connect.remotePlayerDto();
          this.connect.sendCommand({ action: rp?.status === "playing" ? "pause" : "play" });
          return;
        }
        this.togglePlay();
      },
      next: () => { if (!this.connect.sendCommand({ action: "next" })) this.advance(false); },
      previous: () => {
        if (this.connect.sendCommand({ action: "previous" })) return;
        if (p().position > 3 || q().index === 0) {
          this.engine.seek(0);
          p().position = 0;
          if (this.playTrackState) this.playTrackState.lastPos = 0;
          this.saveSessionNow();
          this.emit("player", { position: 0 });
          this.syncMediaPositionState(true);
        } else this.advance(true);
      },
      seek: (a) => {
        if (this.connect.sendCommand({ action: "seek", position_ms: Math.max(0, Math.round((a.position ?? 0) * 1000)) })) return;
        const dur = p().duration ?? (p().current?.duration_ms ? p().current!.duration_ms! / 1000 : null);
        const pos = dur != null ? Math.max(0, Math.min(a.position, dur)) : Math.max(0, a.position);
        p().position = pos;
        if (this.playTrackState) this.playTrackState.lastPos = pos;
        if (this.loaded) this.engine.seek(pos);
        this.saveSessionSoon(150);
        this.emit("player", { position: pos });
        this.syncMediaPositionState(true);
      },
      set_volume: (a) => {
        if (this.connect.sendCommand({ action: "volume", volume: a.volume })) return;
        p().volume = a.volume; p().muted = false; this.engine.setVolume(a.volume, false); this.config.player.volume = a.volume; save("config", this.config); this.emit("player", { volume: a.volume, muted: false });
      },
      toggle_mute: () => {
        if (this.connect.isControlling()) {
          const curVol = this.connect.remoteVolume() ?? 0.8;
          this.connect.sendCommand({ action: "volume", volume: curVol > 0.001 ? 0 : (this.config.player.volume || 0.8) });
          return;
        }
        p().muted = !p().muted; this.engine.setVolume(p().volume, p().muted); this.emit("player", { muted: p().muted });
      },
      set_shuffle: (a) => { p().shuffle = !!a.on; this.config.player.shuffle = !!a.on; save("config", this.config); this.emit("player", { shuffle: !!a.on }); this.scheduleQueuePreload(); },
      set_repeat: (a) => { p().repeat = a.mode; this.config.player.repeat = a.mode; save("config", this.config); this.emit("player", { repeat: a.mode }); this.scheduleQueuePreload(); },
      set_loop: (a) => { p().loop_a = a.a; p().loop_b = a.b; this.emit("player", { loop_a: a.a, loop_b: a.b }); },
      set_radio: (a) => { this.radio = !!a.on; },
      radio_state: () => ({ on: this.radio, ok: null }),
      set_sleep: () => undefined,
      // ---- queue ----
      queue_add: (a) => {
        const qq = q(), at = (qq.index ?? -1) + 1 + (a.next ? 0 : qq.user);
        qq.items.splice(at, 0, ...(a.tracks as Track[]));
        qq.user += (a.tracks as Track[]).length;
        if (qq.index == null) { qq.index = 0; void this.start(qq.items[0], false); }
        this.saveQueue();
      },
      queue_remove: (a) => { const qq = q(); qq.items.splice(a.index, 1); if (qq.index != null && a.index < qq.index) qq.index--; this.saveQueue(); },
      queue_move: (a) => {
        const qq = q(), cur = qq.index != null ? qq.items[qq.index] : null;
        const [it] = qq.items.splice(a.from, 1);
        qq.items.splice(a.to, 0, it);
        if (cur) qq.index = qq.items.indexOf(cur);
        this.saveQueue();
      },
      queue_clear: () => { const qq = q(); qq.items = qq.index != null ? [qq.items[qq.index]] : []; qq.index = qq.items.length ? 0 : null; qq.user = 0; this.saveQueue(); },
      queue_jump: (a) => { const qq = q(); if (!qq.items[a.index]) return; qq.index = a.index; this.saveQueue(); void this.start(qq.items[a.index], true); },
      // ---- library ----
      like: (a) => {
        const k = trackKey(a.track);
        lib().liked = a.on ? [a.track, ...lib().liked.filter((t) => trackKey(t) !== k)] : lib().liked.filter((t) => trackKey(t) !== k);
        this.saveLibrary();
      },
      playlist_create: (a) => {
        const id = `pl-${Date.now().toString(36)}`;
        lib().playlists.unshift({ id, name: a.name, tracks: a.tracks ?? [], created_at: Date.now(), local_only: !this.account.isSignedIn() });
        this.saveLibrary();
        return id;
      },
      playlist_rename: (a) => { const x = playlist(a.id); if (x) x.name = a.name; this.saveLibrary(); },
      playlist_delete: (a) => {
        const x = playlist(a.id);
        if (!x) return;
        lib().playlists = lib().playlists.filter((y) => y.id !== a.id);
        lib().playlist_trash = [{ id: x.id, name: x.name, playlist: x }, ...(lib().playlist_trash ?? [])].slice(0, 30);
        this.saveLibrary();
      },
      playlist_restore: (a) => {
        const it = (lib().playlist_trash ?? []).find((x) => x.id === a.id);
        if (!it?.playlist) return;
        lib().playlist_trash = (lib().playlist_trash ?? []).filter((x) => x.id !== a.id);
        lib().playlists.unshift(it.playlist);
        this.saveLibrary();
      },
      playlist_purge: (a) => { lib().playlist_trash = (lib().playlist_trash ?? []).filter((x) => x.id !== a.id); this.saveLibrary(); },
      playlist_add: (a) => { playlist(a.id)?.tracks.push(...a.tracks); this.saveLibrary(); },
      playlist_remove: (a) => { playlist(a.id)?.tracks.splice(a.index, 1); this.saveLibrary(); },
      playlist_move: (a) => { const x = playlist(a.id); if (x) { const [it] = x.tracks.splice(a.from, 1); x.tracks.splice(a.to, 0, it); } this.saveLibrary(); },
      playlist_remove_cover: (a) => { const x = playlist(a.id); if (x) { x.cover = null; this.saveLibrary(); } },
      list_insert: (a) => {
        const l = a.list as ListKey, tr = a.track as Track, i = a.index as number;
        if (l.kind === "playlist") playlist(l.id)?.tracks.splice(i, 0, tr);
        else if (l.kind === "liked") lib().liked.splice(i, 0, tr);
        else if (l.kind === "recent") lib().recent.splice(i, 0, { kind: "song", track: tr });
        this.saveLibrary();
      },
      pin: (a) => { lib().pinned = a.on ? [...lib().pinned, a.list] : lib().pinned.filter((l) => !sameList(l, a.list)); this.saveLibrary(); },
      save_album: (a) => { const l = { ...a.list, tracks: [] } as ListKey; lib().saved_albums = a.on ? [...lib().saved_albums, l] : lib().saved_albums.filter((x) => !sameList(x, l)); this.saveLibrary(); },
      follow: (a) => { lib().followed = a.on ? [...lib().followed, a.artist] : lib().followed.filter((x) => norm(x) !== norm(a.artist)); this.saveLibrary(); },
      list_opened: (a) => { lib().recent_lists = [a.list, ...lib().recent_lists.filter((l) => !sameList(l, a.list))].slice(0, 20); this.saveLibrary(); },
      recent_remove: (a) => { lib().recent.splice(a.index, 1); this.saveLibrary(); },
      recent_remove_track: (a) => { const k = trackKey(a.track); lib().recent = lib().recent.filter((r) => !(r.kind === "song" && trackKey(r.track) === k)); this.saveLibrary(); },
      hide_sidebar: (a) => { lib().hidden_sidebar.push(a.id); this.saveLibrary(); },
      hide_artist: (a) => {
        const n = norm(a.artist);
        const cur = (lib().hidden_artists ?? []).filter((x) => norm(x) !== n);
        lib().hidden_artists = a.on ? [...cur, a.artist] : cur;
        this.saveLibrary();
      },
      hidden_artists: () => lib().hidden_artists ?? [],
      toggle_hidden_artist: (a) => this.handlers.hide_artist({ artist: a.name, on: !(lib().hidden_artists ?? []).some((x) => norm(x) === norm(a.name)) }),
      search_history_add: (a) => { lib().search_history = [a.item, ...lib().search_history.filter((x) => JSON.stringify(x) !== JSON.stringify(a.item))].slice(0, 20); this.saveLibrary(); },
      search_history_remove: (a) => { lib().search_history.splice(a.index, 1); this.saveLibrary(); },
      recommend_feedback: () => undefined,
      // ---- data ----
      search: (a) => this.runSearch(String(a.query), 0, true),
      search_more: (a) => this.runSearch(String(a.query), (this.search && this.search.query === a.query ? this.search.page : 0) + 1, true),
      home: () => this.homeFeeds(),
      artist: (a) => this.artistPage(String(a.name)),
      artist_photo: (a) => this.artists.get(norm(String(a.name)))?.info?.photo ?? null,
      artist_verified: (a) => Boolean(this.artists.get(norm(String(a.name)))?.info?.verified),
      album: (a) => this.albumTracks(String(a.title), String(a.artist)),
      album_meta: () => null,
      lyrics: (a) => this.lyrics(a.track),
      lyrics_alternatives: () => [],
      lyrics_pick: () => undefined,
      lyrics_offset: (a) => {
        this.lyricsOffsets[trackKey(a.track)] = a.offset_ms;
        save("lyrics_offsets", this.lyricsOffsets);
        this.account.scheduleSync();
      },
      add_suggestions: async (a) => {
        const own = playlist(a.id)?.tracks ?? [];
        const seed = (own.length ? own : lib().liked).slice(-2);
        const have = new Set(own.map(trackKey));
        const lists = seed.length ? await Promise.all(seed.map((t) => server.similar(primaryArtist(t.artist), t.title).catch(() => [] as Track[]))) : [await server.chart()];
        return mergeInto([], lists.flat()).filter((t) => !have.has(trackKey(t))).slice(0, 50);
      },
      song_radio: (a) => this.radioFrom([a.track], a.title ?? a.track.title),
      artist_radio: async (a) => { const pg = await this.artistPage(a.artist); if (pg.tracks.length) await this.radioFrom(pg.tracks, a.artist); },
      list_radio: (a) => {
        const l = a.list as ListKey;
        const seed = l.kind === "playlist" ? playlist(l.id)?.tracks ?? [] : l.kind === "album" ? l.tracks : lib().liked;
        if (seed.length) return this.radioFrom(seed, "Radio");
      },
      set_config: (a) => {
        const wasStats = this.config.stats.enabled;
        this.config = a.config;
        this.config.mcp = {
          enabled: Boolean(this.config.mcp?.enabled),
          mode: "server",
          port: this.config.mcp?.port ?? 0,
          token: this.config.mcp?.token ?? "",
          access: {
            playback: this.config.mcp?.access?.playback ?? "write",
            library: this.config.mcp?.access?.library ?? "read",
            stats: this.config.mcp?.access?.stats ?? "none",
            settings: this.config.mcp?.access?.settings ?? "write",
          },
        };
        save("config", this.config);
        this.emit("config", this.config);
        if (wasStats && !this.config.stats.enabled) {
          this.stats.clear();
          this.toast("stats.deleted", "info");
          this.account.scheduleSync(200);
        }
        this.mcp.sync();
      },
      // ---- links ----
      // `copy: false` (phone share menu) only returns the link.
      share: async (a) => {
        const out = (link: string) => (a.copy === false ? link : this.copyLink(link));
        if (a.track) {
          if (this.config.sharing.glyph_links) {
            try {
              const token = this.account.isSignedIn() ? await this.account.validToken().catch(() => undefined) : undefined;
              const res = await server.shareCreate(
                { kind: "song", title: a.track.title, subtitle: a.track.artist, cover_url: a.track.cover_url ?? null, tracks: [a.track] },
                token,
              );
              return out(res.url);
            } catch { /* fall back to search link */ }
          }
          return out(this.pageUrl(`search?q=${enc(`${primaryArtist(a.track.artist)} ${a.track.title}`)}`));
        }
        const l = a.list as ListKey | undefined;
        if (l?.kind === "album") return out(this.pageUrl(`album/${enc(l.artist)}/${enc(l.title)}`));
        if (l?.kind === "playlist") {
          const pl = playlist(l.id);
          if (pl?.tracks.length) {
            try {
              const token = this.account.isSignedIn() ? await this.account.validToken().catch(() => undefined) : undefined;
              const res = await server.shareCreate(
                { kind: "playlist", title: pl.name, cover_url: pl.cover ?? pl.tracks[0]?.cover_url ?? null, tracks: pl.tracks },
                token,
              );
              return out(res.url);
            } catch { /* fall through */ }
          }
        }
        this.toast("web.app_only", "info");
        return null;
      },
      share_songs: async (a) => {
        const tracks = (a.tracks ?? []) as Track[];
        if (!tracks.length) return null;
        const out = (link: string) => (a.copy === false ? link : this.copyLink(link));
        try {
          const token = this.account.isSignedIn() ? await this.account.validToken().catch(() => undefined) : undefined;
          const res = await server.shareCreate(
            { kind: a.kind === "album" || a.kind === "song" ? a.kind : "playlist", title: String(a.title ?? "Glyph"), cover_url: tracks[0]?.cover_url ?? null, tracks },
            token,
          );
          return out(res.url);
        } catch {
          return out(this.pageUrl(`search?q=${enc(`${primaryArtist(tracks[0].artist)} ${tracks[0].title}`)}`));
        }
      },
      // Phone share menu: the browser's own share sheet, else copy.
      share_system: async (a) => {
        const text = String(a.text ?? "");
        const nav = navigator as Navigator & { share?: (d: ShareData) => Promise<void> };
        if (nav.share) {
          try {
            // Only the link, so the chat shows it as a link with its preview.
            await nav.share(/^https?:\/\//.test(text) ? { url: text } : { text });
            return;
          } catch (e) {
            if ((e as DOMException)?.name === "AbortError") return;
          }
        }
        await this.copyLink(text);
      },
      copy_text: async (a) => { await navigator.clipboard.writeText(String(a.text ?? "")); },
      copy_artist_link: (a) => this.copyLink(this.pageUrl(`artist/${enc(a.artist)}`)),
      open_url: (a) => {
        const url = String(a.url);
        // mailto: / sms: hand off to the phone's app; a new tab would stay blank.
        if (/^(mailto|sms):/.test(url)) location.href = url;
        else window.open(url, "_blank", "noopener,noreferrer");
      },
      import_text: () => this.toast("web.app_only", "info"),
      cover_path: (a) => a.track?.cover_url ?? null,
      track_album: async (a) => {
        const tr = a.track as Track | undefined;
        if (!tr || tr.album?.trim() || !this.coversAvailable || Date.now() < this.preloadCooldownUntil || !tr.title?.trim() || !tr.artist?.trim()) return null;
        try {
          const res = await server.cover(tr);
          const album = res?.album?.trim() || null;
          if (album || (res?.url && !isYoutubeCover(res.url))) {
            this.backfillTrackMeta(tr, album, res?.url ?? null);
          }
          return album;
        } catch (e) {
          if (isRateLimit(e)) this.noteRateLimit(e);
          return null;
        }
      },
      prefetch_hover: (a) => this.prefetchTrack(a.track as Track | undefined),
      onboarding_seen: () => save("onboarding_seen", true),
      diagnostics: () => `Glyph ${VERSION} (web) · ${navigator.userAgent}`,
      // ---- stats, account, profile & MCP ----
      stats_summary: (a) => (this.config.stats.enabled ? this.stats.summary((a.range as StatsRange) ?? "month") : null),
      stats_recap: (a) => (this.config.stats.enabled ? this.stats.recap(String(a.year ?? new Date().getFullYear())) : null),
      ...this.account.handlers(),
      ...this.mcp.handlers(),
      // ---- connect / remote playback ----
      devices_rescan: () => this.connect.rescan(),
      account_devices_open: (a) => this.connect.setPanelOpen(Boolean(a.open)),
      device_connect: (a) => this.connect.connect(String(a.id ?? "")),
      device_use_local: () => this.connect.useLocal(),
      device_select: () => this.connect.devicesJson(),
      // ---- not in a browser: answers that keep the UI calm ----
      download: (a) => {
        for (const t of (a.tracks ?? []) as Track[]) this.emit("download", { key: trackKey(t), progress: null, done: true });
        this.toast("web.app_only", "info");
      },
      downloaded_bytes: () => 0,
      output_devices: () => [],
      playback_status: () => ({ mpd: { connected: null, error: null }, devices: null }),
      providers_cooling: () => [],
      update_state: () => ({ checking: false, installing: null, installed: false, error: null, status: { kind: "none" } }),
    };
  }
}
