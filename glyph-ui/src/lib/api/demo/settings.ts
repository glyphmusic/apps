// Demo handlers of the settings pages (lib/settings/*). Made-up data only.
// The Rust / Tauri side implements the same commands. Settings themselves go
// through `set_config` ({ config }) which must also apply side effects (audio
// engine, MPD, Discord, MCP, glow, statistics, sources, ...) by comparing the
// old and the new config.
//
// NEW COMMANDS (name -> args -> result)
//
// General
//   open_url            { url }                              -> void   (system browser)
//   reveal_data_dir     {}                                   -> void   (file manager)
//   diagnostics         {}                                   -> string (version, OS, setup; no keys)
//   import_playlist_file {}                                  -> void   (file dialog, then import)
//   set_autostart       { on }                               -> void   (rejects with the error text)
//
// Playback
//   output_devices      {}                                   -> string[]
//   playback_status     {}                                   -> { mpd: { connected: string|null, error: string|null },
//                                                               devices: { found, searching, playing: string|null, error: string|null }|null }
//   radio_state         {}                                   -> { on: boolean, ok: boolean|null }
//   set_radio           { on }                               -> void
//   hidden_artists      {}                                   -> string[]
//   toggle_hidden_artist { name }                            -> void
//   providers_cooling   {}                                   -> ProviderId[]  (sources paused after rate limits)
//   ytdlp_status        { force }                            -> YtDlpStatus
//   ytdlp_install       {}                                   -> YtDlpStatus
//
// Equalizer
//   set_eq_live         { enabled, gains }                   -> void   (applies at once, does not save)
//   eq_code             { name, gains }                      -> string (share code)
//   eq_import           { text }                             -> { name, gains } | null  (code or .glyph-eq file text; rejects with the error)
//   eq_file             { name, gains }                      -> { name, text }  (file name + content)
//
// Server / integrations / privacy
//   server_info         {}                                   -> ServerInfo
//   server_test         { url }                              -> { ok, text, auth_required }
//   server_store_token  { url, token }                       -> void
//   retry_backend       {}                                   -> void
//   mcp_info            {}                                   -> McpInfo
//   mcp_rotate_token    {}                                   -> void
//   mcp_server_token    {}                                   -> void
//   privacy_report      {}                                   -> { lines, events, has_password }
//   privacy_clear       {}                                   -> void
//   privacy_record_web_login {}                              -> void
//
// Account (rejects with a short code: login_failed, handle_taken, ...)
//   account_status      {}                                   -> AccountStatus
//   account_login_start {}                                   -> { login_id, user_code, url, expires_in_s }
//   account_login_poll  { login_id }                         -> { state: "pending"|"device_limit"|"expired" } | { state: "done", handle }
//   account_password_submit { handle, password, register }   -> { handle, recovery_code? }
//   account_unlock_keys { password }                         -> void
//   account_change_password { password }                     -> void
//   account_link_passkey {}                                  -> { url }
//   account_rename      { handle }                           -> { handle }
//   account_approve     { code }                             -> { device_name, device_limit_reached }
//   account_recovery    {}                                   -> string (new recovery code)
//   account_remove_device { id }                             -> void
//   account_sign_out    {}                                   -> void   (also emits "account": null)
//   account_delete      { handle }                           -> void
//   account_sync_now    {}                                   -> void
//   account_merge       { yes }                              -> void
//
// Profile (settings of the signed-in account's profile)
//   profile_settings    {}                                   -> ProfileSettingsData
//   profile_pick_image  { banner }                           -> void   (file dialog + crop + upload)
//   profile_remove_image { banner }                          -> void
//   profile_update      { public?, show_recent? }            -> void
//   profile_toggle_playlist { id }                           -> void
//   profile_reorder_playlist { order, from, to }             -> void
//
// Storage
//   storage_stats       {}                                   -> StorageStats
//   storage_clear       { kind: covers|downloads|lyrics|catalog|streams } -> void
//   storage_set_download_dir { pick: true } | { dir: null }  -> void   (folder dialog, moves the files)
//
// Backup & share
//   settings_code       {}                                   -> string
//   settings_file       {}                                   -> { name, text }
//   settings_import     { text }                             -> { changed }  (rejects when invalid; keeps the old settings for undo)
//   settings_undo       {}                                   -> void
//
// Transfer music
//   migrate_resume_state {}                                  -> { service, done, all, account } | null
//   migrate_discard     {}                                   -> void
//   migrate_pasted      { text }                             -> [songs, lists]
//   migrate_overview    { service, profile? | pasted?, files?: {name,text}[] | resume? } -> MigrateOverview
//   migrate_start       { service, selected: string[] }      -> void   (runs in the backend)
//   migrate_progress    {}                                   -> MigrateProgress
//   migrate_cancel      {}                                   -> void
//
// Plugins (loaded locally from GitHub, never through glyph-server)
//   plugins_state       {}                                   -> { installed: PluginInstalled[], ui: PluginUiState }
//   plugins_catalog     { force }                            -> PluginCatalog
//   plugin_install      { id, repo, path, rev }              -> void (rejects with the error)
//   plugin_uninstall    { id }                               -> void
//   plugin_enable       { id, on }                           -> void
//   plugins_reload      {}                                   -> void
//   plugins_folder      {}                                   -> void   (creates + reveals the plugins folder)
//   plugin_set_value    { plugin, item, value }              -> void
//   plugin_action       { plugin, item }                     -> void
//
// Updates
//   update_state        {}                                   -> { checking, installing: [done,total]|null, installed, error, status: UpdateState }
//   update_check        { manual }                           -> UpdateState
//   update_install      {}                                   -> void
//   update_restart      {}                                   -> void
import type { DemoCtx, DemoHandler } from "../demo";
import type {
  AccountDevice, AccountStatus, MigrateCollection, MigrateOverview, MigrateProgress, MigrateResume, PluginCatalog, PluginInstalled, PluginListing, PluginVersion,
  PluginSection, PluginUiState, PrivacyReport, ProfileSettingsData, StorageStats, YtDlpStatus,
} from "../../settings/types";

