// MCP server-relay client for the browser player (/web).
// Mirrors glyph_tauri::mcp_tools in Server mode: polls `GET /v1/mcp/pull`,
// surfaces OAuth client approvals via `mcp_approvals`, executes allowed MCP
// tools against WebBackend, and posts results to `POST /v1/mcp/result`.
import type { StatsRange } from "../pages/stats/types";
import { LANGS } from "../i18n.svelte";
import { EQ_PRESETS, eqGains } from "../settings/eq";
import { THEMES } from "../theme";
import type { BackendEvents, Config, Library, Lyrics, PlayerState, ProfileSection, ProviderId, QueueState, Track } from "./types";
import { PROVIDER_NAME, primaryArtist, trackKey } from "./types";
import type { WebAccountManager } from "./web-account";
import { server, serverOrigin, wireTrack, type McpApproval, type PlaybackOutputWire } from "./web-server";
import type { McpSession } from "../settings/types";
import type { WebStats } from "./web-stats";
import { load, save } from "./web-store";

type Area = "playback" | "library" | "stats" | "settings";
type McpAccess = "none" | "read" | "write";
const norm = (s: string) => s.trim().toLowerCase();

const GLOW_SHAPES = ["auto", "strahl", "band", "sonnenaufgang", "horizont", "solid"] as const;
const QUALITIES = new Set(["auto", "low", "standard", "high"]);
const LEVELS = new Set(["loud", "normal", "quiet"]);
const SHUFFLE_STYLES = new Set(["smart", "random"]);
const REPEATS = new Set(["off", "all", "one"]);
const SORT_MODES = new Set(["custom", "title", "artist", "album", "duration"]);
const CHANNELS = new Set(["stable", "prerelease"]);
const PROFILE_SECTIONS: ProfileSection[] = ["playlists", "recent"];
const PROVIDERS = Object.keys(PROVIDER_NAME) as ProviderId[];

const DOCS_MCP_URL = "https://docs.glyph.my/mcp";

function toolArea(name: string): [Area, boolean] | null {
  switch (name) {
    case "now_playing":
    case "get_queue":
    case "get_lyrics":
    case "search":
    case "list_devices":
    case "recent_songs":
      return ["playback", false];
    case "play":
    case "pause":
    case "toggle_play":
    case "next":
    case "previous":
    case "seek":
    case "set_volume":
    case "set_shuffle":
    case "set_repeat":
    case "play_song":
    case "play_artist":
    case "queue_add":
    case "queue_remove":
    case "queue_move":
    case "queue_jump":
    case "queue_clear":
    case "play_playlist":
    case "start_radio":
    case "sleep_timer":
    case "play_on_device":
    case "select_device":
      return ["playback", true];
    case "list_playlists":
    case "get_playlist":
    case "list_followed":
      return ["library", false];
    case "create_playlist":
    case "rename_playlist":
    case "delete_playlist":
    case "add_to_playlist":
    case "remove_from_playlist":
    case "like":
    case "unlike":
    case "follow_artist":
      return ["library", true];
    case "listening_stats":
      return ["stats", false];
    case "get_settings":
      return ["settings", false];
    case "set_settings":
      return ["settings", true];
    default:
      return null;
  }
}

const ALL_TOOLS = [
  "now_playing",
  "play",
  "pause",
  "toggle_play",
  "next",
  "previous",
  "seek",
  "set_volume",
  "set_shuffle",
  "set_repeat",
  "search",
  "play_song",
  "play_artist",
  "queue_add",
  "queue_remove",
  "queue_move",
  "queue_jump",
  "queue_clear",
  "start_radio",
  "sleep_timer",
  "recent_songs",
  "list_devices",
  "play_on_device",
  "select_device",
  "get_queue",
  "list_playlists",
  "get_playlist",
  "play_playlist",
  "create_playlist",
  "rename_playlist",
  "delete_playlist",
  "add_to_playlist",
  "remove_from_playlist",
  "like",
  "unlike",
  "list_followed",
  "follow_artist",
  "get_lyrics",
  "listening_stats",
  "get_settings",
  "set_settings",
  "docs_search",
  "docs_read",
];

function effectiveAccess(cfg: Config): { playback: McpAccess; library: McpAccess; stats: McpAccess; settings: McpAccess } {
  const a = cfg.mcp.access;
  return {
    playback: a?.playback ?? "write",
    library: a?.library ?? "read",
    stats: a?.stats ?? "none",
    settings: a?.settings ?? "write",
  };
}

function allowsTool(cfg: Config, name: string): boolean {
  if (name === "docs_search" || name === "docs_read") return true;
  const info = toolArea(name);
  if (!info) return false;
  const [area, writes] = info;
  const perm = effectiveAccess(cfg)[area];
  if (perm === "none") return false;
  if (perm === "read") return !writes;
  return true;
}

function textResult(value: unknown, isError: boolean) {
  const text = typeof value === "string" ? value : JSON.stringify(value, null, 2);
  const r: Record<string, unknown> = {
    content: [{ type: "text", text }],
    isError,
  };
  if (value && typeof value === "object" && !Array.isArray(value)) {
    r.structuredContent = value;
  }
  return r;
}

function clientConfig(url: string, token: string): string {
  return JSON.stringify(
    {
      mcpServers: {
        glyph: {
          url,
          headers: { Authorization: `Bearer ${token}` },
        },
      },
    },
    null,
    2,
  );
}

export interface WebMcpHost {
  account: WebAccountManager;
  stats: WebStats;
  getConfig: () => Config;
  getLibrary: () => Library;
  getQueue: () => QueueState;
  getPlayer: () => PlayerState;
  callHandler: (cmd: string, args?: Record<string, any>) => Promise<any>;
  searchTracks: (query: string) => Promise<Track[]>;
  getLyrics: (t: Track) => Promise<Lyrics | null>;
  emit: <K extends keyof BackendEvents>(ev: K, p: BackendEvents[K]) => void;
  toast: (key: string, kind?: "ok" | "info") => void;
}

export class WebMcpManager {
  private refs = new Map<string, Track>();
  private lastSearch: Track[] = [];
  private relayGen = 0;
  private relayRunning = false;
  private relayAbort: AbortController | null = null;
  private error: string | null = null;
  private serverToken: { token: string; url: string } | null = null;
  private tokenLoading = false;
  private approvals: McpApproval[] = [];
  private sleepTimerId: ReturnType<typeof setTimeout> | null = null;
  private sleepPollId: ReturnType<typeof setInterval> | null = null;
  private sleepUntil: number | null = null;
  private sleepEndOfSong = false;

  constructor(private host: WebMcpHost) {
    this.serverToken = load<{ token: string; url: string }>("mcp_token") ?? null;
    setTimeout(() => this.sync(), 250);
  }

