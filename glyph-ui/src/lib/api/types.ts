// Wire types shared with the Rust side (glyph-protocol / glyph-core, serde
// JSON). Field names stay snake_case as serde writes them.

export type ProviderId =
  | "audius" | "jamendo" | "archive" | "soundcloud" | "youtube" | "spotify" | "deezer" | "musicbrainz" | "lastfm" | "lrclib";

export const PROVIDER_BADGE: Record<ProviderId, string> = {
  audius: "AU", jamendo: "JA", archive: "IA", soundcloud: "SC", youtube: "YT", spotify: "SP", deezer: "DZ", musicbrainz: "MB", lastfm: "LF", lrclib: "LR",
};
export const PROVIDER_NAME: Record<ProviderId, string> = {
  audius: "Audius", jamendo: "Jamendo", archive: "Internet Archive", soundcloud: "SoundCloud", youtube: "YouTube", spotify: "Spotify", deezer: "Deezer", musicbrainz: "MusicBrainz", lastfm: "Last.fm", lrclib: "LRCLIB",
};

export interface Track {
  id: string;
  source: ProviderId;
  title: string;
  artist: string;
  album?: string;
  duration_ms?: number;
  cover_url?: string;
  isrc?: string;
  permalink?: string;
  external_ids?: Partial<Record<ProviderId, string>>;
  explicit?: boolean;
}

/** Track identity across sources (glyph_core::dedupe::track_key). */
export function trackKey(t: Track): string {
  return `${t.title.trim().toLowerCase()}|${t.artist.trim().toLowerCase()}`;
}

/** True when `url` (or its `/v1/upstream?url=` target) points to a YouTube image host. */
export function isYoutubeCover(url?: string | null): boolean {
  if (!url) return false;
  const ytHost = (host: string) => {
    const h = host.toLowerCase();
    return ["ytimg.com", "youtube.com", "ggpht.com"].some((s) => h === s || h.endsWith(`.${s}`));
  };
  try {
    const parsed = new URL(url, typeof location !== "undefined" ? location.origin : "https://glyph.my");
    if (parsed.pathname.endsWith("/v1/upstream")) {
      const inner = parsed.searchParams.get("url");
      if (inner) return ytHost(new URL(inner).hostname);
    }
    return ytHost(parsed.hostname);
  } catch {
    return false;
  }
}

export function sources(t: Track): ProviderId[] {
  const v: ProviderId[] = [t.source];
  for (const p of Object.keys(t.external_ids ?? {}) as ProviderId[]) if (p !== t.source) v.push(p);
  return v;
}

export type Rating = "clean" | "explicit" | "unknown";
export function rating(t: Track): Rating {
  if (t.explicit) return "explicit";
  const rated = sources(t).some((p) => p === "deezer" || p === "spotify");
  return t.source === "youtube" && !rated ? "unknown" : "clean";
}

/** Split a multi-artist credit string ("A, B", "A & B", "A feat. B") into unique artist names. */
export function splitArtists(s: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const part of s.split(/\s*(?:,(?!\s*the\s+creator\b)|&|\bfeat\.?|\bft\.?|\bx\b|\band\b|\bи\b)\s*/i)) {
    const name = part.trim();
    const key = name.toLowerCase();
    if (name && !seen.has(key)) {
      seen.add(key);
      out.push(name);
    }
  }
  return out.length ? out : s.trim() ? [s.trim()] : [];
}

/** First artist of "A, B", "A & B", "A feat. B" (dedupe::primary_artist). */
export function primaryArtist(s: string): string {
  return splitArtists(s)[0] ?? s;
}

export type ReleaseKind = "album" | "ep" | "single" | "compilation";

export type ListKey =
  | { kind: "liked" }
  | { kind: "recent" }
  | { kind: "playlist"; id: string }
  | { kind: "album"; title: string; artist: string; tracks: Track[]; cover?: Track | null; loading?: boolean; releaseKind?: ReleaseKind; expectedTracks?: number | null }
  | { kind: "artist_liked"; artist: string }
  | { kind: "downloads" };