const lag = (ms = 250) => new Promise((r) => setTimeout(r, ms));
const fail = (code: string): never => { throw new Error(code); };
const b64 = (o: unknown) => btoa(unescape(encodeURIComponent(JSON.stringify(o))));
const unb64 = (s: string) => JSON.parse(decodeURIComponent(escape(atob(s))));

// ---- demo state --------------------------------------------------------------
const st = {
  radio: { on: false, ok: true as boolean | null },
  hidden: [] as string[],
  mpdOn: false,
  ytdlp: { path: null, version: null, js_runtime: null, can_install: true, managed: false, update_available: null, latest_version: null, update_error: null } as YtDlpStatus,
  mcpToken: "demo-token-3f9a1c",
  mcpServerToken: null as string | null,
  events: [
    { at: Date.now() - 3600e3 * 5, code: "web_login", detail: "", alarm: false },
    { at: Date.now() - 86400e3 * 2, code: "web_login", detail: "", alarm: true },
  ],
  signedIn: false,
  handle: "demo_listener",
  hasPassword: true,
  devices: [
    { id: "d1", name: "Demo Laptop", platform: "Windows", last_seen: Date.now() - 60e3, current: true, approximate_location: "Demo City" },
    { id: "d2", name: "Demo Phone", platform: "Android", last_seen: Date.now() - 3600e3 * 3, current: false, approximate_location: null },
    { id: "d3", name: "Living Room Box", platform: "Linux", last_seen: Date.now() - 86400e3 * 4, current: false, approximate_location: "Sample Town" },
  ] as AccountDevice[],
  lastSync: null as number | null,
  polls: 0,
  profile: { avatar: null, banner: null, public: true, show_recent: false, playlists: ["pl-1", "pl-3"], sections: ["playlists", "recent"] } as ProfileSettingsData,
  storage: { covers: 38_400_000, downloads: 412_000_000, lyrics: 1_900_000, catalog: 7_300_000, streams: 96_000_000, data_dir: "C:\\Users\\demo\\AppData\\Roaming\\Glyph", default_download_dir: "C:\\Users\\demo\\Music\\Glyph", local_songs: 0 } as StorageStats,
  prevTheme: null as string | null,
  update: { checking: false, installing: null as [number, number] | null, installed: false, error: null as string | null, status: { kind: "none" } as unknown },
  resume: { service: "spotify", done: 3, all: 8, account: "demo_user" } as MigrateResume | null,
  run: null as null | { start: number; selected: MigrateCollection[]; service: string; cancelled: boolean },
  plugins: [] as PluginInstalled[],
  unfit: [] as PluginInstalled[],
  pluginValues: {} as Record<string, unknown>,
};