  rememberSearch(tracks: Track[]) {
    this.lastSearch = tracks.slice(0, 300);
    for (const t of this.lastSearch) this.refs.set(trackKey(t), t);
  }

  private rememberTrack(t: Track) {
    const k = trackKey(t);
    this.refs.set(k, t);
    const liked = this.host.getLibrary().liked.some((x) => trackKey(x) === k);
    return {
      ref: k,
      title: t.title,
      artist: t.artist,
      album: t.album ?? null,
      duration_s: t.duration_ms ? Math.round(t.duration_ms / 1000) : null,
      liked,
      source: t.source,
    };
  }

  private findTrackByRef(refStr: string): Track | null {
    const r = refStr.trim();
    if (!r) return null;
    const hit = this.refs.get(r);
    if (hit) return hit;
    const lib = this.host.getLibrary();
    const q = this.host.getQueue();
    const p = this.host.getPlayer();
    const pools: Track[][] = [
      p.current ? [p.current] : [],
      q.items,
      this.lastSearch,
      lib.liked,
      ...lib.playlists.map((pl) => pl.tracks),
      lib.recent.flatMap((x) => (x.kind === "song" ? [x.track] : [])),
    ];
    for (const list of pools) {
      const found = list.find((t) => trackKey(t) === r || `${t.source}:${t.id}` === r);
      if (found) {
        this.refs.set(trackKey(found), found);
        return found;
      }
    }
    return null;
  }

  sync() {
    const cfg = this.host.getConfig().mcp;
    const wantRelay = Boolean(cfg.enabled && this.host.account.isSignedIn());
    if (wantRelay && !this.relayRunning) {
      this.startRelay();
    } else if (!wantRelay && this.relayRunning) {
      this.relayGen++;
      this.relayRunning = false;
      this.relayAbort?.abort();
      this.relayAbort = null;
    }
  }

  info() {
    const signed = this.host.account.isSignedIn();
    const base = serverOrigin().replace(/\/+$/, "");
    const url = signed ? (this.serverToken?.url ?? `${base}/mcp`) : null;
    const token = signed ? (this.serverToken?.token ?? null) : null;
    const config = url && token ? clientConfig(url, token) : null;
    return {
      url,
      token,
      config,
      error: this.error,
      relay_running: this.relayRunning,
      token_loading: this.tokenLoading,
    };
  }

  async createToken(name?: string) {
    if (this.tokenLoading) return this.info();
    this.tokenLoading = true;
    try {
      const authToken = await this.host.account.validToken();
      const res = await server.mcpToken(authToken, name);
      this.serverToken = res;
      save("mcp_token", res);
      this.error = null;
    } catch (e) {
      this.error = String((e as Error)?.message ?? e);
    } finally {
      this.tokenLoading = false;
    }
    this.sync();
    return this.info();
  }

  async sessions(): Promise<McpSession[]> {
    if (!this.host.account.isSignedIn()) return [];
    const authToken = await this.host.account.validToken();
    return server.mcpSessions(authToken);
  }

  async renameSession(id: string, name: string): Promise<McpSession> {
    const authToken = await this.host.account.validToken();
    return server.mcpRenameSession(authToken, id, name);
  }

  async revokeSession(id: string): Promise<void> {
    const authToken = await this.host.account.validToken();
    await server.mcpRevokeSession(authToken, id);
  }

  async claimOAuth(id: string): Promise<McpApproval> {
    const authToken = await this.host.account.validToken();
    return server.mcpClaimOAuth(authToken, id);
  }

  async startConnectFlow(id: string): Promise<McpApproval> {
    const approval = await this.claimOAuth(id);
    if (!this.approvals.some((x) => x.id === approval.id)) {
      this.approvals.push(approval);
      this.host.emit("mcp_approvals", [...this.approvals]);
    }
    return approval;
  }

  private startRelay() {
    const gen = ++this.relayGen;
    this.relayRunning = true;
    void (async () => {
      let failures = 0;
      while (this.relayGen === gen) {
        const ctl = new AbortController();
        this.relayAbort = ctl;
        try {
          const authToken = await this.host.account.validToken();
          const pull = await server.mcpPull(authToken, ctl.signal);
          if (this.relayGen !== gen) break;
          failures = 0;
          this.error = null;

          let addedApproval = false;
          for (const a of pull.approvals ?? []) {
            if (!this.approvals.some((x) => x.id === a.id)) {
              this.approvals.push(a);
              addedApproval = true;
            }
          }
          if (addedApproval) {
            this.host.emit("mcp_approvals", [...this.approvals]);
          }

          for (const call of pull.calls ?? []) {
            void (async () => {
              const result = await this.dispatchCall(call.name, call.arguments ?? {});
              try {
                const tok = await this.host.account.validToken();
                await server.mcpResult(tok, { id: call.id, result });
              } catch { /* ignore */ }
            })();
          }
        } catch (e) {
          if (this.relayGen !== gen || (e as Error)?.name === "AbortError") break;
          failures++;
          this.error = String((e as Error)?.message ?? e);
          const waitS = Math.min(60, 1 << Math.min(6, failures));
          await new Promise((r) => setTimeout(r, waitS * 1000));
        }
      }
    })();
  }

  private async dispatchCall(name: string, args: Record<string, any>): Promise<unknown> {
    const cfg = this.host.getConfig();
    if (name === "tools/list") {
      const tools = ALL_TOOLS.filter((t) => allowsTool(cfg, t)).map((t) => ({ name: t }));
      return { tools };
    }
    if (!allowsTool(cfg, name)) {
      return textResult(
        `The user does not allow AI assistants to use \`${name}\` (Glyph: Settings › Integrations › AI assistants).`,
        true,
      );
    }
    try {
      const val = await this.runTool(name, args ?? {});
      return textResult(val, false);
    } catch (e) {
      return textResult(String((e as Error)?.message ?? e), true);
    }
  }

  private async resolveSongArg(args: Record<string, any>): Promise<Track> {
    if (typeof args.ref === "string" && args.ref.trim()) {
      const t = this.findTrackByRef(args.ref);
      if (!t) throw new Error(`Unknown song ref: ${args.ref}`);
      return t;
    }
    if (typeof args.query === "string" && args.query.trim()) {
      const hits = await this.host.searchTracks(args.query.trim());
      this.rememberSearch(hits);
      if (!hits.length) throw new Error(`No songs found for "${args.query}"`);
      return hits[0];
    }
    throw new Error("Provide either `ref` or `query`.");
  }

  private async pollOutputs(): Promise<PlaybackOutputWire[]> {
    if (!this.host.account.isSignedIn()) return [];
    const token = await this.host.account.validToken();
    const p = this.host.getPlayer();
    const res = await server.playbackPoll(token, {
      acknowledgements: false,
      track: p.current,
      playing: p.status === "playing",
      position_ms: Math.max(0, Math.round((p.position || 0) * 1000)),
      duration_ms: p.duration != null ? Math.round(p.duration * 1000) : null,
      volume: Math.max(0, Math.min(1, p.volume ?? 0.8)),
      available: false,
    });
    return res.devices ?? [];
  }