export function sameList(a: ListKey | null | undefined, b: ListKey | null | undefined): boolean {
  if (!a || !b || a.kind !== b.kind) return false;
  switch (a.kind) {
    case "playlist": return a.id === (b as typeof a).id;
    case "album": return a.title.toLowerCase() === (b as typeof a).title.toLowerCase() && a.artist.toLowerCase() === (b as typeof a).artist.toLowerCase();
    case "artist_liked": return a.artist.toLowerCase() === (b as typeof a).artist.toLowerCase();
    default: return true;
  }
}

export function listId(k: ListKey): string {
  switch (k.kind) {
    case "playlist": return `playlist:${k.id}`;
    case "album": return `album:${k.artist}:${k.title}`;
    case "artist_liked": return `liked:${k.artist}`;
    default: return k.kind;
  }
}

export interface Playlist {
  id: string;
  name: string;
  tracks: Track[];
  /** Picture chosen by the user (data / asset URL). */
  cover?: string | null;
  description?: string | null;
  /** Shared with others / followed from someone. */
  owner?: string | null;
  public?: boolean;
  created_at?: number;
  /** Kept on this device only (not synced). */
  local_only?: boolean;
  /** New songs are downloaded for offline use. */
  auto_download?: boolean;
  /** Name of the person who shared it. */
  shared_by?: string | null;
}

export type RecentItem =
  | { kind: "song"; track: Track; at?: number; from?: ListKey }
  | { kind: "artist"; name: string; track?: Track | null; at?: number }
  | { kind: "list"; list: ListKey; at?: number };

export interface HomeTopList { kind: "list"; list: ListKey; score: number; ms: number }
export interface HomeTopSong { kind: "song"; track: Track; score: number; ms: number }
export interface HomeTopArtist { kind: "artist"; name: string; track?: Track | null; score: number; ms: number }
export interface HomeTop {
  playlists: HomeTopList[];
  albums: HomeTopList[];
  artists: HomeTopArtist[];
  songs: HomeTopSong[];
}

export type SearchItem =
  | { kind: "song"; track: Track }
  | { kind: "query"; text: string }
  | { kind: "artist"; name: string; track?: Track | null };

export interface Library {
  liked: Track[];
  /** Unix ms of the last "Clear Liked Songs" action (0 / unset = never). */
  liked_cleared_at?: number;
  recent: RecentItem[];
  playlists: Playlist[];
  saved_albums: ListKey[];
  pinned: ListKey[];
  recent_lists: ListKey[];
  followed: string[];
  downloads: Track[];
  search_history: SearchItem[];
  hidden_sidebar: string[];
  /** Deleted playlists: restore or purge. */
  playlist_trash?: { id: string; name: string; playlist?: Playlist }[];
  /** Older saved versions of playlists. */
  playlist_versions?: { revision: string; id: string; saved_at: string }[];
  /** "Don't show this artist". */
  hidden_artists?: string[];
  /** Track keys explicitly marked "Not interested". */
  disliked_tracks?: string[];
  /** Liked Songs download new songs automatically. */
  liked_auto_download?: boolean;
  /** Artist pages opened lately (sidebar), newest first. */
  recent_artists?: { name: string; track?: Track | null }[];
  /** How much each sidebar entry is used (list / artist id -> weight). */
  sidebar_weights?: Record<string, number>;
  /** Ranked candidates for Home's top cards (most listening time & activity). */
  home_top?: HomeTop;
}

export type RepeatMode = "off" | "all" | "one";
export type ShuffleStyle = "off" | "smart" | "random";
export type Status = "idle" | "loading" | "playing" | "paused" | "error";

