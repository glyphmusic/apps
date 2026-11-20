// Wire types of the settings pages (only what lib/api/types.ts does not
// already have). Field names stay snake_case, like serde writes them.

// ---- plugins (glyph_core::plugins) -----------------------------------
export interface PluginThemeColors { accent: number; glow: [number, number, number] | null; base: number }

/** A catalog entry (glyph_core::plugins::Listing) with its manifest. */
export interface PluginListing {
  id: string; name: string; author: string; description: string; version: string; license: string;
  min_app: string | null;
  /** "theme" or "extension". */
  kind: string;
  repo: string; path: string; rev: string;
  checked: boolean;
  stars?: number | null;
  /** ISO date of the last update. */
  updated?: string | null;
  /** Listing::installable(). */
  installable: "ok" | "needs_newer_app" | "broken";
  /** Has code (not data_only). */
  code: boolean;
  /** Permissions without "code": `playback`, `library`, ... and `network:<host>`. */
  permissions: string[];
  theme: PluginThemeColors | null;
  /** Settings sections it adds ("Title (page)"), menu names, number of texts, hidden row names. */
  sections: string[]; menus: string[]; strings: number; hide: string[];
  /** Credentials it needs, as readable lines. */
  setup: string[];
  /** What its look changes (font, corners, pictures, style sheet), if it has one. */
  look?: { font: string; radius: number | null; backgrounds: number; css: boolean; assets?: number; icons?: number } | null;
}

export interface PluginInstalled extends PluginListing {
  /** Dropped into the plugins folder by hand (a maker's plugin). */
  local: boolean;
  enabled: boolean;
  /** Has its own settings page (a tab under Plugins). */
  has_page: boolean;
  missing_credentials: boolean;
  /** Why its code stopped. */
  stopped: string | null;
  /** Last lines the code logged (local plugins). */
  log: string[];
  /** A newer version is in the catalog. */
  update: PluginListing | null;
}

export interface PluginCatalog { checked: PluginListing[]; community: PluginListing[]; partial: boolean }

export interface PluginItem {
  id: string;
  kind: "toggle" | "slider" | "select" | "text" | "button" | "link" | "progress" | "note";
  /** Already in the user's language and with the plugin's patches applied. */
  label: string; desc: string; caption: string; url: string; unit: string;
  min?: number; max?: number; step?: number;
  options: { value: string; label: string }[];
  value: unknown;
  status?: string | null;
  disabled?: boolean;
  secret?: boolean;
  configured?: boolean;
}

export interface PluginSection {
  plugin: string; plugin_name: string;
  /** Glyph page it joins ("playback", "playback.content", "audio", ..., "own"). */
  page: string;
  title: string;
  /** Label key of the Glyph row it goes under ("" = at the end). */
  after: string;
  items: PluginItem[];
}

export interface PluginUiState {
  /** Label keys of Glyph rows plugins hide. */
  hidden: string[];
  sections: PluginSection[];
  /** Plugins with their own settings page (tab order). */
  pages: { id: string; name: string }[];
  /** Texts plugins bring, by language. */
  strings: Record<string, Record<string, string>>;
  /** Looks of the enabled plugins (skin.ts). */
  looks?: import("../skin").PluginLook[];
}

// ---- account (glyph_core::account, glyph_protocol) -------------------
export type LibraryCrypto = "guest" | "starting" | "encrypted" | "escrow" | "locked";

export interface AccountDevice { id: string; name: string; platform: string; last_seen: number; current: boolean; approximate_location?: string | null }

export interface AccountStatus {
  /** The server offers accounts (null = not checked yet). */
  available: boolean | null;
  /** The configured server is the official one. */
  official: boolean;
  signed_in: boolean;
  handle: string;
  has_password: boolean;
  device_limit: number;
  crypto: LibraryCrypto;
  merge_pending: boolean;
  syncing: boolean;
  sync_error: string | null;
  /** Unix ms of the last sync, null = never. */
  last_sync: number | null;
  sync_pending: number;
  devices: AccountDevice[] | null;
  devices_error: string | null;
}