  private deviceEntry(o: PlaybackOutputWire) {
    return {
      id: o.device.id,
      name: o.device.name,
      platform: o.device.platform,
      current: o.device.current,
      online: o.online,
      available: o.online && o.state.available,
      selected: false,
      playing: o.online && o.state.playing,
      volume: Math.round((o.state.volume ?? 0) * 100),
      song: o.state.track ? this.rememberTrack(o.state.track) : null,
      last_seen: o.device.last_seen ?? null,
      location: o.device.approximate_location ?? null,
    };
  }

  private async resolveTargetDevice(query: string): Promise<{ kind: "local" } | { kind: "account"; output: PlaybackOutputWire }> {
    const q = query.trim();
    if (!q) throw new Error("device is empty");
    const ql = q.toLowerCase();
    if (ql === "local" || ql === "this" || ql === "this_computer" || ql === "here") {
      return { kind: "local" };
    }
    const outputs = await this.pollOutputs();
    const byId = outputs.find((o) => o.device.id === q);
    if (byId) return byId.device.current ? { kind: "local" } : { kind: "account", output: byId };
    let matches = outputs.filter((o) => norm(o.device.name) === ql);
    if (!matches.length) matches = outputs.filter((o) => o.device.name.toLowerCase().includes(ql));
    if (matches.length === 1) {
      const m = matches[0];
      return m.device.current ? { kind: "local" } : { kind: "account", output: m };
    }
    if (matches.length > 1) {
      throw new Error(`Multiple devices match "${q}": ${matches.map((m) => `${m.device.name} (${m.device.id})`).join(", ")}. Use the device id.`);
    }
    const names = outputs.map((o) => o.device.name).join(", ");
    throw new Error(names ? `No device "${q}" found. Available: ${names}.` : `No device "${q}" found (sign in to see account devices).`);
  }

  private settingsSnapshot(c: Config, section?: string) {
    const pick = section?.trim().toLowerCase() ?? "all";
    const all: Record<string, unknown> = {
      general: { language: c.language || "system" },
      appearance: {
        theme: c.visuals.theme,
        glow_shape: c.visuals.glow_shape || "auto",
        intensity: c.visuals.intensity,
        music_reactivity: c.visuals.music_reactivity,
        ascii_covers: c.visuals.ascii_covers,
        ascii_effects: c.visuals.ascii_effects,
        startup_animation: c.visuals.startup_animation,
        throttle_unfocused: c.visuals.throttle_unfocused,
      },
      audio: {
        quality: c.audio.quality,
        download_quality: c.audio.download_quality,
        crossfade_s: c.audio.crossfade_s,
        level: c.audio.level,
        normalize: c.audio.normalize,
        hide_explicit: c.audio.hide_explicit,
        eq_enabled: c.audio.eq_enabled,
        eq_preset: c.audio.eq_preset,
        eq_gains: eqGains(c.audio.eq_gains),
        eq_bass_db: c.audio.eq_bass_db,
        eq_headroom_db: c.audio.eq_headroom_db,
        eq_user_presets: (c.audio.eq_user_presets ?? []).map((p) => p.name),
      },
      playback: {
        shuffle_style: c.playback.shuffle,
        skip_unplayable: c.playback.skip_unplayable,
        patient_search: c.playback.patient_search,
      },
      player: {
        shuffle: c.player.shuffle,
        repeat: c.player.repeat,
        show_queue: c.player.show_queue,
        mini_large: c.player.mini_large,
      },
      stats: { enabled: c.stats.enabled },
      sharing: {
        glyph_links: c.sharing.glyph_links,
        profile_sections: c.sharing.profile_sections ?? ["playlists", "recent"],
      },
      integrations: {
        discord_presence: c.integrations.discord_presence,
        discord_client_id: c.integrations.discord_client_id,
      },
      providers: {
        enabled: c.providers.enabled,
        priority: c.providers.priority,
        youtube_daily_searches: (c.providers as Record<string, unknown>).youtube_daily_searches ?? 0,
      },
      updates: {
        check_on_start: c.updates.check_on_start,
        auto_install: c.updates.auto_install,
        channel: c.updates.channel,
      },
      lists: { sort: c.lists.sort },
    };
    const settings = pick === "all" || !pick ? all : pick in all ? { [pick]: all[pick] } : (() => { throw new Error(`Unknown settings section "${section}".`); })();
    return {
      settings,
      available: {
        languages: ["system", ...LANGS.map(([code]) => code)],
        themes: THEMES.map((t) => t.id),
        glow_shapes: [...GLOW_SHAPES],
        qualities: ["auto", "low", "standard", "high"],
        levels: ["loud", "normal", "quiet"],
        eq_presets: [...EQ_PRESETS.map(([k]) => k), ...(c.audio.eq_user_presets ?? []).map((p) => `user:${p.name}`)],
        shuffle_styles: ["smart", "random"],
        repeats: ["off", "all", "one"],
        profile_sections: [...PROFILE_SECTIONS],
        providers: PROVIDERS,
        channels: ["stable", "prerelease"],
        list_sorts: ["custom", "title", "artist", "album", "duration"],
      },
    };
  }

  private applySettingsPatch(c: Config, args: Record<string, any>): string[] {
    const updated: string[] = [];
    const obj = (v: unknown): Record<string, any> | null => (v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, any>) : null);
    const num = (v: unknown, name: string, min: number, max: number): number => {
      if (typeof v !== "number" || !Number.isFinite(v)) throw new Error(`${name} must be a number`);
      return Math.min(max, Math.max(min, v));
    };
    const bool = (v: unknown, name: string): boolean => {
      if (typeof v !== "boolean") throw new Error(`${name} must be a boolean`);
      return v;
    };

    const gen = obj(args.general);
    const langVal = gen?.language ?? (typeof args.language === "string" ? args.language : undefined);
    if (langVal !== undefined) {
      const code = String(langVal).trim().toLowerCase();
      if (code === "" || code === "system") c.language = "";
      else if (LANGS.some(([k]) => k === code)) c.language = code;
      else throw new Error(`Unsupported language "${langVal}".`);
      updated.push("general.language");
    }