export interface PlayerState {
  status: Status;
  current: Track | null;
  /** Seconds. */
  position: number;
  duration: number | null;
  volume: number;
  muted: boolean;
  shuffle: boolean;
  repeat: RepeatMode;
  /** Source the song plays from, `[YT]`. */
  source: ProviderId | null;
  preview: boolean;
  error?: string | null;
  /** A-B loop in seconds. */
  loop_a?: number | null;
  loop_b?: number | null;
}

export interface QueueState {
  items: Track[];
  /** Index of the current song in `items`. */
  index: number | null;
  /** Songs added with "play next" / "add to queue" (shown first). */
  user: number;
  /** Where the queue came from ("Liked Songs", an album, ...). */
  context?: string | null;
}

export interface LyricLine { time_ms: number | null; text: string }
export interface Lyrics {
  synced: boolean;
  lines: LyricLine[];
  /** Match quality 0..1 and the source. */
  score?: number;
  source?: string;
  offset_ms?: number;
}

export interface Release {
  id: string;
  title: string;
  artist: string;
  kind: ReleaseKind;
  year?: number | null;
  date?: string | null;
  cover_url?: string | null;
  tracks?: number | null;
}

export interface ArtistInfo {
  name: string;
  bio?: string | null;
  photo?: string | null;
  fans?: number | null;
  verified?: boolean;
  listeners?: number | null;
  tags?: string[];
  links?: { label: string; url: string }[];
}

export interface ArtistPage {
  name: string;
  tracks: Track[];
  loading: boolean;
  info: ArtistInfo | null;
  similar: string[];
  releases: Release[];
}

export interface DailyMix {
  id: string;
  kind: "genre" | "time" | "discovery" | string;
  title: string;
  subtitle: string;
  tags?: string[];
  accent_hue: number;
  tracks: Track[];
}

export interface HomeFeeds {
  recs: Track[];
  trending: Track[];
  new_releases: Release[];
  mixes?: DailyMix[];
  loading: boolean;
}

export interface TasteConfig {
  enabled: boolean;
  server_collab: boolean;
  metadata_enrichment: boolean;
  /** Beta: Glyph Drift Radio after the queue ends (apps only). */
  drift_radio?: boolean;
}

export interface WeightedTag {
  name: string;
  score: number;
  share_pct: number;
  feedback: number;
}

export interface VersionInsight {
  style: string;
  score: number;
  plays: number;
  skips: number;
  feedback: number;
}

export interface ListeningHabits {
  time_of_day: [number, number, number, number];
  peak_period: "morning" | "afternoon" | "evening" | "night";
  completion_rate: number;
  early_skip_rate: number;
  variety_score: number;
  total_plays: number;
  total_skips: number;
  total_replays?: number;
  versions: VersionInsight[];
}

export type ListeningMode =
  | "balanced"
  | "discovery"
  | "comfort"
  | "deep_focus"
  | "artist_dive"
  | "vibe_reset"
  | "high_energy"
  | "late_night_wind_down";

export interface InferredIntent {
  mode: ListeningMode;
  focus_genres: string[];
  focus_moods: string[];
  energy: number;
  novelty: number;
  vibe_pivot: boolean;
}

export interface ArtistAffinityInsight {
  name: string;
  score: number;
  reasons: string[];
  genres: string[];
}

export interface ArtistDislikeInsight {
  name: string;
  kind: "hidden" | "blocked" | "less" | "high_skips" | string;
  skips: number;
}

export interface CollabArtistRec {
  artist: string;
  score: number;
  because_of: string[];
  genres: string[];
  collaborative: boolean;
}