// ---- plugins -----------------------------------------------------------------
const L = (p: Partial<PluginListing> & Pick<PluginListing, "id" | "name">): PluginListing => ({
  author: "Sample Maker", description: "", version: "1.0.0", license: "MIT", min_app: "0.11.0", kind: "extension", repo: `sample-maker/${p.id}`, path: "", rev: "0123456789abcdef0123456789abcdef01234567",
  checked: true, stars: 12, updated: "2026-10-30T10:00:00Z", installable: "ok", code: false, permissions: [], theme: null, sections: [], menus: [], strings: 0, hide: [], setup: [], ...p,
});
const CATALOG: PluginCatalog = {
  partial: false,
  checked: [
    L({ id: "sunset-palette", name: "Sunset Palette", kind: "theme", description: "A warm orange and rose colour theme.", stars: 120, theme: { accent: 0xff7a59, glow: [0x3a1206, 0xe8553a, 0xffc9a0], base: 0x050302 } }),
    L({ id: "lyrics-tidy", name: "Lyrics Tidy", version: "1.2.0", description: "Cleans up lyric lines and adds a small option page.", stars: 64, strings: 4, sections: ["Lyrics (own page)"], changes: "Keeps blank lines between verses.",
      versions: [V("1.1.0", "a", "Faster on long songs."), V("1.0.0", "b", "")] }),
    L({ id: "pixel-skin", name: "Pixel Skin", kind: "theme", version: "2.0.0", min_app: "0.99.0", installable: "needs_newer_app", description: "Chunky pixel corners and a green glow.", stars: 22, theme: { accent: 0x5cff8a, glow: [0x04220e, 0x1fa34a, 0xb6ffcb], base: 0x020403 },
      versions: [V("1.4.0", "c", "Last version for Glyph 0.13.")], fitting: "1.4.0" }),
    L({ id: "gig-alerts", name: "Gig Alerts", description: "Tells you when followed artists announce a show.", stars: 31, code: true, permissions: ["library", "network:api.example.org"], setup: ["Gig service API key"] }),
  ],
  community: [
    L({ id: "tiny-visualizer", name: "Tiny Visualizer", author: "", description: "A small spectrum bar for the player.", checked: false, stars: 3, code: true, permissions: ["playback"] }),
    L({ id: "mono-paper", name: "Mono Paper", kind: "theme", description: "Black and white with one grey glow.", checked: false, stars: 8, theme: { accent: 0xd8d8d8, glow: null, base: 0x030303 } }),
  ],
};
function V(version: string, ch: string, changes: string): PluginVersion {
  return { version, rev: ch.repeat(40), min_app: "0.11.0", max_app: null, changes, code: false, permissions: [], fits: true };
}
const findL = (id: string) => [...CATALOG.checked, ...CATALOG.community].find((l) => l.id === id);
const newer = (a: string, b: string) => { const x = a.split(".").map(Number), y = b.split(".").map(Number); for (let i = 0; i < 3; i++) if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) > (y[i] ?? 0); return false; };
function asInstalled(l: PluginListing, extra: Partial<PluginInstalled> = {}): PluginInstalled {
  const cat = findL(l.id);
  return { ...l, local: false, enabled: true, has_page: l.sections.length > 0, missing_credentials: false, stopped: null, log: [], update: cat && newer(cat.version, l.version) ? cat : null, ...extra };
}
st.plugins = [asInstalled(L({ id: "lyrics-tidy", name: "Lyrics Tidy", version: "1.1.0", stars: 64, strings: 4, sections: ["Lyrics (own page)"] }), { installed_at: 1793000000, latest: "1.2.0" })];
const NEON_LOOK = { font: "rounded", radius: 1.6, backgrounds: 0, css: true, app_icon: true, icons: 2, splash: true, icon_names: ["heart", "heart-fill"],
  splash_art: ["  *  NEON  *  ", " ~~~~~~~~~~~~ ", "  *        *  "],
  preview: { app_icon: "https://raw.githubusercontent.com/glyphmusic/plugins/main/themes/aurora/app-icon.png", icons: { heart: "https://raw.githubusercontent.com/glyphmusic/plugins/main/themes/aurora/icons/heart.svg", "heart-fill": "https://raw.githubusercontent.com/glyphmusic/plugins/main/themes/aurora/icons/heart-fill.svg" }, backgrounds: [] } };