    const app = obj(args.appearance) ?? obj(args.visuals);
    const themeVal = app?.theme ?? (typeof args.theme === "string" ? args.theme : undefined);
    if (themeVal !== undefined) {
      const id = String(themeVal).trim();
      if (!THEMES.some((t) => t.id === id) && !id.startsWith("plugin:")) throw new Error(`Unknown theme "${themeVal}".`);
      c.visuals.theme = id;
      updated.push("appearance.theme");
    }
    const glowVal = app?.glow_shape ?? (typeof args.glow_shape === "string" ? args.glow_shape : undefined);
    if (glowVal !== undefined) {
      const g = String(glowVal).trim().toLowerCase();
      if (g === "" || g === "auto") c.visuals.glow_shape = "";
      else if ((GLOW_SHAPES as readonly string[]).includes(g)) c.visuals.glow_shape = g;
      else throw new Error(`Unknown glow_shape "${glowVal}".`);
      updated.push("appearance.glow_shape");
    }
    if (app) {
      if (app.intensity !== undefined) { c.visuals.intensity = num(app.intensity, "appearance.intensity", 0, 1); updated.push("appearance.intensity"); }
      if (app.music_reactivity !== undefined) { c.visuals.music_reactivity = num(app.music_reactivity, "appearance.music_reactivity", 0, 1); updated.push("appearance.music_reactivity"); }
      if (app.ascii_covers !== undefined) { c.visuals.ascii_covers = bool(app.ascii_covers, "appearance.ascii_covers"); updated.push("appearance.ascii_covers"); }
      if (app.ascii_effects !== undefined) { c.visuals.ascii_effects = bool(app.ascii_effects, "appearance.ascii_effects"); updated.push("appearance.ascii_effects"); }
      if (app.startup_animation !== undefined) { c.visuals.startup_animation = bool(app.startup_animation, "appearance.startup_animation"); updated.push("appearance.startup_animation"); }
      if (app.throttle_unfocused !== undefined) { c.visuals.throttle_unfocused = bool(app.throttle_unfocused, "appearance.throttle_unfocused"); updated.push("appearance.throttle_unfocused"); }
    }

    const aud = obj(args.audio);
    if (aud) {
      if (aud.quality !== undefined) {
        const q = String(aud.quality).trim().toLowerCase();
        if (!QUALITIES.has(q)) throw new Error(`Invalid audio.quality "${aud.quality}".`);
        c.audio.quality = q; updated.push("audio.quality");
      }
      if (aud.download_quality !== undefined) {
        const q = String(aud.download_quality).trim().toLowerCase();
        if (!QUALITIES.has(q)) throw new Error(`Invalid audio.download_quality "${aud.download_quality}".`);
        c.audio.download_quality = q; updated.push("audio.download_quality");
      }
      if (aud.crossfade_s !== undefined) { c.audio.crossfade_s = Math.round(num(aud.crossfade_s, "audio.crossfade_s", 0, 20)); updated.push("audio.crossfade_s"); }
      if (aud.level !== undefined) {
        const l = String(aud.level).trim().toLowerCase();
        if (!LEVELS.has(l)) throw new Error(`Invalid audio.level "${aud.level}".`);
        c.audio.level = l; updated.push("audio.level");
      }
      if (aud.normalize !== undefined) { c.audio.normalize = bool(aud.normalize, "audio.normalize"); updated.push("audio.normalize"); }
      if (aud.hide_explicit !== undefined) { c.audio.hide_explicit = bool(aud.hide_explicit, "audio.hide_explicit"); updated.push("audio.hide_explicit"); }
      if (aud.eq_gains !== undefined) {
        if (!Array.isArray(aud.eq_gains) || aud.eq_gains.length !== 10) throw new Error("audio.eq_gains must be an array of 10 numbers");
        c.audio.eq_gains = aud.eq_gains.map((v, i) => Math.round(num(v, `audio.eq_gains[${i}]`, -12, 12) * 10) / 10);
        if (aud.eq_enabled === undefined) c.audio.eq_enabled = true;
        if (aud.eq_preset === undefined) c.audio.eq_preset = "custom";
        updated.push("audio.eq_gains");
      }
      if (aud.eq_preset !== undefined) {
        const key = String(aud.eq_preset).trim();
        const builtin = EQ_PRESETS.find(([k]) => k === key);
        const userName = key.startsWith("user:") ? key.slice(5) : key;
        const userPreset = (c.audio.eq_user_presets ?? []).find((p) => p.name.toLowerCase() === userName.toLowerCase());
        if (builtin) {
          c.audio.eq_preset = builtin[0];
          if (aud.eq_gains === undefined) c.audio.eq_gains = [...builtin[1]];
          if (aud.eq_enabled === undefined) c.audio.eq_enabled = true;
        } else if (userPreset) {
          c.audio.eq_preset = `user:${userPreset.name}`;
          if (aud.eq_gains === undefined) c.audio.eq_gains = eqGains(userPreset.gains);
          if (aud.eq_enabled === undefined) c.audio.eq_enabled = true;
        } else if (key === "custom") {
          c.audio.eq_preset = "custom";
        } else {
          throw new Error(`Unknown EQ preset "${aud.eq_preset}".`);
        }
        updated.push("audio.eq_preset");
      }
      if (aud.eq_enabled !== undefined) { c.audio.eq_enabled = bool(aud.eq_enabled, "audio.eq_enabled"); updated.push("audio.eq_enabled"); }
      if (aud.eq_bass_db !== undefined) { c.audio.eq_bass_db = Math.round(num(aud.eq_bass_db, "audio.eq_bass_db", -12, 12) * 2) / 2; updated.push("audio.eq_bass_db"); }
      if (aud.eq_headroom_db !== undefined) { c.audio.eq_headroom_db = Math.round(num(aud.eq_headroom_db, "audio.eq_headroom_db", 0, 12) * 2) / 2; updated.push("audio.eq_headroom_db"); }
    }

    const pb = obj(args.playback);
    if (pb) {
      const st = pb.shuffle_style ?? pb.shuffle;
      if (st !== undefined) {
        const s = String(st).trim().toLowerCase();
        if (!SHUFFLE_STYLES.has(s)) throw new Error(`Invalid playback.shuffle_style "${st}".`);
        c.playback.shuffle = s as "smart" | "random"; updated.push("playback.shuffle_style");
      }
      if (pb.skip_unplayable !== undefined) { c.playback.skip_unplayable = bool(pb.skip_unplayable, "playback.skip_unplayable"); updated.push("playback.skip_unplayable"); }
      if (pb.patient_search !== undefined) { c.playback.patient_search = bool(pb.patient_search, "playback.patient_search"); updated.push("playback.patient_search"); }
    }

    const pl = obj(args.player);
    if (pl) {
      if (pl.shuffle !== undefined) { c.player.shuffle = bool(pl.shuffle, "player.shuffle"); updated.push("player.shuffle"); }
      if (pl.repeat !== undefined) {
        const r = String(pl.repeat).trim().toLowerCase();
        if (!REPEATS.has(r)) throw new Error(`Invalid player.repeat "${pl.repeat}".`);
        c.player.repeat = r; updated.push("player.repeat");
      }
      if (pl.show_queue !== undefined) { c.player.show_queue = bool(pl.show_queue, "player.show_queue"); updated.push("player.show_queue"); }
      if (pl.mini_large !== undefined) { c.player.mini_large = bool(pl.mini_large, "player.mini_large"); updated.push("player.mini_large"); }
    }