export interface TasteProfile {
  mode: "server" | "hybrid" | "local" | "off";
  confidence: number;
  signals_count: number;
  onboarding_done: boolean;
  top_genres: WeightedTag[];
  avoided_genres: WeightedTag[];
  moods: WeightedTag[];
  top_eras?: WeightedTag[];
  intent?: InferredIntent;
  habits: ListeningHabits;
  liked_artists: ArtistAffinityInsight[];
  disliked_artists: ArtistDislikeInsight[];
  disliked_tracks: Track[];
  onboarding_artists: string[];
  onboarding_tracks: Track[];
  onboarding_genres?: string[];
  collab_recs: CollabArtistRec[];
  collab_contributors: number;
  collab_model_ready: boolean;
}

export interface TasteRecommendResponse {
  recommendations: { track: Track; score: number; reason: string; because_of?: string[] }[];
  mixes?: DailyMix[];
  collab?: { recommendations: CollabArtistRec[]; contributors: number; model_ready: boolean };
  artist_tags?: { artist: string; genres?: string[]; moods?: string[]; eras?: string[]; tags?: string[]; similar?: string[] }[];
  intent?: InferredIntent;
  engine?: string;
}

export interface OnboardingArtist {
  name: string;
  genre: string;
  photo?: string | null;
  sample_track?: Track | null;
}

export interface OnboardingCatalogResponse {
  genres: string[];
  artists: OnboardingArtist[];
  tracks: Track[];
}

export interface SearchResults {
  query: string;
  tracks: Track[];
  artists: { name: string; track?: Track | null; fans?: number | null }[];
  albums: Release[];
  suggestions: string[];
  lyric_hits: { track: Track; line: string }[];
  loading: boolean;
  more: boolean;
  error?: string | null;
  errors: { source: ProviderId; message: string }[];
}

export type BackendState = { kind: "connecting" } | { kind: "ready"; mode: string } | { kind: "offline"; reason: string } | { kind: "incompatible"; version: number };

export interface Device {
  id: string;
  name: string;
  kind: "this" | "glyph" | "cast" | "airplay" | "upnp" | "account";
  active: boolean;
  /** Not reachable right now. */
  offline?: boolean;
}

export interface Account {
  handle: string;
  name?: string | null;
  avatar?: string | null;
  /** Signed in with data on this computer: merge it into the account or not. */
  merge_pending?: boolean;
  banner?: string | null;
  bio?: string | null;
  email?: string | null;
  syncing?: boolean;
  last_sync?: number | null;
}

export type ProfileSection = "playlists" | "recent";

export interface Profile {
  handle: string;
  name: string;
  avatar?: string | null;
  banner?: string | null;
  bio?: string | null;
  followers: number;
  following: number;
  playlists: Playlist[];
  top_artists: string[];
  top_tracks: Track[];
  sections?: ProfileSection[];
  is_me: boolean;
  follows: boolean;
}

export interface PluginInfo {
  id: string;
  name: string;
  author: string;
  description: string;
  version: string;
  kind: "theme" | "source" | "lyrics" | "integration" | "other";
  installed: boolean;
  enabled: boolean;
  checked: boolean;
  stars?: number;
  update?: string | null;
  repo: string;
}

export interface MonthStats {
  month: string;
  minutes: number;
  plays: number;
  top_tracks: { track: Track; plays: number }[];
  top_artists: { name: string; plays: number; track?: Track | null }[];
  by_hour: number[];
  by_day: number[];
}

export interface UpdateInfo { version: string; notes?: string; date?: string }

export interface EqPreset { name: string; gains: number[] }

