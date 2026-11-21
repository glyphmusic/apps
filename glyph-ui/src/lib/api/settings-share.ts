// Shareable settings for the web player, wire-compatible with
// `glyph-core/src/settings_share.rs`: `glyph:settings:` +
// base64url(raw deflate(JSON)), or the same JSON saved as `glyph-settings.json`.
// Never exports API keys, server addresses, or device-bound values.
import type { Config, EqPreset, ProfileSection, ProviderId } from "./types";

export const SETTINGS_PREFIX = "glyph:settings:";
export const SETTINGS_FILE_NAME = "glyph-settings.json";
const MAX_JSON = 256 * 1024;
const VALID_PROVIDERS = new Set<ProviderId>(["audius", "jamendo", "archive", "soundcloud", "youtube", "deezer"]);

interface SharedSettingsPayload {
  glyph_settings: number;
  app_version?: string;
  sections: Record<string, unknown>;
}

function b64url(bytes: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function unb64url(s: string): Uint8Array {
  const std = s.replace(/-/g, "+").replace(/_/g, "/").replace(/=+$/, "");
  const bin = atob(std + "=".repeat((4 - (std.length % 4)) % 4));
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

async function pipe(
  bytes: Uint8Array,
  stream: CompressionStream | DecompressionStream,
  limit = Infinity,
  lenient = false,
): Promise<Uint8Array> {
  const writer = stream.writable.getWriter();
  const reader = stream.readable.getReader();
  const fed = writer.write(bytes as BufferSource).then(() => writer.close()).catch(() => {});
  const parts: Uint8Array[] = [];
  let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      if (size > limit) {
        void reader.cancel().catch(() => {});
        throw new RangeError("settings too large");
      }
      parts.push(value);
    }
  } catch (e) {
    if (!lenient || !size || e instanceof RangeError) throw e;
  }
  await fed;
  const out = new Uint8Array(size);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

export function isSettingsCode(s: string): boolean {
  return s.trim().startsWith(SETTINGS_PREFIX);
}

function buildPayload(cfg: Config, appVersion = "0.0.0"): SharedSettingsPayload {
  const a = cfg.audio;
  const sections: Record<string, unknown> = {
    general: { language: cfg.language },
    providers: {
      enabled: cfg.providers.enabled,
      priority: cfg.providers.priority,
    },
    visuals: { ...cfg.visuals },
    updates: { ...cfg.updates },
    integrations: { ...cfg.integrations },
    sharing: { ...cfg.sharing },
    stats: { ...cfg.stats },
    playback: { ...cfg.playback },
    lists: { ...cfg.lists },
    audio: {
      crossfade_s: a.crossfade_s,
      quality: a.quality,
      download_quality: a.download_quality,
      level: a.level,
      normalize: a.normalize,
      hide_explicit: a.hide_explicit,
      eq_enabled: a.eq_enabled,
      eq_preset: a.eq_preset,
      eq_gains: a.eq_gains,
      eq_bass_db: a.eq_bass_db,
      eq_headroom_db: a.eq_headroom_db,
    },
    player: {
      shuffle: cfg.player.shuffle,
      repeat: cfg.player.repeat,
      show_queue: cfg.player.show_queue,
      mini_large: cfg.player.mini_large,
    },
  };
  if (a.eq_user_presets?.length) {
    sections.eq_presets = a.eq_user_presets;
  }
  return { glyph_settings: 1, app_version: appVersion, sections };
}

export function settingsToJson(cfg: Config, appVersion = "0.0.0"): string {
  return JSON.stringify(buildPayload(cfg, appVersion), null, 2);
}

export async function encodeSettings(cfg: Config, appVersion = "0.0.0"): Promise<string> {
  const json = new TextEncoder().encode(JSON.stringify(buildPayload(cfg, appVersion)));
  const packed = await pipe(json, new CompressionStream("deflate-raw"));
  return SETTINGS_PREFIX + b64url(packed);
}

export async function decodeSettings(raw: string): Promise<Record<string, unknown>> {
  const s = raw.trim();
  let bytes: Uint8Array;
  if (s.startsWith(SETTINGS_PREFIX)) {
    const body = s.slice(SETTINGS_PREFIX.length);
    try {
      bytes = await pipe(unb64url(body), new DecompressionStream("deflate-raw"), MAX_JSON, true);
    } catch {
      throw new Error("broken settings code");
    }
  } else {
    bytes = new TextEncoder().encode(s);
    if (bytes.length > MAX_JSON) throw new Error("settings too large");
  }
  let parsed: SharedSettingsPayload;
  try {
    parsed = JSON.parse(new TextDecoder().decode(bytes)) as SharedSettingsPayload;
  } catch {
    throw new Error("not Glyph settings");
  }
  if (!parsed || typeof parsed !== "object" || parsed.glyph_settings !== 1 || !parsed.sections || typeof parsed.sections !== "object") {
    throw new Error(parsed?.glyph_settings && parsed.glyph_settings > 1 ? "settings from a newer Glyph; update first" : "not Glyph settings");
  }
  return parsed.sections;
}

const isObj = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
const filterProviders = (v: unknown): ProviderId[] | null =>
  Array.isArray(v) ? v.filter((x): x is ProviderId => typeof x === "string" && VALID_PROVIDERS.has(x as ProviderId)) : null;

/** Applies decoded shared sections onto `cfg` in place; returns whether anything changed. */
export function applySharedSettings(cfg: Config, sections: Record<string, unknown>): boolean {
  const before = JSON.stringify(cfg);

  if (isObj(sections.general) && typeof sections.general.language === "string") {
    cfg.language = sections.general.language;
  }
  if (isObj(sections.providers)) {
    const enabled = filterProviders(sections.providers.enabled);
    const priority = filterProviders(sections.providers.priority);
    if (enabled?.length) cfg.providers.enabled = enabled;
    if (priority?.length) cfg.providers.priority = priority;
  }
  if (isObj(sections.visuals)) {
    const v = sections.visuals;
    if (typeof v.intensity === "number") cfg.visuals.intensity = Math.max(0, Math.min(2, v.intensity));
    if (typeof v.music_reactivity === "number") cfg.visuals.music_reactivity = Math.max(0, Math.min(2, v.music_reactivity));
    if (typeof v.ascii_covers === "boolean") cfg.visuals.ascii_covers = v.ascii_covers;
    if (typeof v.startup_animation === "boolean") cfg.visuals.startup_animation = v.startup_animation;
    if (typeof v.ascii_effects === "boolean") cfg.visuals.ascii_effects = v.ascii_effects;
    if (typeof v.theme === "string" && v.theme.trim()) cfg.visuals.theme = v.theme.trim();
    if (typeof v.throttle_unfocused === "boolean") cfg.visuals.throttle_unfocused = v.throttle_unfocused;
    if (typeof v.glow_shape === "string") cfg.visuals.glow_shape = v.glow_shape;
    if (typeof v.performance_mode === "boolean") cfg.visuals.performance_mode = v.performance_mode;
    if (typeof v.render_mode === "string") cfg.visuals.render_mode = v.render_mode;
    if (typeof v.solid_color === "string") cfg.visuals.solid_color = v.solid_color;
    if (typeof v.bg_image === "string") cfg.visuals.bg_image = v.bg_image;
    else if (v.bg_image === null) delete cfg.visuals.bg_image;
    if (typeof v.bg_dim === "number") cfg.visuals.bg_dim = v.bg_dim;
    if (typeof v.bg_blur === "number") cfg.visuals.bg_blur = v.bg_blur;
  }
  if (isObj(sections.sharing)) {
    const s = sections.sharing;
    if (typeof s.glyph_links === "boolean") cfg.sharing.glyph_links = s.glyph_links;
    if (Array.isArray(s.profile_sections)) {
      cfg.sharing.profile_sections = s.profile_sections.filter(
        (x): x is ProfileSection => x === "playlists" || x === "recent",
      );
    }
  }
  if (isObj(sections.stats) && typeof sections.stats.enabled === "boolean") {
    cfg.stats.enabled = sections.stats.enabled;
  }
  if (isObj(sections.playback)) {
    const p = sections.playback;
    if (p.shuffle === "smart" || p.shuffle === "random") cfg.playback.shuffle = p.shuffle;
    if (typeof p.skip_unplayable === "boolean") cfg.playback.skip_unplayable = p.skip_unplayable;
    if (typeof p.patient_search === "boolean") cfg.playback.patient_search = p.patient_search;
  }
  if (isObj(sections.lists) && isObj(sections.lists.sort)) {
    const next: Record<string, string> = {};
    for (const [k, val] of Object.entries(sections.lists.sort)) {
      if (typeof val === "string") next[k] = val;
    }
    cfg.lists.sort = next;
  }
  if (isObj(sections.audio)) {
    const a = sections.audio;
    if (typeof a.hide_explicit === "boolean") cfg.audio.hide_explicit = a.hide_explicit;
    if (typeof a.quality === "string") cfg.audio.quality = a.quality;
    if (typeof a.normalize === "boolean") cfg.audio.normalize = a.normalize;
    if (typeof a.eq_enabled === "boolean") cfg.audio.eq_enabled = a.eq_enabled;
    if (typeof a.eq_preset === "string") cfg.audio.eq_preset = a.eq_preset;
    if (Array.isArray(a.eq_gains) && a.eq_gains.length === 10) {
      cfg.audio.eq_gains = a.eq_gains.map((n) => (typeof n === "number" && Number.isFinite(n) ? n : 0));
    }
    if (typeof a.eq_bass_db === "number") cfg.audio.eq_bass_db = a.eq_bass_db;
    if (typeof a.eq_headroom_db === "number") cfg.audio.eq_headroom_db = a.eq_headroom_db;
  }
  if (isObj(sections.player)) {
    const p = sections.player;
    if (typeof p.shuffle === "boolean") cfg.player.shuffle = p.shuffle;
    if (p.repeat === "off" || p.repeat === "all" || p.repeat === "one") cfg.player.repeat = p.repeat;
    if (typeof p.show_queue === "boolean") cfg.player.show_queue = p.show_queue;
    if (typeof p.mini_large === "boolean") cfg.player.mini_large = p.mini_large;
  }
  if (Array.isArray(sections.eq_presets)) {
    for (const raw of sections.eq_presets) {
      if (!isObj(raw) || typeof raw.name !== "string" || !raw.name.trim() || !Array.isArray(raw.gains)) continue;
      const preset: EqPreset = {
        name: raw.name.trim(),
        gains: raw.gains.slice(0, 10).map((n) => (typeof n === "number" && Number.isFinite(n) ? n : 0)),
      };
      const idx = cfg.audio.eq_user_presets.findIndex((q) => q.name === preset.name);
      if (idx >= 0) cfg.audio.eq_user_presets[idx] = preset;
      else cfg.audio.eq_user_presets.push(preset);
    }
  }

  return JSON.stringify(cfg) !== before;
}