st.plugins.push(
  asInstalled(L({ id: "neon-night", name: "Neon Night", description: "Rounder corners, a rounded font, star hearts and its own app icon.", stars: 9, look: NEON_LOOK }), { installed_at: 1793500000 }),
  asInstalled(L({ id: "big-corners", name: "Big Corners", description: "Very round corners and a serif font.", stars: 4, look: { font: "serif", radius: 2, backgrounds: 0, css: false } }), { installed_at: 1793600000 }),
);
st.unfit = [asInstalled(L({ id: "classic-bar", name: "Classic Bar", version: "1.0.0", max_app: "0.12", installable: "needs_older_app", description: "The old sidebar look.", stars: 5, strings: 2 }), { enabled: false, installed_at: 1780000000 })];

function pluginUi(): PluginUiState {
  const active = st.plugins.filter((p) => p.enabled);
  const sections: PluginSection[] = [];
  const pages: { id: string; name: string }[] = [];
  for (const p of active) {
    if (p.id === "lyrics-tidy") {
      pages.push({ id: p.id, name: p.name });
      sections.push({
        plugin: p.id, plugin_name: p.name, page: "own", title: "Lyrics", after: "",
        items: [
          { id: "note", kind: "note", label: "", desc: "These options belong to the sample plugin.", caption: "", url: "", unit: "", options: [], value: null },
          { id: "trim", kind: "toggle", label: "Trim empty lines", desc: "Removes blank lines between verses.", caption: "", url: "", unit: "", options: [], value: (st.pluginValues[`${p.id}.trim`] as boolean) ?? true },
          { id: "size", kind: "slider", label: "Line size", desc: "", caption: "", url: "", unit: " px", min: 10, max: 30, step: 1, options: [], value: (st.pluginValues[`${p.id}.size`] as number) ?? 16 },
          { id: "mode", kind: "select", label: "Line style", desc: "", caption: "", url: "", unit: "", options: [{ value: "plain", label: "Plain" }, { value: "soft", label: "Soft" }], value: (st.pluginValues[`${p.id}.mode`] as string) ?? "plain" },
          { id: "name", kind: "text", label: "Nickname", desc: "Shown nowhere, just a demo field.", caption: "", url: "", unit: "", options: [], value: (st.pluginValues[`${p.id}.name`] as string) ?? "" },
          { id: "docs", kind: "link", label: "Plugin guide", desc: "", caption: "Open guide", url: "https://example.org/guide", unit: "", options: [], value: null },
        ],
      });
      sections.push({
        plugin: p.id, plugin_name: p.name, page: "playback", title: "", after: "sp.crossfade",
        items: [{ id: "gap", kind: "toggle", label: "Gapless lyrics", desc: "Keeps lyrics on screen between songs.", caption: "", url: "", unit: "", options: [], value: (st.pluginValues[`${p.id}.gap`] as boolean) ?? false }],
      });
    }
  }
  // Looks: font and corners of the demo plugins (sorted by name, like the app).
  const looks = active.filter((p) => p.look).sort((a, b) => a.name.localeCompare(b.name)).map((p) => ({ plugin: p.id, font: p.look!.font, radius: p.look!.radius, css: "" }));
  return { hidden: [], sections, pages, strings: {}, looks };
}