    const st = obj(args.stats);
    if (st && st.enabled !== undefined) { c.stats.enabled = bool(st.enabled, "stats.enabled"); updated.push("stats.enabled"); }

    const sh = obj(args.sharing);
    if (sh) {
      if (sh.glyph_links !== undefined) { c.sharing.glyph_links = bool(sh.glyph_links, "sharing.glyph_links"); updated.push("sharing.glyph_links"); }
      if (sh.profile_sections !== undefined) {
        if (!Array.isArray(sh.profile_sections)) throw new Error("sharing.profile_sections must be an array");
        const next: ProfileSection[] = [];
        for (const s of sh.profile_sections) {
          const k = String(s).trim().toLowerCase() as ProfileSection;
          if (!PROFILE_SECTIONS.includes(k)) throw new Error(`Unknown profile_section "${s}".`);
          if (!next.includes(k)) next.push(k);
        }
        for (const def of PROFILE_SECTIONS) if (!next.includes(def)) next.push(def);
        c.sharing.profile_sections = next;
        updated.push("sharing.profile_sections");
      }
    }

    const intg = obj(args.integrations);
    if (intg) {
      if (intg.discord_presence !== undefined) { c.integrations.discord_presence = bool(intg.discord_presence, "integrations.discord_presence"); updated.push("integrations.discord_presence"); }
      if (intg.discord_client_id !== undefined) { c.integrations.discord_client_id = String(intg.discord_client_id).trim(); updated.push("integrations.discord_client_id"); }
    }

    const prov = obj(args.providers);
    if (prov) {
      const parseProviders = (arr: unknown, field: string): ProviderId[] => {
        if (!Array.isArray(arr)) throw new Error(`${field} must be an array of provider IDs`);
        const out: ProviderId[] = [];
        for (const item of arr) {
          const id = String(item).trim().toLowerCase() as ProviderId;
          if (!PROVIDERS.includes(id)) throw new Error(`Unknown provider "${item}".`);
          if (!out.includes(id)) out.push(id);
        }
        return out;
      };
      if (prov.enabled !== undefined) { c.providers.enabled = parseProviders(prov.enabled, "providers.enabled"); updated.push("providers.enabled"); }
      if (prov.priority !== undefined) {
        const p = parseProviders(prov.priority, "providers.priority");
        for (const id of c.providers.priority) if (!p.includes(id)) p.push(id);
        c.providers.priority = p; updated.push("providers.priority");
      }
      if (prov.youtube_daily_searches !== undefined) {
        (c.providers as Record<string, unknown>).youtube_daily_searches = Math.round(num(prov.youtube_daily_searches, "providers.youtube_daily_searches", 0, 100_000));
        updated.push("providers.youtube_daily_searches");
      }
    }

    const upd = obj(args.updates);
    if (upd) {
      if (upd.check_on_start !== undefined) { c.updates.check_on_start = bool(upd.check_on_start, "updates.check_on_start"); updated.push("updates.check_on_start"); }
      if (upd.auto_install !== undefined) { c.updates.auto_install = bool(upd.auto_install, "updates.auto_install"); updated.push("updates.auto_install"); }
      if (upd.channel !== undefined) {
        const ch = String(upd.channel).trim().toLowerCase();
        if (!CHANNELS.has(ch)) throw new Error(`Invalid updates.channel "${upd.channel}".`);
        c.updates.channel = ch; updated.push("updates.channel");
      }
    }

    const ls = obj(args.lists);
    if (ls && ls.sort !== undefined) {
      const map = obj(ls.sort);
      if (!map) throw new Error("lists.sort must be an object mapping list id to sort mode");
      for (const [k, v] of Object.entries(map)) {
        const m = String(v).trim().toLowerCase();
        if (!SORT_MODES.has(m)) throw new Error(`Invalid list sort mode "${v}" for "${k}".`);
        c.lists.sort[k] = m;
      }
      updated.push("lists.sort");
    }