export interface LoginStart { login_id: string; user_code: string; url: string; expires_in_s: number }
export type LoginPoll = { state: "pending" | "device_limit" | "expired" } | { state: "done"; handle: string };

export interface PrivacyLine { area: string; level: "encrypted" | "local" | "decryptable" | "visible" | "public"; note: string | null }
export interface PrivacyEvent { at: number; code: string; detail: string; alarm: boolean }
export interface PrivacyReport { lines: PrivacyLine[]; events: PrivacyEvent[]; has_password: boolean }

export interface ProfileSettingsData {
  avatar: string | null; banner: string | null;
  public: boolean; show_recent: boolean;
  /** Ids of the playlists shown on the profile, in order. */
  playlists: string[];
  /** Order of profile sections ("playlists" | "recent"). */
  sections?: ("playlists" | "recent")[];
  /** Picture URLs (avatar, banner), if any. */
  avatar_url?: string | null; banner_url?: string | null;
}

// ---- storage ---------------------------------------------------------
export interface StorageStats {
  covers: number; downloads: number; lyrics: number; catalog: number; streams: number;
  data_dir: string; default_download_dir: string; local_songs: number;
}

// ---- yt-dlp ----------------------------------------------------------
export interface YtDlpStatus {
  path: string | null; version: string | null; js_runtime: string | null;
  can_install: boolean; managed: boolean;
  update_available: boolean | null; latest_version: string | null; update_error: string | null;
}

// ---- updates (UpdateStatus of the backend) ---------------------------
export type UpdateState =
  | { kind: "none" } | { kind: "up_to_date"; current: string } | { kind: "skipped" }
  | { kind: "available"; version: string; can_self_install: boolean; html_url: string; release_package: boolean };

// ---- connection / servers --------------------------------------------
export interface ConnTestResult { ok: boolean; text: string; auth_required?: boolean }

export interface DeviceDiscovery { found: number; searching: boolean; playing: string | null; error: string | null }
export interface MpdStatus { connected: string | null; error: string | null }
export interface McpSession {
  id: string;
  name: string;
  client_name: string;
  site?: string | null;
  kind: string;
  created_at: number;
  last_used_at?: number | null;
}
export interface McpInfo { url: string | null; token: string | null; config: string | null; error: string | null; relay_running: boolean; token_loading: boolean; sessions?: McpSession[] }

// ---- migrate (glyph_core::import::accounts) --------------------------
export type MigrateService = "spotify" | "youtube" | "soundcloud" | "deezer" | "lastfm";
export type MigrateTarget =
  | { kind: "liked" }
  | { kind: "playlist"; id: string }
  | { kind: "new_playlist"; name?: string | null };
export interface MigrateCollection { id: string; kind: "liked" | "playlist" | "albums" | "artists"; name: string; hint: string; count: number | null; followed: boolean; default_on: boolean }
export interface MigrateOverview { account: string; collections: MigrateCollection[]; notes: string[]; done_before: string[] }
export interface MigrateTotals { songs: number; matched: number; unsure: number; liked: number; playlists: number; albums: number; artists: number; failed: number }
export interface MigrateProgress {
  state: "idle" | "running" | "finished";
  index: number; lists: number; current: string; done: number; total: number;
  totals: MigrateTotals; failed: [string, string][]; notes: string[]; cancelled: boolean; stopping: boolean;
  /** Short progress text of the move ("[#####-----] 12/24"). */
  bar: string;
}
export interface MigrateResume { service: MigrateService; done: number; all: number; account: string }

/** How the backend is connected (server_info). */
export interface ServerInfo {
  kind: "server" | "direct" | "incompatible" | "offline";
  url?: string; version?: string; protocol?: number; reason?: string;
  /** The server wants an access token. */
  auth_required: boolean;
  /** Settings of the discord row: a built-in application id exists. */
  discord_builtin?: boolean;
}