/** glyph_core::Config (serde JSON). Only what the UI reads is typed. */
export interface Config {
  server_url: string;
  language: string;
  keys: Record<string, string | null>;
  providers: { local_search: boolean; youtube_mode: string; youtube_only: boolean; enabled: ProviderId[]; priority: ProviderId[]; extractor: string; ytdlp_path?: string | null; rdlp_path?: string | null };
  visuals: { intensity: number; music_reactivity: number; ascii_covers: boolean; startup_animation: boolean; ascii_effects: boolean; theme: string; throttle_unfocused: boolean; glow_shape: string; solid_color?: string; performance_mode?: boolean; render_mode?: string; gpu_blocked?: boolean; bg_image?: string; bg_dim?: number; bg_blur?: number };
  updates: { repo: string; check_on_start: boolean; auto_install: boolean; channel: string };
  audio: {
    crossfade_s: number; quality: string; download_quality: string; download_parallelism: number; download_adaptive: boolean; level: string; normalize: boolean;
    hide_explicit: boolean; channels: string; eq_enabled: boolean; eq_preset: string; eq_gains: number[]; eq_user_presets: EqPreset[]; eq_bass_db: number; eq_headroom_db: number; output_device: string | null;
  };
  storage: { cache_limit_mb: number; data_saver: boolean; download_dir: string | null };
  integrations: { discord_presence: boolean; discord_client_id: string };
  sharing: { glyph_links: boolean; profile_sections?: ProfileSection[] };
  player: { volume: number; shuffle: boolean; repeat: string; show_queue: boolean; mini_on_top: boolean; mini_large: boolean; mini_card_w?: number; mini_card_h?: number };
  playback: { shuffle: ShuffleStyle; skip_unplayable: boolean; patient_search: boolean; autoplay?: boolean; gapless?: boolean };
  lists: { sort: Record<string, string> };
  stats: { enabled: boolean };
  taste?: TasteConfig;
  mcp: { enabled: boolean; mode: string; port: number; token: string; access?: Partial<Record<"playback" | "library" | "stats" | "settings" | "plugins", "none" | "read" | "write">> };
  mpd: { enabled: boolean; address: string };
  devices: { discovery: boolean };
  plugins: { auto_update: boolean; skip_install_warning: boolean; sort: string; disabled: string[]; held?: string[] };
  desktop: { autostart: boolean; start_in_tray: boolean; close_to_tray?: boolean; clipboard_links?: boolean };
  /** Phone notifications (Android, notify.rs); missing in older configs. */
  notifications?: { app_updates: boolean; new_releases: boolean };
  ui: { wheel_seek_s: number; wheel_volume_pct: number; np_download: boolean; np_lyrics: boolean; np_lyric_line?: boolean; shake_feedback?: boolean; keys: Record<string, string>; reveal_downloads: boolean; text_scale: number; recommendations?: boolean; zen?: boolean; reduce_motion?: boolean };
  local: { folders: string[] };
  [k: string]: unknown;
}

export interface Snapshot {
  config: Config;
  library: Library;
  queue: QueueState;
  player: PlayerState;
  backend: BackendState;
  account: Account | null;
  version: string;
  platform: "windows" | "macos" | "linux" | "android" | "ios" | "web";
  first_run: boolean;
  output_devices: string[];
  update: UpdateInfo | null;
}