// ---- migrate -----------------------------------------------------------------
const COLS = (service: string): MigrateCollection[] => [
  { id: "liked", kind: "liked", name: "", hint: "", count: 128, followed: false, default_on: true },
  { id: "albums", kind: "albums", name: "", hint: "", count: 12, followed: false, default_on: true },
  { id: "artists", kind: "artists", name: "", hint: "", count: 21, followed: false, default_on: true },
  { id: "pl:a", kind: "playlist", name: "Late Night Coding", hint: `${service}/playlist`, count: 54, followed: false, default_on: true },
  { id: "pl:b", kind: "playlist", name: "Sunday Morning", hint: `${service}/playlist`, count: 23, followed: false, default_on: true },
  { id: "pl:c", kind: "playlist", name: "Road Trip", hint: `${service}/playlist`, count: 77, followed: true, default_on: false },
  { id: "pl:d", kind: "playlist", name: "", hint: `${service}/playlist · 37xDemoId`, count: null, followed: false, default_on: false },
];
const LIST_MS = 1400;
function migrateProgress(): MigrateProgress {
  const empty: MigrateProgress = {
    state: "idle", service: null, phase: "reading", index: 0, lists: 0, current: "", done: 0, total: 0, elapsed_ms: 0,
    totals: { songs: 0, matched: 0, unsure: 0, deferred: 0, liked: 0, playlists: 0, albums: 0, artists: 0, failed: 0 },
    items: [], failed: [], notes: [], cancelled: false, stopping: false, bar: "",
  };
  const r = st.run;
  if (!r) return empty;
  const n = r.selected.length, e = Date.now() - r.start;
  const idx = Math.min(n - 1, Math.floor(e / LIST_MS));
  const finished = r.cancelled || e >= n * LIST_MS;
  const upto = finished ? (r.cancelled ? idx : n) : idx;
  const totals = { songs: 0, matched: 0, unsure: 0, deferred: 0, liked: 0, playlists: 0, albums: 0, artists: 0, failed: 0 };
  const items = r.selected.map((c, i) => {
    const k = c.count ?? 10;
    const isDone = i < upto;
    const isCur = !finished && i === idx;
    const phase = ((e % LIST_MS) / LIST_MS) < 0.3 ? "reading" as const : "matching" as const;
    if (isDone) {
      totals.songs += k; totals.matched += Math.round(k * 0.94); totals.unsure += Math.round(k * 0.03);
      if (c.kind === "liked") totals.liked += k; else if (c.kind === "playlist") totals.playlists++; else if (c.kind === "albums") totals.albums += k; else totals.artists += k;
    }
    return {
      id: c.id, name: c.name || c.hint || c.kind, kind: c.kind, cover: c.cover ?? null,
      status: isDone ? "done" as const : isCur ? phase : finished && r.cancelled ? "skipped" as const : "queued" as const,
      count: k, added: isDone ? k : 0, matched: isDone ? Math.round(k * 0.94) : 0, total_songs: isDone ? k : 0, deferred: 0,
      playlist_id: null, error: null,
    };
  });
  const total = 24, done = Math.round(((e % LIST_MS) / LIST_MS) * total);
  const phase = ((e % LIST_MS) / LIST_MS) < 0.3 ? "reading" as const : "matching" as const;
  const bar = `[${"#".repeat(Math.round((done / total) * 10)).padEnd(10, "-")}] ${done}/${total}`;
  return {
    state: finished ? "finished" : "running", service: r.service as any, phase, index: idx, lists: n,
    current: r.selected[idx]?.name || r.selected[idx]?.hint || r.selected[idx]?.kind || "", done, total, elapsed_ms: e, totals, items,
    failed: finished ? [["Mix of the week", "This list is private."]] : [], notes: r.service === "spotify" ? ["spotify_mixes"] : [], cancelled: r.cancelled, stopping: false, bar,
  };
}

// ---- handlers ----------------------------------------------------------------
const acct = (ctx: DemoCtx) => ctx.emit("account", st.signedIn ? { handle: st.handle, name: "Demo Listener" } : null);
const privacy = (): PrivacyReport => ({
  has_password: st.hasPassword,
  lines: [
    { area: "library", level: st.signedIn ? "encrypted" : "local", note: null },
    { area: "settings", level: "visible", note: null },
    { area: "profile", level: "public", note: st.hasPassword ? null : "no_password" },
  ],
  events: st.events,
});
const status = (ctx: DemoCtx): AccountStatus => ({
  available: true, official: !ctx.config.server_url, signed_in: st.signedIn, handle: st.handle, has_password: st.hasPassword, device_limit: 5, crypto: "encrypted", merge_pending: false,
  syncing: false, sync_error: null, last_sync: st.lastSync, sync_pending: 0, devices: st.signedIn ? st.devices : null, devices_error: null,
});

