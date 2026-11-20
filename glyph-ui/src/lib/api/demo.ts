// Demo backend: the whole UI in a plain browser, without the Rust core.
// Playback is a clock, data comes from demo-data.ts. More commands live in
// api/demo/*.ts (each exports `handlers`), so screens can add their own.
import type { Backend } from "./backend";
import type { BackendEvents, Config, HomeFeeds, Library, ListKey, PlayerState, QueueState, RecentItem, SearchResults, Snapshot, Track } from "./types";
import { sameList, trackKey } from "./types";
import { bgAdd, bgList, bgRemove } from "./bg-store";
import { ARTIST_META, ARTISTS, DEMO_NEW_RELEASES, TRACKS, byArtist, demoCover, demoLyrics, findAlbumDef, hash } from "./demo-data";

export interface DemoCtx {
  config: Config;
  library: Library;
  queue: QueueState;
  player: PlayerState;
  emit<K extends keyof BackendEvents>(ev: K, p: BackendEvents[K]): void;
  saveLibrary(): void;
}
/** `?admin` once in the address: signed in as a demo admin for this tab. */
function demoAdmin(): boolean {
  try {
    if (new URLSearchParams(location.search).has("admin")) sessionStorage.setItem("glyph-demo-admin", "1");
    return sessionStorage.getItem("glyph-demo-admin") === "1";
  } catch { return false; }
}
demoAdmin();

export type DemoHandler = (ctx: DemoCtx, args: Record<string, any>) => unknown;

const extra = import.meta.glob("./demo/*.ts", { eager: true }) as Record<string, { handlers?: Record<string, DemoHandler> }>;

export function demoConfig(): Config {
  return {
    server_url: "", language: "", keys: {},
    providers: { local_search: true, youtube_mode: "local", youtube_only: false, enabled: ["audius", "jamendo", "archive", "soundcloud", "youtube", "deezer"], priority: ["jamendo", "audius", "archive", "soundcloud", "youtube"], extractor: "r_dlp" },
    visuals: { intensity: 0.8, music_reactivity: 1, ascii_covers: false, startup_animation: false, ascii_effects: true, theme: "glyph", throttle_unfocused: true, glow_shape: "", performance_mode: false, render_mode: "", gpu_blocked: false },
    updates: { repo: "glyphmusic/apps", check_on_start: true, auto_install: false, channel: "stable" },
    audio: { crossfade_s: 0, quality: "auto", download_quality: "high", download_parallelism: 3, download_adaptive: true, level: "normal", normalize: true, hide_explicit: false, channels: "stereo", eq_enabled: false, eq_preset: "flat", eq_gains: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0], eq_user_presets: [], eq_bass_db: 0, eq_headroom_db: 0, output_device: null },
    storage: { cache_limit_mb: 2048, data_saver: false, download_dir: null },
    integrations: { discord_presence: true, discord_client_id: "" },
    sharing: { glyph_links: true, profile_sections: ["playlists", "recent"] },
    player: { volume: 0.8, shuffle: false, repeat: "off", show_queue: false, mini_on_top: true, mini_large: false },
    playback: { shuffle: "smart", skip_unplayable: true, patient_search: false },
    lists: { sort: {} },
    stats: { enabled: true },
    mcp: { enabled: false, mode: "read", port: 4917, token: "" },
    mpd: { enabled: false, address: "127.0.0.1:6600" },
    devices: { discovery: true },
    plugins: { auto_update: true, skip_install_warning: false, sort: "stars", disabled: [] },
    desktop: { autostart: false, start_in_tray: false },
    ui: { wheel_seek_s: 5, wheel_volume_pct: 5, np_download: true, np_lyrics: true, keys: {}, reveal_downloads: true, text_scale: 1, recommendations: true },
    local: { folders: [] },
  };
}