    if (!updated.length) throw new Error("Provide at least one setting section or field to update.");
    return updated;
  }

  private async resolvePlayTracks(args: Record<string, any>): Promise<{ tracks: Track[]; context: string | null }> {
    const lib = this.host.getLibrary();
    if (typeof args.playlist_id === "string" && args.playlist_id.trim()) {
      const id = args.playlist_id.trim();
      const pl =
        id === "liked"
          ? { name: "Liked Songs", tracks: lib.liked }
          : id === "recent"
            ? { name: "Recently played", tracks: lib.recent.flatMap((r) => (r.kind === "song" ? [r.track] : [])) }
            : lib.playlists.find((x) => x.id === id || norm(x.name) === norm(id));
      if (!pl) throw new Error(`Playlist not found: ${id}`);
      if (!pl.tracks.length) throw new Error(`Playlist "${pl.name}" is empty`);
      return { tracks: [...pl.tracks], context: pl.name };
    }
    if (typeof args.artist === "string" && args.artist.trim()) {
      const artistName = args.artist.trim();
      const page = await this.host.callHandler("artist", { name: artistName });
      const tracks: Track[] = page?.tracks ?? [];
      if (!tracks.length) throw new Error(`No songs found for artist "${artistName}"`);
      this.rememberSearch(tracks);
      return { tracks, context: artistName };
    }
    const t = await this.resolveSongArg(args);
    return { tracks: [t], context: t.title };
  }

  private shuffleCopy(list: Track[]): Track[] {
    const out = [...list];
    for (let i = out.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [out[i], out[j]] = [out[j], out[i]];
    }
    return out;
  }

  private async sendRemoteTransfer(
    output: PlaybackOutputWire,
    tracks: Track[],
    opts: { index?: number; position_ms?: number; playing?: boolean } = {},
  ) {
    const token = await this.host.account.validToken();
    const confirm = Boolean(output.state.acknowledgements);
    const res = await server.playbackCommand(
      token,
      {
        target: output.device.id,
        command: {
          action: "transfer",
          tracks: tracks.slice(0, 100).map(wireTrack),
          index: opts.index ?? 0,
          position_ms: opts.position_ms ?? 0,
          playing: opts.playing ?? true,
        },
      },
      confirm,
    );
    if (confirm && res?.id) {
      const deadline = Date.now() + 18_000;
      while (Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, 350));
        const rc = await server.playbackReceipt(token, res.id).catch(() => null);
        if (rc?.done) {
          if (rc.error) throw new Error(rc.error);
          return;
        }
      }
      throw new Error(`${output.device.name} did not confirm playback in time.`);
    }
  }

  private clearSleepState() {
    if (this.sleepTimerId) {
      clearTimeout(this.sleepTimerId);
      this.sleepTimerId = null;
    }
    if (this.sleepPollId) {
      clearInterval(this.sleepPollId);
      this.sleepPollId = null;
    }
    this.sleepUntil = null;
    this.sleepEndOfSong = false;
  }

  private sleepStatus() {
    if (this.sleepEndOfSong) return { active: true, remaining_s: null, end_of_song: true };
    if (this.sleepUntil != null) {
      const rem = Math.max(0, Math.ceil((this.sleepUntil - Date.now()) / 1000));
      return { active: rem > 0, remaining_s: rem, end_of_song: false };
    }
    return { active: false, remaining_s: null, end_of_song: false };
  }

  private setSleepTimer(mins: number | null, endOfSong: boolean) {
    this.clearSleepState();
    if (endOfSong) {
      this.sleepEndOfSong = true;
      const startKey = this.host.getPlayer().current ? trackKey(this.host.getPlayer().current!) : null;
      this.sleepPollId = setInterval(() => {
        const cur = this.host.getPlayer();
        const curKey = cur.current ? trackKey(cur.current) : null;
        if (curKey !== startKey || cur.status === "idle") {
          this.clearSleepState();
          if (cur.status === "playing") void this.host.callHandler("toggle_play", {});
        }
      }, 400);
      return;
    }
    if (mins != null && mins > 0) {
      const ms = mins * 60_000;
      this.sleepUntil = Date.now() + ms;
      this.sleepTimerId = setTimeout(() => {
        this.clearSleepState();
        if (this.host.getPlayer().status === "playing") void this.host.callHandler("toggle_play", {});
      }, ms);
    }
  }

  private async forwardDocsTool(name: string, args: Record<string, any>): Promise<unknown> {
    const r = await fetch(DOCS_MCP_URL, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "tools/call",
        params: { name, arguments: args },
      }),
    });
    if (!r.ok) throw new Error(`Docs server returned HTTP ${r.status}`);
    const body = await r.json();
    if (body?.error?.message) throw new Error(String(body.error.message));
    const text = body?.result?.content?.[0]?.text;
    if (typeof text === "string") {
      try { return JSON.parse(text); } catch { return text; }
    }
    return body?.result ?? {};
  }

  private async runTool(name: string, args: Record<string, any>): Promise<unknown> {
    const p = this.host.getPlayer();
    const q = this.host.getQueue();
    const lib = this.host.getLibrary();

    switch (name) {
      case "now_playing": {
        return {
          status: p.status,
          song: p.current ? this.rememberTrack(p.current) : null,
          position_s: Math.round(p.position),
          duration_s: p.duration != null ? Math.round(p.duration) : null,
          volume: Math.round(p.volume * 100),
          muted: p.muted,
          shuffle: p.shuffle,
          repeat: p.repeat,
          queue_length: q.items.length,
        };
      }
      case "play": {
        if (p.status !== "playing") await this.host.callHandler("toggle_play", {});
        return { status: this.host.getPlayer().status };
      }
      case "pause": {
        if (p.status === "playing" || p.status === "loading") await this.host.callHandler("toggle_play", {});
        return { status: this.host.getPlayer().status };
      }
      case "toggle_play": {
        await this.host.callHandler("toggle_play", {});
        return { status: this.host.getPlayer().status };
      }
      case "next": {
        await this.host.callHandler("next", {});
        const cur = this.host.getPlayer().current;
        return { song: cur ? this.rememberTrack(cur) : null };
      }
      case "previous": {
        await this.host.callHandler("previous", {});
        const cur = this.host.getPlayer().current;
        return { song: cur ? this.rememberTrack(cur) : null };
      }
      case "seek": {
        const sec = Math.max(0, Number(args.seconds) || 0);
        await this.host.callHandler("seek", { position: sec });
        return { position_s: sec };
      }
      case "set_volume": {
        const v = Math.max(0, Math.min(100, Number(args.volume) || 0));
        await this.host.callHandler("set_volume", { volume: v / 100 });
        return { volume: v };
      }
      case "set_shuffle": {
        const on = Boolean(args.on);
        await this.host.callHandler("set_shuffle", { on });
        return { shuffle: on };
      }
      case "set_repeat": {
        const mode = args.mode === "all" || args.mode === "one" ? args.mode : "off";
        await this.host.callHandler("set_repeat", { mode });
        return { repeat: mode };
      }
      case "search": {
        const query = String(args.query ?? "").trim();
        const limit = Math.max(1, Math.min(50, Number(args.limit) || 15));
        const tracks = await this.host.searchTracks(query);
        this.rememberSearch(tracks);
        return {
          query,
          songs: tracks.slice(0, limit).map((t) => this.rememberTrack(t)),
        };
      }
      case "play_song": {
        const t = await this.resolveSongArg(args);
        await this.host.callHandler("play", { tracks: [t], index: 0, context: t.title });
        return { playing: this.rememberTrack(t) };
      }
      case "play_artist": {
        const artistName = String(args.name ?? "").trim();
        const page = await this.host.callHandler("artist", { name: artistName });
        const tracks: Track[] = page?.tracks ?? [];
        if (!tracks.length) throw new Error(`No songs found for artist "${artistName}"`);
        this.rememberSearch(tracks);
        if (args.shuffle) {
          await this.host.callHandler("play_shuffled", { tracks, context: artistName });
        } else {
          await this.host.callHandler("play", { tracks, index: 0, context: artistName });
        }
        return { artist: artistName, count: tracks.length, playing: this.rememberTrack(this.host.getPlayer().current ?? tracks[0]) };
      }
      case "queue_add": {
        const t = await this.resolveSongArg(args);
        const next = Boolean(args.next);
        await this.host.callHandler("queue_add", { tracks: [t], next });
        return { queued: this.rememberTrack(t), next };
      }
      case "get_queue": {
        const idx = q.index ?? 0;
        const upcoming = q.items.slice(idx + 1, idx + 31).map((t) => this.rememberTrack(t));
        return {
          current: p.current ? this.rememberTrack(p.current) : null,
          up_next: upcoming,
          total: q.items.length,
        };
      }
      case "queue_remove": {
        const i = Number(args.index);
        const base = q.index != null ? q.index + 1 : 0;
        const len = Math.max(0, q.items.length - base);
        if (!Number.isInteger(i) || i < 0 || i >= len) throw new Error(`Queue index ${i} out of range (${len} upcoming songs)`);
        await this.host.callHandler("queue_remove", { index: base + i });
        return { removed_index: i };
      }
      case "queue_move": {
        const from = Number(args.from);
        const to = Number(args.to);
        const base = q.index != null ? q.index + 1 : 0;
        const len = Math.max(0, q.items.length - base);
        if (!Number.isInteger(from) || !Number.isInteger(to) || from < 0 || to < 0 || from >= len || to >= len) {
          throw new Error(`Queue index out of range (${len} upcoming songs)`);
        }
        await this.host.callHandler("queue_move", { from: base + from, to: base + to });
        return { from, to };
      }
      case "queue_jump": {
        const i = Number(args.index);
        const base = q.index != null ? q.index + 1 : 0;
        const len = Math.max(0, q.items.length - base);
        if (!Number.isInteger(i) || i < 0 || i >= len) throw new Error(`Queue index ${i} out of range (${len} upcoming songs)`);
        const targetTrack = q.items[base + i];
        await this.host.callHandler("queue_jump", { index: base + i });
        return { playing: this.rememberTrack(targetTrack), index: i };
      }
      case "queue_clear": {
        const base = q.index != null ? q.index + 1 : 0;
        const cleared = Math.max(0, q.items.length - base);
        await this.host.callHandler("queue_clear", {});
        return { cleared };
      }
      case "start_radio": {
        if (typeof args.playlist_id === "string" && args.playlist_id.trim()) {
          const id = args.playlist_id.trim();
          if (id === "recent") {
            const songs = lib.recent.flatMap((r) => (r.kind === "song" ? [r.track] : []));
            if (!songs.length) throw new Error("No recently played songs to base radio on");
            await this.host.callHandler("song_radio", { track: songs[0], title: "Recently played" });
            return { started: true, from: "Recently played" };
          }
          if (id === "liked") {
            if (!lib.liked.length) throw new Error("Liked Songs is empty");
            await this.host.callHandler("list_radio", { list: { kind: "liked" } });
            return { started: true, from: "Liked Songs" };
          }
          const pl = lib.playlists.find((x) => x.id === id || norm(x.name) === norm(id));
          if (!pl) throw new Error(`Playlist not found: ${id}`);
          if (!pl.tracks.length) throw new Error(`Playlist "${pl.name}" is empty`);
          await this.host.callHandler("list_radio", { list: { kind: "playlist", id: pl.id } });
          return { started: true, from: pl.name };
        }
        if (typeof args.artist === "string" && args.artist.trim()) {
          const artist = args.artist.trim();
          const pg = await this.host.callHandler("artist", { name: artist });
          const tracks: Track[] = pg?.tracks ?? [];
          if (!tracks.length) throw new Error(`No songs by "${artist}" found to start radio`);
          await this.host.callHandler("artist_radio", { artist });
          return { started: true, from: artist };
        }
        if (args.ref || args.query) {
          const t = await this.resolveSongArg(args);
          await this.host.callHandler("song_radio", { track: t, title: t.title });
          return { started: true, seed: this.rememberTrack(t) };
        }
        if (p.current) {
          await this.host.callHandler("song_radio", { track: p.current, title: p.current.title });
          return { started: true, seed: this.rememberTrack(p.current) };
        }
        throw new Error("Nothing is playing and no ref, query, artist, or playlist_id was given");
      }
      case "sleep_timer": {
        const off = args.off === true || Number(args.minutes) === 0;
        if (off) {
          this.setSleepTimer(null, false);
        } else if (args.end_of_song === true) {
          this.setSleepTimer(null, true);
        } else if (args.minutes !== undefined && args.minutes !== null) {
          const mins = Math.max(1, Math.min(480, Math.round(Number(args.minutes) || 0)));
          this.setSleepTimer(mins, false);
        }
        return this.sleepStatus();
      }
      case "recent_songs": {
        const limit = Math.max(1, Math.min(50, Number(args.limit) || 20));
        const songs = lib.recent.flatMap((r) => (r.kind === "song" ? [r.track] : [])).slice(0, limit);
        return { songs: songs.map((t) => this.rememberTrack(t)) };
      }
      case "list_devices": {
        const signedIn = this.host.account.isSignedIn();
        const outputs = signedIn ? await this.pollOutputs() : [];
        const current = outputs.find((o) => o.device.current);
        return {
          signed_in: signedIn,
          active_output: {
            id: current?.device.id ?? "this",
            name: current?.device.name ?? "Web player",
            kind: "local",
            active: true,
          },
          account_devices: outputs.map((o) => this.deviceEntry(o)),
          network_devices: [],
        };
      }
      case "play_on_device": {
        const target = await this.resolveTargetDevice(String(args.device ?? ""));
        if (target.kind === "account") {
          if (!target.output.online) throw new Error(`${target.output.device.name} is currently offline`);
          if (!target.output.state.available) throw new Error(`${target.output.device.name} is currently busy or in a Jam`);
        }
        let { tracks, context } = await this.resolvePlayTracks(args);
        const shuffle = Boolean(args.shuffle);
        if (target.kind === "local") {
          if (shuffle) {
            await this.host.callHandler("play_shuffled", { tracks, context });
          } else {
            await this.host.callHandler("play", { tracks, index: 0, context });
          }
          return {
            device: { id: "this", name: "Web player", kind: "local" },
            playing: this.rememberTrack(this.host.getPlayer().current ?? tracks[0]),
            count: tracks.length,
          };
        }
        if (shuffle && tracks.length > 1) tracks = this.shuffleCopy(tracks);
        await this.sendRemoteTransfer(target.output, tracks, { index: 0, position_ms: 0, playing: true });
        return {
          device: {
            id: target.output.device.id,
            name: target.output.device.name,
            platform: target.output.device.platform,
            kind: "account",
          },
          playing: this.rememberTrack(tracks[0]),
          count: Math.min(tracks.length, 100),
          controlling: false,
        };
      }
      case "select_device": {
        const target = await this.resolveTargetDevice(String(args.device ?? ""));
        if (target.kind === "local") {
          return { device: { id: "this", name: "Web player", kind: "local" } };
        }
        if (!target.output.online) throw new Error(`${target.output.device.name} is currently offline`);
        if (!target.output.state.available) throw new Error(`${target.output.device.name} is currently busy or in a Jam`);
        const cur = p.current;
        if (!cur) {
          throw new Error("Web player cannot remotely control another device when nothing is playing here; use play_on_device instead.");
        }
        const idx = q.index ?? 0;
        const tracks = [cur, ...q.items.slice(idx + 1, idx + 100)];
        const wasPlaying = p.status === "playing";
        await this.sendRemoteTransfer(target.output, tracks, {
          index: 0,
          position_ms: Math.max(0, Math.round((p.position || 0) * 1000)),
          playing: wasPlaying,
        });
        if (wasPlaying) await this.host.callHandler("toggle_play", {});
        return {
          device: {
            id: target.output.device.id,
            name: target.output.device.name,
            platform: target.output.device.platform,
            kind: "account",
          },
          transferred: true,
        };
      }
      case "list_followed": {
        return { artists: [...lib.followed] };
      }
      case "follow_artist": {
        const artistName = String(args.name ?? "").trim();
        if (!artistName) throw new Error("Artist name is required");
        const on = args.follow === undefined ? true : Boolean(args.follow);
        await this.host.callHandler("follow", { artist: artistName, on });
        return { artist: artistName, following: on };
      }
      case "list_playlists": {
        return {
          playlists: [
            { id: "liked", name: "Liked Songs", songs: lib.liked.length },
            ...lib.playlists.map((pl) => ({ id: pl.id, name: pl.name, songs: pl.tracks.length })),
          ],
        };
      }
      case "get_playlist": {
        const id = String(args.id ?? "");
        if (id === "liked") {
          return { id: "liked", name: "Liked Songs", songs: lib.liked.map((t) => this.rememberTrack(t)) };
        }
        const pl = lib.playlists.find((x) => x.id === id || norm(x.name) === norm(id));
        if (!pl) throw new Error(`Playlist not found: ${id}`);
        return { id: pl.id, name: pl.name, songs: pl.tracks.map((t) => this.rememberTrack(t)) };
      }
      case "play_playlist": {
        const id = String(args.id ?? "");
        const pl = id === "liked" ? { name: "Liked Songs", tracks: lib.liked } : lib.playlists.find((x) => x.id === id || norm(x.name) === norm(id));
        if (!pl) throw new Error(`Playlist not found: ${id}`);
        if (!pl.tracks.length) throw new Error(`Playlist "${pl.name}" is empty`);
        if (args.shuffle) {
          await this.host.callHandler("play_shuffled", { tracks: pl.tracks, context: pl.name });
        } else {
          await this.host.callHandler("play", { tracks: pl.tracks, index: 0, context: pl.name });
        }
        return { playlist: pl.name, songs: pl.tracks.length };
      }
      case "create_playlist": {
        const name = String(args.name ?? "").trim();
        if (!name) throw new Error("Playlist name is required");
        const refs: string[] = Array.isArray(args.refs) ? args.refs : [];
        const tracks = refs.map((r) => this.findTrackByRef(String(r))).filter((t): t is Track => t !== null);
        const id = await this.host.callHandler("playlist_create", { name, tracks });
        return { id, name, songs: tracks.length };
      }
      case "rename_playlist": {
        const id = String(args.id ?? "");
        const name = String(args.name ?? "").trim();
        await this.host.callHandler("playlist_rename", { id, name });
        return { id, name };
      }
      case "delete_playlist": {
        const id = String(args.id ?? "");
        await this.host.callHandler("playlist_delete", { id });
        return { deleted: id };
      }
      case "add_to_playlist": {
        const id = String(args.id ?? "");
        const refs: string[] = Array.isArray(args.refs) ? args.refs : [];
        const tracks = refs.map((r) => this.findTrackByRef(String(r))).filter((t): t is Track => t !== null);
        if (!tracks.length) throw new Error("No valid song refs found");
        await this.host.callHandler("playlist_add", { id, tracks });
        return { id, added: tracks.length };
      }
      case "remove_from_playlist": {
        const id = String(args.id ?? "");
        const index = Number(args.index);
        await this.host.callHandler("playlist_remove", { id, index });
        return { id, removed_index: index };
      }
      case "like":
      case "unlike": {
        const on = name === "like";
        const t = args.ref ? this.findTrackByRef(String(args.ref)) : p.current;
        if (!t) throw new Error("No song is playing and no valid `ref` was given");
        await this.host.callHandler("like", { track: t, on });
        return { song: this.rememberTrack(t), liked: on };
      }
      case "get_lyrics": {
        if (!p.current) throw new Error("Nothing is playing right now");
        const l = await this.host.getLyrics(p.current);
        if (!l) return { song: `${primaryArtist(p.current.artist)} - ${p.current.title}`, lyrics: null };
        return {
          song: `${primaryArtist(p.current.artist)} - ${p.current.title}`,
          source: l.source,
          lyrics: l.lines.map((x) => x.text).join("\n"),
        };
      }
      case "listening_stats": {
        if (!this.host.getConfig().stats.enabled) {
          throw new Error("Listening statistics are turned off in Settings › Playback › Content.");
        }
        const r: StatsRange =
          args.range === "last_month" || args.range === "year" || args.range === "all" ? args.range : "month";
        const s = this.host.stats.summary(r);
        return {
          range: s.range,
          minutes: Math.round(s.ms / 60000),
          plays: s.plays,
          skips: s.skips,
          artists: s.artists,
          songs: s.songs,
          active_days: s.active_days,
          top_artists: s.top_artists.slice(0, 10).map((a) => ({ name: a.name, minutes: Math.round(a.ms / 60000), plays: a.plays })),
          top_songs: s.top_tracks.slice(0, 10).map((x) => ({ ...this.rememberTrack(x.track), minutes: Math.round(x.ms / 60000), plays: x.plays })),
        };
      }
      case "get_settings": {
        return this.settingsSnapshot(this.host.getConfig(), typeof args.section === "string" ? args.section : undefined);
      }
      case "set_settings": {
        const next: Config = JSON.parse(JSON.stringify(this.host.getConfig()));
        const updated = this.applySettingsPatch(next, args);
        await this.host.callHandler("set_config", { config: next });
        if (this.host.account.isSignedIn()) this.host.account.scheduleSync();
        return { updated, settings: this.settingsSnapshot(next).settings };
      }
      case "docs_search":
      case "docs_read":
        return this.forwardDocsTool(name, args);
      default:
        throw new Error(`Unknown tool: ${name}`);
    }
  }

  handlers(): Record<string, (a: Record<string, any>) => unknown> {
    return {
      mcp_info: () => {
        this.sync();
        return this.info();
      },

      mcp_rotate_token: async () => {
        if (!this.host.account.isSignedIn()) return this.info();
        return this.createToken();
      },

      mcp_server_token: async (a) => {
        const name = typeof a?.name === "string" ? a.name : undefined;
        return this.createToken(name);
      },

      mcp_sessions: async () => {
        return this.sessions();
      },

      mcp_rename_session: async (a) => {
        return this.renameSession(String(a.id ?? ""), String(a.name ?? ""));
      },

      mcp_revoke_session: async (a) => {
        return this.revokeSession(String(a.id ?? ""));
      },

      mcp_claim_oauth: async (a) => {
        return this.claimOAuth(String(a.id ?? ""));
      },

      mcp_pending: () => [...this.approvals],

      mcp_approve: async (a) => {
        const id = String(a.id ?? "");
        const allow = Boolean(a.allow);
        this.approvals = this.approvals.filter((x) => x.id !== id);
        this.host.emit("mcp_approvals", [...this.approvals]);
        const authToken = await this.host.account.validToken();
        await server.mcpApprove(authToken, { id, allow });
        if (allow) {
          const cfg = this.host.getConfig();
          if (!cfg.mcp.enabled) {
            const next = JSON.parse(JSON.stringify(cfg));
            next.mcp.enabled = true;
            await this.host.callHandler("set_config", { config: next });
          }
        }
        this.host.toast(allow ? "mcp.allowed" : "mcp.denied", allow ? "ok" : "info");
      },
    };
  }
}