/** Events the backend pushes (Tauri `listen`). */
export interface BackendEvents {
  player: Partial<PlayerState>;
  queue: QueueState;
  library: Library;
  config: Config;
  backend: BackendState;
  search: SearchResults;
  home: HomeFeeds;
  artist: ArtistPage;
  lyrics: { key: string; lyrics: Lyrics | null; loading: boolean };
  /** `key`: an i18n key the UI translates (the Rust side sends keys, not words). */
  toast: { text: string; key?: string; vars?: Record<string, string | number>; sub?: string; kind?: "ok" | "info" | "warn";
    /** A button on the toast: `key` is its i18n text, `command` the backend command it calls (e.g. "reveal_path"). */
    action?: { key: string; command: string; args?: Record<string, unknown> } };
  download: { key: string; progress: number | null; done: boolean };
  devices: Device[];
  account: Account | null;
  /** The jam changed on its own (ended, left, taken over): the Jam snapshot (`jam_state`). */
  jam: { jam: unknown | null; waiting: string | null };
  /** A jam link was opened: show the Jam page of the Connect panel. */
  jam_open: Record<string, never>;
  /** Signed in with local data: the merge question (Settings > Account). */
  account_merge: { needed: boolean };
  /** Signed in (after the merge choice) or out: reload feeds; out also goes Home. */
  account_reset: { signed_in: boolean };
  /** The sleep timer ran out. */
  sleep: { state: "done" };
  update: UpdateInfo | null;
  /** A tapped notification (glyph://update, glyph://album/..): the page to show. */
  open_page: { settings?: string; album?: { artist: string; title: string } };
  /** The server takes this app version no more (or again): "Update Glyph". */
  app_outdated: { outdated: boolean; min: string | null; current: string };
  spectrum: [number, number, number];
  /** Text or link shared to Glyph (deep link, share sheet). */
  shared: string;
  /** Songs shared or opened with Glyph (an audio file on the phone): ask where they go. */
  pick_songs: { tracks: Track[]; file?: boolean };
  /** The share sheet reads a shared playlist: songs found so far. */
  share_progress: { done: number; total: number };
  /** A music link was copied in another app (desktop, window focused): offer to add it. */
  clipboard_link: string;
  /** The mini player window was opened or closed (Tauri). */
  mini: boolean;
  /** A plugin (or a glyph://plugin link) asks the window to open something. */
  plugin_ui:
    | { kind: "page"; id: string }
    | { kind: "search"; query: string }
    | { kind: "artist"; name: string }
    | { kind: "install"; id: string }
    /** An AI assistant changed the plugins (mcp_tools.rs plugins_changed). */
    | { kind: "changed"; what: "install" | "uninstall" | "enable" | "disable"; name: string };
  /** Lyrics a plugin published for the song with this key (the UI's trackKey). */
  plugin_lyrics: { key: string; lyrics: Lyrics };
  /** Plugins started or stopped: plugin lyrics may be gone. */
  plugin_lyrics_reset: null;
  /** An import saved a playlist: open it. */
  open_playlist: string;
  /** Reading a link / file: songs read or matched so far (0/0 when done). */
  import_progress: { done: number; total: number; phase?: "reading" | "matching" };
  /** Real-time progress snapshot of a running/finished library transfer. */
  migrate_progress: import("../settings/types").MigrateProgress;
  /** AI assistants waiting for the user's yes or no (relay mode). */
  mcp_approvals: { id: string; client: string; code: string }[];
  /** Emitted by the desktop memory guard when the process tree approaches its RAM cap. */
  memory_pressure: { level: "soft" | "hard" };
  /** 30-second preview playback state inside the "Not the right one" modal. */
  wrong_song_preview: WrongSongPreviewState;
}

export interface MatchAlternative {
  track: Track;
  confidence: number;
  preview_url?: string | null;
  tag?: string | null;
}

export interface MatchAlternativesResponse {
  current_source?: ProviderId | null;
  current_id?: string | null;
  alternatives: MatchAlternative[];
}

export interface WrongMatchReportResponse {
  recorded: boolean;
  promoted: boolean;
  votes: number;
  threshold: number;
}

export interface WrongSongPreviewState {
  key: string | null;
  status: "loading" | "playing" | "paused" | "stopped" | "error";
  position: number;
  duration: number;
  error?: string | null;
}

export type VideoKind = "official" | "live" | "lyric" | "topic" | "video";

export interface VideoCandidate {
  id: string;
  title: string;
  channel: string;
  duration_ms?: number | null;
  thumbnail?: string | null;
  kind: VideoKind;
}

export interface VideoSearchResponse {
  query: string;
  results: VideoCandidate[];
}

export interface VideoQuality {
  label: string;
  height: number;
  url: string;
  audio_url?: string | null;
  mime?: string | null;
}

export interface VideoResolveResponse {
  id: string;
  title?: string | null;
  channel?: string | null;
  duration_ms?: number | null;
  url: string;
  audio_url?: string | null;
  mime?: string | null;
  qualities?: VideoQuality[];
}