function demoLibrary(): Library {
  const pick = (...ix: number[]) => ix.map((i) => TRACKS[i % TRACKS.length]);
  return {
    liked: pick(0, 1, 4, 11, 18, 23, 30, 31),
    recent: [
      { kind: "song", track: TRACKS[0] }, { kind: "song", track: TRACKS[1] }, { kind: "song", track: TRACKS[4] },
      { kind: "song", track: TRACKS[11] }, { kind: "song", track: TRACKS[18] }, { kind: "song", track: TRACKS[23] },
      { kind: "artist", name: "Tame Impala", track: TRACKS[0] }, { kind: "list", list: { kind: "playlist", id: "pl-1" } },
    ],
    playlists: [
      { id: "pl-1", name: "Late Night Synth", tracks: pick(0, 4, 14, 21, 33, 37, 41, 31), created_at: Date.now() - 864e5 * 20 },
      { id: "pl-2", name: "Sunday Morning", tracks: pick(23, 24, 33, 34, 41, 43), created_at: Date.now() - 864e5 * 4 },
      { id: "pl-3", name: "Indie Classics", tracks: pick(1, 11, 17, 18, 19, 27, 30, 38), created_at: Date.now() - 864e5 },
    ],
    saved_albums: [], pinned: [{ kind: "playlist", id: "pl-1" }], recent_lists: [{ kind: "playlist", id: "pl-2" }],
    followed: ["Tame Impala", "Gorillaz", "Men I Trust"],
    downloads: pick(0, 1, 11, 23),
    search_history: [
      { kind: "song", track: TRACKS[0] },
      { kind: "query", text: "tame impala" },
      { kind: "artist", name: "Gorillaz", track: TRACKS[11] },
    ],
    hidden_sidebar: [],
  };
}

/** Copy a value the way Tauri's IPC does (JSON both ways), so the demo
 *  behaves like the real bridge and never holds on to UI state proxies. */
function wire<T>(v: T): T {
  return v === undefined ? v : (JSON.parse(JSON.stringify(v)) as T);
}

export class DemoBackend implements Backend, DemoCtx {
  readonly kind = "demo" as const;
  config = demoConfig();
  library = demoLibrary();
  queue: QueueState = { items: [], index: null, user: 0 };
  player: PlayerState = { status: "idle", current: null, position: 0, duration: null, volume: 0.8, muted: false, shuffle: false, repeat: "off", source: null, preview: false };
  private subs = new Map<string, Set<(p: any) => void>>();
  private handlers: Record<string, DemoHandler> = {};
  private timer = 0;

  constructor() {
    this.handlers = {
      ...this.core(),
      backgrounds_list: () => bgList(),
      background_add: (_, a) => bgAdd(a.data as string),
      background_remove: (_, a) => bgRemove(a.id as string),
    };
    for (const m of Object.values(extra)) Object.assign(this.handlers, m.handlers ?? {});
    // A song is loaded (paused), like a restored session.
    this.queue = { items: [...this.library.liked], index: 0, user: 0, context: "Liked Songs" };
    this.setCurrent(this.library.liked[0], false);
    this.player.position = 40;
    this.timer = window.setInterval(() => this.tick(), 250);
  }

  emit<K extends keyof BackendEvents>(ev: K, p: BackendEvents[K]) {
    this.subs.get(ev)?.forEach((cb) => cb(wire(p)));
  }
  on<K extends keyof BackendEvents>(ev: K, cb: (p: BackendEvents[K]) => void) {
    if (!this.subs.has(ev)) this.subs.set(ev, new Set());
    this.subs.get(ev)!.add(cb);
    return () => this.subs.get(ev)?.delete(cb);
  }
  saveLibrary() { this.emit("library", this.library); }

  async snapshot(): Promise<Snapshot> {
    return wire({
      config: this.config, library: this.library, queue: this.queue, player: this.player,
      backend: { kind: "ready", mode: "server" },
      // `?admin` signs in a demo admin (server dashboard, demo/admin.ts).
      account: demoAdmin() ? { handle: "demo" } : null,
      version: "0.12.4-demo", platform: "web",
      first_run: false, output_devices: ["System default", "Speakers (Demo)", "Headphones (Demo)"], update: null,
    } satisfies Snapshot);
  }