export const handlers: Record<string, DemoHandler> = {
  open_url: (_, a) => { window.open(String(a.url), "_blank", "noopener"); },
  reveal_data_dir: () => {},
  diagnostics: (ctx) => `Glyph 0.12.4-demo\nOS: web\nBackend: demo\nLanguage: ${ctx.config.language || "en"}`,
  import_playlist_file: () => {},
  set_autostart: () => {},
  reveal_path: () => {},
  prefetch_hover: () => {},

  output_devices: () => ["System default", "Speakers (Demo)", "Headphones (Demo)", "Desk DAC (Demo)"],
  playback_status: (ctx) => ({
    mpd: ctx.config.mpd.enabled ? { connected: "127.0.0.1:6600", error: null } : { connected: null, error: null },
    devices: ctx.config.devices.discovery ? { found: 2, searching: false, playing: null, error: null } : null,
  }),
  radio_state: () => st.radio,
  set_radio: (_, a) => { st.radio.on = !!a.on; },
  hidden_artists: () => st.hidden,
  toggle_hidden_artist: (_, a) => { st.hidden = st.hidden.includes(a.name) ? st.hidden.filter((x) => x !== a.name) : [...st.hidden, a.name]; },
  providers_cooling: () => ["jamendo"],
  ytdlp_status: async (_, a) => { if (a.force) await lag(400); return st.ytdlp; },
  ytdlp_install: async () => { await lag(1500); st.ytdlp = { path: "demo/yt-dlp", version: "2026.10.01", js_runtime: "deno", can_install: true, managed: true, update_available: false, latest_version: "2026.10.01", update_error: null }; return st.ytdlp; },

  set_eq_live: () => {},
  eq_code: (_, a) => `GLYPH-EQ:${b64({ name: a.name, gains: a.gains })}`,
  eq_import: (_, a) => {
    const text = String(a.text).trim();
    try {
      const raw = text.startsWith("GLYPH-EQ:") ? unb64(text.slice(9)) : text.startsWith("{") ? JSON.parse(text) : null;
      return raw && Array.isArray(raw.gains) ? { name: String(raw.name ?? "Imported"), gains: raw.gains.slice(0, 10).map(Number) } : null;
    } catch { return null; }
  },
  eq_file: (_, a) => ({ name: `${String(a.name).replace(/[\\/:*?"<>|]/g, "-")}.glyph-eq`, text: JSON.stringify({ name: a.name, gains: a.gains }, null, 2) }),

  server_info: (ctx) => ({ kind: "server", url: ctx.config.server_url || "https://api.glyph.my", version: "0.11.0", auth_required: false, discord_builtin: true }),
  server_test: async (_, a) => { await lag(600); return /^https?:\/\//.test(String(a.url)) ? { ok: true, text: `Reachable · v0.11.0`, auth_required: false } : { ok: false, text: "Not reachable", auth_required: false }; },
  server_store_token: () => {},
  retry_backend: () => {},
  mcp_info: (ctx) => {
    const local = ctx.config.mcp.mode !== "server";
    const url = local ? `http://127.0.0.1:${ctx.config.mcp.port}/mcp` : st.signedIn ? "https://api.glyph.my/mcp" : null;
    const token = local ? st.mcpToken : st.mcpServerToken;
    return { url, token, config: url && token ? JSON.stringify({ mcpServers: { glyph: { url, headers: { Authorization: `Bearer ${token}` } } } }, null, 2) : null, error: null, relay_running: !local && st.signedIn, token_loading: false };
  },
  mcp_rotate_token: () => { st.mcpToken = `demo-token-${Math.random().toString(16).slice(2, 8)}`; },
  mcp_server_token: () => { st.mcpServerToken = `demo-server-${Math.random().toString(16).slice(2, 8)}`; },
  mcp_pending: () => [],
  mcp_approve: () => {},
  plugin_lyrics: () => [],
  share_save: () => null,
  privacy_report: () => privacy(),
  privacy_clear: () => { st.events = []; },
  privacy_record_web_login: () => {},

  account_status: (ctx) => status(ctx),
  account_login_start: () => { st.polls = 0; return { login_id: "demo-login", user_code: "DEMO-4821", url: "https://example.org/glyph-demo-login", expires_in_s: 120 }; },
  account_login_poll: (ctx) => {
    if (++st.polls < 3) return { state: "pending" };
    st.signedIn = true; st.lastSync = Date.now(); acct(ctx);
    return { state: "done", handle: st.handle };
  },
  account_password_submit: async (ctx, a) => {
    await lag(500);
    if (a.password === "wrong") fail("login_failed");
    if (a.register && String(a.password).length < 8) fail("password_too_short");
    st.signedIn = true; st.handle = String(a.handle).toLowerCase(); st.lastSync = Date.now(); acct(ctx);
    return { handle: st.handle, recovery_code: a.register ? "K7QD-M2XP-9WTE-4HFA" : null };
  },
  account_unlock_keys: () => {},
  account_change_password: (_, a) => { if (String(a.password).length < 8) fail("password_too_short"); st.hasPassword = true; },
  account_link_passkey: () => ({ url: "https://example.org/glyph-demo-passkey" }),
  account_rename: (ctx, a) => { st.handle = String(a.handle).toLowerCase(); acct(ctx); return { handle: st.handle }; },
  account_approve: (_, a) => (String(a.code).length < 4 ? fail("not_found") : { device_name: "Demo Tablet", device_limit_reached: false }),
  account_recovery: () => "R3NB-5GYT-H8LC-2VZK",
  account_remove_device: (_, a) => { st.devices = st.devices.filter((d) => d.id !== a.id); },
  account_sign_out: (ctx) => { st.signedIn = false; st.lastSync = null; acct(ctx); },
  account_sign_out_all: (ctx) => { st.signedIn = false; st.devices = []; st.lastSync = null; acct(ctx); },
  account_delete: (ctx) => { st.signedIn = false; acct(ctx); },
  account_sync_now: () => { st.lastSync = Date.now(); },
  account_merge: () => {},

  profile_settings: () => st.profile,
  profile_pick_image: (_, a) => { const p = st.profile; if (a.banner) { p.banner = "demo-banner"; p.banner_url = "data:image/svg+xml," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 40"><rect width="120" height="40" fill="hsl(300 50% 35%)"/><circle cx="90" cy="20" r="14" fill="hsl(330 80% 70%)"/></svg>'); } else { p.avatar = "demo-avatar"; p.avatar_url = "data:image/svg+xml," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><rect width="40" height="40" fill="hsl(200 60% 40%)"/><circle cx="20" cy="16" r="7" fill="hsl(40 90% 75%)"/></svg>'); } },
  profile_remove_image: (_, a) => { if (a.banner) { st.profile.banner = null; st.profile.banner_url = null; } else { st.profile.avatar = null; st.profile.avatar_url = null; } },
  profile_update: (_, a) => { Object.assign(st.profile, a); },
  profile_toggle_playlist: (_, a) => { const p = st.profile; p.playlists = p.playlists.includes(a.id) ? p.playlists.filter((x) => x !== a.id) : [...p.playlists, a.id]; },
  profile_reorder_playlist: (_, a) => { st.profile.playlists = (a.order as string[]).filter((x) => st.profile.playlists.includes(x)); },

  storage_stats: () => st.storage,
  storage_clear: (_, a) => { (st.storage as any)[a.kind] = 0; },
  storage_set_download_dir: (ctx, a) => { ctx.config.storage.download_dir = a.pick ? "D:\\Music\\Demo Downloads" : null; ctx.emit("config", ctx.config); },

  settings_code: (ctx) => `GLYPH-SETTINGS:${b64({ theme: ctx.config.visuals.theme, language: ctx.config.language })}`,
  settings_file: (ctx) => ({ name: "glyph-settings.json", text: JSON.stringify({ theme: ctx.config.visuals.theme, language: ctx.config.language }, null, 2) }),
  settings_import: (ctx, a) => {
    const text = String(a.text).trim();
    let o: any;
    try { o = text.startsWith("GLYPH-SETTINGS:") ? unb64(text.slice(15)) : JSON.parse(text); } catch { return fail("not a settings code"); }
    if (!o || typeof o !== "object") return fail("not a settings code");
    const changed = (o.theme && o.theme !== ctx.config.visuals.theme) || (o.language != null && o.language !== ctx.config.language);
    if (changed) { st.prevTheme = ctx.config.visuals.theme; if (o.theme) ctx.config.visuals.theme = o.theme; if (o.language != null) ctx.config.language = o.language; ctx.emit("config", ctx.config); }
    return { changed: !!changed };
  },
  settings_undo: (ctx) => { if (st.prevTheme) { ctx.config.visuals.theme = st.prevTheme; ctx.emit("config", ctx.config); } },

  migrate_resume_state: () => st.resume,
  migrate_discard: () => { st.resume = null; },
  migrate_pasted: (_, a) => { const n = (String(a.text).match(/https?:\/\//g) ?? []).length; return n ? [Math.max(1, n - 1), 1] : [0, 0]; },
  migrate_overview: async (_, a): Promise<MigrateOverview> => {
    await lag(900);
    const service = String(a.service);
    return { account: service === "lastfm" ? "demo_user" : "Demo Account", collections: COLS(service), notes: [], done_before: a.resume ? ["liked"] : [] };
  },
  migrate_start: (_, a) => {
    const all = COLS(String(a.service));
    const targets = (a.targets ?? {}) as Record<string, { kind: string }>;
    const order: Record<string, number> = { liked: 0, playlist: 1, albums: 2, artists: 3 };
    st.run = {
      start: Date.now(),
      service: String(a.service),
      cancelled: false,
      selected: all
        .filter((c) => (a.selected as string[]).includes(c.id))
        .map((c) => {
          const tgt = targets[c.id];
          if (!tgt) return c;
          if (tgt.kind === "liked") return { ...c, kind: "liked" as const };
          if (tgt.kind === "playlist" || tgt.kind === "new_playlist") return { ...c, kind: "playlist" as const };
          return c;
        })
        .sort((x, y) => order[x.kind] - order[y.kind]),
    };
  },
  migrate_progress: () => migrateProgress(),
  migrate_cancel: () => { if (st.run) st.run.cancelled = true; },

  plugins_state: () => ({ installed: st.plugins, unfit: st.unfit, ui: pluginUi() }),
  plugins_catalog: async (_, a) => { await lag(a.force ? 700 : 300); return CATALOG; },
  plugin_install: async (_, a) => {
    await lag(1000);
    const top = findL(String(a.id));
    if (!top) return fail("Plugin not found");
    // An earlier version by its commit, else the newest.
    const v = top.versions?.find((x) => x.rev === a.rev);
    const l: PluginListing = v ? { ...top, version: v.version, rev: v.rev, changes: v.changes, min_app: v.min_app, max_app: v.max_app, installable: "ok", versions: [], fitting: null } : top;
    st.plugins = [...st.plugins.filter((p) => p.id !== l.id), asInstalled(l, { installed_at: Math.floor(Date.now() / 1000), latest: top.version, update: v && top.installable === "ok" && newer(top.version, v.version) ? top : null })];
  },
  plugin_uninstall: async (_, a) => { await lag(400); st.plugins = st.plugins.filter((p) => p.id !== a.id); st.unfit = st.unfit.filter((p) => p.id !== a.id); },
  plugin_enable: (_, a) => { const p = st.plugins.find((x) => x.id === a.id); if (p) p.enabled = !!a.on; },
  plugins_reload: () => {},
  plugins_folder: () => {},
  plugin_set_value: (_, a) => { st.pluginValues[`${a.plugin}.${a.item}`] = a.value; },
  plugin_action: () => {},

  update_state: () => st.update,
  update_check: async () => { st.update.checking = true; await lag(900); st.update.checking = false; st.update.status = { kind: "up_to_date", current: "0.12.4-demo" }; return st.update.status; },
  update_install: async () => { await lag(500); st.update.installed = true; },
  update_restart: () => {},
};