  async call<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
    const h = this.handlers[cmd];
    if (!h) { console.debug("[demo] no handler for", cmd, args); return undefined as T; }
    await new Promise((r) => setTimeout(r, 30 + Math.random() * 120));
    return wire(await h(this, wire(args))) as T;
  }

  async cover(t: Track) { return t.cover_url ?? null; }

  // ---- playback clock ----
  private tick() {
    const p = this.player;
    if (p.status !== "playing" || p.duration == null) return;
    p.position += 0.25;
    if (p.loop_a != null && p.loop_b != null && p.position >= p.loop_b) { p.position = p.loop_a; this.emit("player", { position: p.position }); }
    if (p.position >= p.duration) this.advance(false);
  }
  private setCurrent(t: Track | null, play: boolean) {
    const p = this.player;
    p.current = t; p.position = 0; p.duration = t?.duration_ms ? t.duration_ms / 1000 : null; p.source = t?.source ?? null;
    p.status = t ? (play ? "loading" : "paused") : "idle";
    this.emit("player", { ...p });
    if (t && play) setTimeout(() => { if (p.current === t) { p.status = "playing"; this.emit("player", { status: "playing", position: p.position }); } }, 450);
  }
  private advance(back: boolean) {
    const q = this.queue, p = this.player;
    if (q.index == null || !q.items.length) return;
    if (p.repeat === "one" && !back) { p.position = 0; this.emit("player", { position: 0 }); return; }
    let i = q.index + (back ? -1 : 1);
    if (p.shuffle && !back) i = Math.floor(Math.random() * q.items.length);
    if (i >= q.items.length) { if (p.repeat === "all") i = 0; else { p.status = "paused"; this.emit("player", { status: "paused" }); return; } }
    if (i < 0) i = 0;
    q.index = i; q.user = Math.max(0, q.user - 1);
    this.emit("queue", q);
    this.setCurrent(q.items[i], true);
    this.addRecent(q.items[i]);
  }
  private addRecent(t: Track) {
    this.library.recent = [{ kind: "song", track: t } as RecentItem, ...this.library.recent.filter((r) => !(r.kind === "song" && trackKey(r.track) === trackKey(t)))].slice(0, 50);
    this.saveLibrary();
  }

  private core(): Record<string, DemoHandler> {
    const q = () => this.queue, p = () => this.player, lib = () => this.library;
    const search = (query: string): SearchResults => {
      const s = query.toLowerCase();
      const tracks = TRACKS.filter((t) => `${t.title} ${t.artist} ${t.album}`.toLowerCase().includes(s));
      const artists = ARTISTS.filter((a) => a.toLowerCase().includes(s) || tracks.some((t) => t.artist === a)).map((name) => {
        const meta = ARTIST_META[name];
        const first = byArtist(name)[0];
        return { name, track: meta?.photo ? { ...first, cover_url: meta.photo } : first, fans: meta?.fans ?? (hash(name) % 90000) };
      });
      const albums = [...new Set(tracks.map((t) => `${t.artist}|${t.album}`))].map((k) => {
        const [artist, title] = k.split("|");
        const albTracks = byArtist(artist).filter((t) => t.album === title);
        const def = findAlbumDef(artist, title);
        return {
          id: k,
          title,
          artist,
          kind: def?.kind ?? "album" as const,
          year: def?.year ?? (2015 + (hash(k) % 10)),
          cover_url: def?.cover ?? albTracks[0]?.cover_url ?? demoCover(artist + title),
          tracks: albTracks.length,
        };
      });
      return { query, tracks, artists, albums, suggestions: tracks.slice(0, 6).map((t) => `${t.artist} - ${t.title}`), lyric_hits: [], loading: false, more: false, errors: [] };
    };
    return {
      play: (_, a) => {
        const tracks = a.tracks as Track[], i = a.index as number;
        this.queue = { items: [...tracks], index: i, user: 0, context: (a.context as string) ?? null };
        this.emit("queue", this.queue);
        this.setCurrent(tracks[i], true);
        this.addRecent(tracks[i]);
      },
      play_recent: () => { const songs = lib().recent.flatMap((r) => (r.kind === "song" ? [r.track] : [])); if (songs.length) this.handlers.play(this, { tracks: songs, index: 0 }); },
      toggle_play: () => { const pl = p(); if (!pl.current) return; pl.status = pl.status === "playing" ? "paused" : "playing"; this.emit("player", { status: pl.status, position: pl.position }); },
      next: () => this.advance(false),
      previous: () => { if (p().position > 3) { p().position = 0; this.emit("player", { position: 0 }); } else this.advance(true); },
      seek: (_, a) => { p().position = a.position; this.emit("player", { position: a.position }); },
      set_volume: (_, a) => { p().volume = a.volume; p().muted = false; this.emit("player", { volume: a.volume, muted: false }); },
      toggle_mute: () => { p().muted = !p().muted; this.emit("player", { muted: p().muted }); },
      set_shuffle: (_, a) => { p().shuffle = a.on; this.emit("player", { shuffle: a.on }); },
      set_repeat: (_, a) => { p().repeat = a.mode; this.emit("player", { repeat: a.mode }); },
      set_loop: (_, a) => { p().loop_a = a.a; p().loop_b = a.b; this.emit("player", { loop_a: a.a, loop_b: a.b }); },
      queue_add: (_, a) => {
        const qq = q(), at = (qq.index ?? -1) + 1 + (a.next ? 0 : qq.user);
        qq.items.splice(at, 0, ...(a.tracks as Track[])); qq.user += (a.tracks as Track[]).length;
        if (qq.index == null) qq.index = 0;
        this.emit("queue", qq);
      },
      queue_remove: (_, a) => { const qq = q(); qq.items.splice(a.index, 1); if (qq.index != null && a.index < qq.index) qq.index--; this.emit("queue", qq); },
      queue_move: (_, a) => {
        const qq = q(), cur = qq.index != null ? qq.items[qq.index] : null;
        const [it] = qq.items.splice(a.from, 1); qq.items.splice(a.to, 0, it);
        if (cur) qq.index = qq.items.indexOf(cur);
        this.emit("queue", qq);
      },
      queue_clear: () => { const qq = q(); qq.items = qq.index != null ? [qq.items[qq.index]] : []; qq.index = qq.items.length ? 0 : null; qq.user = 0; this.emit("queue", qq); },
      queue_jump: (_, a) => { q().index = a.index; this.emit("queue", q()); this.setCurrent(q().items[a.index], true); },
      like: (_, a) => {
        const k = trackKey(a.track);
        lib().liked = a.on ? [a.track, ...lib().liked.filter((t) => trackKey(t) !== k)] : lib().liked.filter((t) => trackKey(t) !== k);
        this.saveLibrary();
      },
      like_many: (_, a) => {
        const ks = new Set((a.tracks as Track[]).map(trackKey));
        const rest = lib().liked.filter((t) => !ks.has(trackKey(t)));
        lib().liked = a.on ? [...(a.tracks as Track[]), ...rest] : rest;
        this.saveLibrary();
      },
      playlist_create: (_, a) => { const id = `pl-${Date.now()}`; lib().playlists.unshift({ id, name: a.name, tracks: a.tracks ?? [], created_at: Date.now() }); this.saveLibrary(); return id; },
      playlist_rename: (_, a) => { const pl = lib().playlists.find((x) => x.id === a.id); if (pl) pl.name = a.name; this.saveLibrary(); },
      playlist_delete: (_, a) => { lib().playlists = lib().playlists.filter((x) => x.id !== a.id); this.saveLibrary(); },
      playlist_add: (_, a) => { const pl = lib().playlists.find((x) => x.id === a.id); pl?.tracks.push(...a.tracks); this.saveLibrary(); },
      playlist_remove: (_, a) => { const pl = lib().playlists.find((x) => x.id === a.id); pl?.tracks.splice(a.index, 1); this.saveLibrary(); },
      playlist_remove_many: (_, a) => {
        const pl = lib().playlists.find((x) => x.id === a.id);
        if (!pl) return;
        const drop = new Set<number>(a.indices);
        pl.tracks = pl.tracks.filter((_, i) => !drop.has(i));
        this.saveLibrary();
      },
      playlist_move: (_, a) => { const pl = lib().playlists.find((x) => x.id === a.id); if (pl) { const [it] = pl.tracks.splice(a.from, 1); pl.tracks.splice(a.to, 0, it); } this.saveLibrary(); },
      pin: (_, a) => { lib().pinned = a.on ? [...lib().pinned, a.list] : lib().pinned.filter((l) => !sameList(l, a.list)); this.saveLibrary(); },
      save_album: (_, a) => { const l = { ...a.list, tracks: [] } as ListKey; lib().saved_albums = a.on ? [...lib().saved_albums, l] : lib().saved_albums.filter((x) => !sameList(x, l)); this.saveLibrary(); },
      follow: (_, a) => { lib().followed = a.on ? [...lib().followed, a.artist] : lib().followed.filter((x) => x.toLowerCase() !== a.artist.toLowerCase()); this.saveLibrary(); },
      list_opened: (_, a) => { lib().recent_lists = [a.list, ...lib().recent_lists.filter((l) => !sameList(l, a.list))].slice(0, 20); },
      recent_remove: (_, a) => { lib().recent.splice(a.index, 1); this.saveLibrary(); },
      hide_sidebar: (_, a) => { lib().hidden_sidebar.push(a.id); this.saveLibrary(); },
      search: (_, a) => search(a.query),
      search_more: (_, a) => search(a.query),
      search_history_add: (_, a) => { lib().search_history = [a.item, ...lib().search_history].slice(0, 20); this.saveLibrary(); },
      search_history_remove: (_, a) => { lib().search_history.splice(a.index, 1); this.saveLibrary(); },
      home: (): HomeFeeds => ({
        recs: [27, 28, 29, 19, 20, 21, 33, 37, 41, 14, 32, 24].map((i) => TRACKS[i % TRACKS.length]),
        trending: [17, 12, 14, 2, 3, 5, 38, 39, 43, 34, 25, 15].map((i) => TRACKS[i % TRACKS.length]),
        new_releases: DEMO_NEW_RELEASES,
        loading: false,
      }),
      album: (_, a) => TRACKS.filter((t) => t.album?.toLowerCase() === String(a.title).toLowerCase() && t.artist.toLowerCase() === String(a.artist).toLowerCase()),
      artist: (_, a) => {
        const tracks = byArtist(a.name);
        const albums = [...new Set(tracks.map((t) => t.album!))];
        const meta = ARTIST_META[a.name];
        return {
          name: a.name, tracks, loading: false,
          info: {
            name: a.name,
            bio: meta?.bio ?? `${a.name} is an artist featured in Glyph.`,
            fans: meta?.fans ?? (hash(a.name) % 900000),
            verified: true,
            listeners: meta?.listeners ?? (hash(a.name) % 2_000_000),
            tags: meta?.tags ?? ["indie", "electronic", "dream pop"],
            photo: meta?.photo ?? tracks[0]?.cover_url,
          },
          similar: ARTISTS.filter((x) => x !== a.name).slice(0, 6),
          releases: albums.map((title, i) => {
            const albTracks = tracks.filter((t) => t.album === title);
            const def = findAlbumDef(a.name, title);
            return {
              id: `${a.name}-${i}`,
              title,
              artist: a.name,
              kind: def?.kind ?? "album",
              year: def?.year ?? (2015 + i),
              cover_url: def?.cover ?? albTracks[0]?.cover_url ?? demoCover(a.name + title),
              tracks: albTracks.length,
            };
          }),
        };
      },
      lyrics: (_, a) => ({ synced: true, lines: demoLyrics(a.track), score: 0.92, source: "LRCLIB", offset_ms: 0 }),
      set_config: (_, a) => { this.config = a.config; this.emit("config", this.config); },
      download: (_, a) => {
        for (const t of a.tracks as Track[]) {
          const key = trackKey(t);
          let f = 0;
          const id = window.setInterval(() => {
            f += 0.2;
            if (f >= 1) { clearInterval(id); lib().downloads.push(t); this.saveLibrary(); this.emit("download", { key, progress: 1, done: true }); }
            else this.emit("download", { key, progress: f, done: false });
          }, 300);
        }
      },
      download_remove: (_, a) => { lib().downloads = lib().downloads.filter((t) => trackKey(t) !== trackKey(a.track)); this.saveLibrary(); },
      share: async (_, a) => {
        const link = a.track ? `https://glyph.my/t/${a.track.id}` : "https://glyph.my/p/demo";
        if (a.copy === false) return link;
        try { await navigator.clipboard.writeText(link); } catch { /* preview without clipboard access */ }
        this.emit("toast", { text: a.track ? "Glyph link copied" : "Playlist code copied" });
        return link;
      },
      import_text: (_, a) => { this.emit("toast", { text: "Importing music", sub: String(a.text).slice(0, 40), kind: "info" }); },
      cover_path: (_, a) => a.track?.cover_url ?? null,
    };
  }

  destroy() { clearInterval(this.timer); }
}
