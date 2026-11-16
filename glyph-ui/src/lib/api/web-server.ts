// glyph-server's public /v1 API as the web player uses it (shapes from
// glyph-protocol). Same origin by default (glyph.my/web talks to glyph.my/v1);
// VITE_GLYPH_SERVER points a local build at another server. Only metadata,
// encrypted account sync, profiles and MCP relay go through here: resolve asks
// for `direct=1`, so the answer holds the provider's own URL and never a
// server audio path.
import type { ProviderId, Track } from "./types";
import type { SyncItem } from "./web-e2e";
import type { McpSession } from "../settings/types";
import { installId } from "./web-store";

const BASE = String(import.meta.env.VITE_GLYPH_SERVER ?? "").replace(/\/+$/, "");

export function serverOrigin(): string {
  return BASE || (typeof location !== "undefined" ? location.origin : "");
}

/** glyph_protocol::PlaybackSource */
export type PlaybackSource =
  | { kind: "stream"; provider: ProviderId; url: string; mime?: string | null }
  | { kind: "embed"; provider: ProviderId; embed_id: string }
  | { kind: "preview"; provider: ProviderId; url: string };

export interface ResolveResponse { matched: Track | null; source: PlaybackSource | null; confidence: number; youtube_hint?: string | null }
export interface ServerArtistResponse {
  name: string;
  bio?: string | null;
  url?: string | null;
  photo?: string | null;
  banner?: string | null;
  fans?: number | null;
  verified?: boolean;
  tags?: string[];
  similar?: string[];
  links?: { label: string; url: string }[];
}
export type SearchEvent =
  | { event: "results"; provider: ProviderId; tracks: Track[] }
  | { event: "provider_error"; provider: ProviderId; message: string }
  | { event: "done" };
export interface LyricsResponse { provider: ProviderId; synced: { time_ms: number; text: string }[]; plain?: string | null; instrumental: boolean }
export interface HealthResponse { server_version: string; protocol_version: number; providers: ProviderId[]; capabilities?: string[]; accounts?: boolean; covers?: boolean }

export interface AccountInfo {
  id: string;
  handle: string;
  device_limit: number;
  created_at: number;
  has_password?: boolean;
}

export interface AccountSession {
  access_token: string;
  refresh_token: string;
  access_expires_in_s: number;
  device_id: string;
  account: AccountInfo;
}

export interface LoginStartResponse {
  login_id: string;
  code: string;
  browser_url: string;
  expires_in_s: number;
}

export type LoginPollResponse =
  | { status: "pending" }
  | { status: "device_limit" }
  | { status: "expired" }
  | { status: "done"; session: AccountSession };

export interface DeviceInfo {
  id: string;
  name: string;
  platform: string;
  created_at: number;
  last_seen_at: number;
  current: boolean;
}

export interface PlaybackStateWire {
  acknowledgements?: boolean;
  track: Track | null;
  playing: boolean;
  position_ms: number;
  duration_ms: number | null;
  volume: number;
  available: boolean;
}

export interface PlaybackOutputWire {
  device: {
    id: string;
    name: string;
    platform: string;
    current: boolean;
    last_seen?: number;
    created_at?: number;
    approximate_location?: string | null;
  };
  online: boolean;
  state: PlaybackStateWire;
}

export interface PlaybackPollResponse {
  devices: PlaybackOutputWire[];
}

export interface PlaybackReceipt {
  id: string;
  done: boolean;
  error?: string | null;
}

export interface SyncPullResponse {
  rev: number;
  items: SyncItem[];
  more?: boolean;
  reset?: boolean;
}

export interface SyncPushResponse {
  rev: number;
  accepted: number;
  stale?: SyncItem[];
}

export interface ProfileSettingsWire {
  public: boolean;
  show_recent: boolean;
  playlists: string[];
  sections?: string[];
  avatar?: string | null;
  banner?: string | null;
}

export interface ProfilePlaylistWire {
  id: string;
  name: string;
  songs: number;
  cover?: string | null;
  preview?: Track[];
}

export interface PublicProfile {
  handle: string;
  created_at: number;
  avatar?: string | null;
  banner?: string | null;
  private: boolean;
  own?: boolean;
  playlists: ProfilePlaylistWire[];
  recent: Track[];
  sections?: string[];
}

export interface PublicPlaylistFull {
  id: string;
  name: string;
  tracks: Track[];
  cover?: string | null;
  handle: string;
}

export interface SharedItem {
  id: string;
  kind: "song" | "playlist" | "album";
  title: string;
  subtitle?: string | null;
  cover_url?: string | null;
  tracks: Track[];
  created_at: number;
  expires_at: number;
}

export interface McpCall {
  id: string;
  name: string;
  arguments?: any;
}

export interface McpApproval {
  id: string;
  client: string;
  code: string;
}

export interface McpPullResponse {
  calls: McpCall[];
  approvals: McpApproval[];
}

/** An API error: the message starts with the server's code (`rate_limited: ...`). */
export class ApiError extends Error {
  constructor(readonly status: number, readonly code: string, message: string, readonly retryAfter?: number) {
    super(`${code}: ${message}`);
  }
}

let optionalAuth: (() => string | undefined) | null = null;

/** Registers a provider for the signed-in access token on public/member endpoints. */
export function setOptionalAuth(fn: (() => string | undefined) | null) {
  optionalAuth = fn;
}

function headers(json = false, token?: string): HeadersInit {
  const h: Record<string, string> = { "x-glyph-install-id": installId(), accept: "application/json" };
  if (json) h["content-type"] = "application/json";
  if (token) h.authorization = `Bearer ${token}`;
  return h;
}

async function fail(r: Response): Promise<never> {
  let code = "http_" + r.status, message = r.statusText || "request failed", retry: number | undefined;
  try {
    const e = await r.json();
    if (e?.code) code = String(e.code);
    if (e?.message) message = String(e.message);
    if (typeof e?.retry_after_s === "number") retry = e.retry_after_s;
  } catch { /* not JSON */ }
  throw new ApiError(r.status, code, message, retry);
}

async function getJson<T>(
  path: string,
  params: Record<string, string | number | undefined> = {},
  signal?: AbortSignal,
  token?: string,
  allowOptionalAuth = true,
): Promise<T> {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) if (v !== undefined && v !== "") q.set(k, String(v));
  const qs = q.toString();
  const auth = token ?? (allowOptionalAuth ? optionalAuth?.() : undefined);
  const r = await fetch(`${BASE}${path}${qs ? `?${qs}` : ""}`, { headers: headers(false, auth), signal, credentials: "omit" });
  if (!r.ok) return fail(r);
  return r.json() as Promise<T>;
}

async function sendJson<T>(
  method: string,
  path: string,
  body?: unknown,
  token?: string,
  signal?: AbortSignal,
  allowOptionalAuth = true,
): Promise<T> {
  const hasBody = body !== undefined;
  const auth = token ?? (allowOptionalAuth ? optionalAuth?.() : undefined);
  const r = await fetch(`${BASE}${path}`, {
    method,
    headers: headers(hasBody, auth),
    body: hasBody ? JSON.stringify(body) : undefined,
    signal,
    credentials: "omit",
  });
  if (!r.ok) return fail(r);
  if (r.status === 204) return undefined as T;
  const text = await r.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

/** The wire track (glyph_protocol::Track) without UI-only fields. */
export function wireTrack(t: Track): Track {
  const { id, source, title, artist, album, duration_ms, cover_url, isrc, permalink, external_ids, explicit } = t;
  return { id, source, title, artist, album, duration_ms, cover_url, isrc, permalink, external_ids, explicit };
}

export const server = {
  health: () => getJson<HealthResponse>("/v1/health", {}, undefined, undefined, false),

  /** `/v1/search?q=` NDJSON: calls `onEvent` for every line until `done`. */
  async search(q: string, page: number, onEvent: (e: SearchEvent) => void, signal?: AbortSignal): Promise<void> {
    const r = await fetch(`${BASE}/v1/search?${new URLSearchParams({ q, page: String(page) })}`, { headers: headers(false, optionalAuth?.()), signal, credentials: "omit" });
    if (!r.ok) return fail(r);
    const reader = r.body?.getReader();
    if (!reader) {
      for (const line of (await r.text()).split("\n")) if (line.trim()) onEvent(JSON.parse(line));
      return;
    }
    const dec = new TextDecoder();
    let buf = "";
    for (;;) {
      const { value, done } = await reader.read();
      if (value) buf += dec.decode(value, { stream: true });
      let nl: number;
      while ((nl = buf.indexOf("\n")) >= 0) {
        const line = buf.slice(0, nl).trim();
        buf = buf.slice(nl + 1);
        if (!line) continue;
        try { onEvent(JSON.parse(line) as SearchEvent); } catch { /* a broken line: skip it */ }
      }
      if (done) break;
    }
    if (buf.trim()) { try { onEvent(JSON.parse(buf) as SearchEvent); } catch { /* ignore */ } }
  },

  resolve: (track: Track, priority: ProviderId[], youtubeOnly = false) =>
    sendJson<ResolveResponse>("POST", "/v1/resolve?direct=1", { track: wireTrack(track), priority, youtube_only: youtubeOnly }),

  cover: (track: Track) =>
    sendJson<{ url?: string | null }>("POST", "/v1/cover", wireTrack(track)),

  async lyrics(t: Track): Promise<LyricsResponse | null> {
    try {
      return await getJson<LyricsResponse>("/v1/lyrics", { title: t.title, artist: t.artist, album: t.album, duration: t.duration_ms ? Math.round(t.duration_ms / 1000) : undefined });
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) return null;
      throw e;
    }
  },

  similar: (artist: string, title: string) => getJson<{ tracks: Track[] }>("/v1/similar", { artist, title }).then((r) => r.tracks ?? []),

  /** Popular songs right now (Deezer's public chart). Empty on servers without `/v1/chart`. */
  chart: () => getJson<{ tracks: Track[] }>("/v1/chart").then((r) => r.tracks ?? [], () => [] as Track[]),

  /** Look up normalized artist metadata (bio, photo, banner, fans, etc.) cached on the server. */
  artist: (name: string, lang = "en") =>
    getJson<ServerArtistResponse>("/v1/artist", { name, lang }),

  // ---- Account & Auth ----
  loginStart: (device_name: string, platform: string, pake?: string) =>
    sendJson<LoginStartResponse>("POST", "/v1/account/login/start", { device_name, platform, ...(pake ? { pake } : {}) }, undefined, undefined, false),

  loginPoll: (login_id: string) =>
    getJson<LoginPollResponse>("/v1/account/login/poll", { login_id }, undefined, undefined, false),

  tokenRefresh: (refresh_token: string) =>
    sendJson<AccountSession>("POST", "/v1/account/token/refresh", { refresh_token }, undefined, undefined, false),

  passwordSalt: (handle: string) =>
    getJson<{ salt: string }>(`/v1/account/password/salt/${encodeURIComponent(handle)}`, {}, undefined, undefined, false),

  passwordRegister: (req: { handle: string; salt: string; key: string; device_name: string; platform: string }) =>
    sendJson<{ session: AccountSession; recovery_code: string } | AccountSession>("POST", "/v1/account/password/register", req, undefined, undefined, false),

  passwordLogin: (req: { handle: string; key: string; device_name: string; platform: string }) =>
    sendJson<AccountSession>("POST", "/v1/account/password/login", req, undefined, undefined, false),

  passwordCheck: (token: string, key: string) =>
    sendJson<void>("POST", "/v1/account/password/check", { key }, token),

  passwordSet: (token: string, req: { salt: string; key: string }) =>
    sendJson<AccountInfo>("PUT", "/v1/account/password", req, token),

  passkeyLink: (token: string) =>
    sendJson<{ url: string; expires_in_s: number }>("POST", "/v1/account/passkey/link", {}, token),

  accountGet: (token: string) =>
    getJson<AccountInfo>("/v1/account", {}, undefined, token),

  accountPatch: (token: string, handle: string) =>
    sendJson<AccountInfo>("PATCH", "/v1/account", { handle }, token),

  accountDelete: (token: string) =>
    sendJson<{ ok: boolean }>("DELETE", "/v1/account", undefined, token),

  loginApprove: (token: string, code: string) =>
    sendJson<{ ok: boolean; pake?: string | null }>("POST", "/v1/account/login/approve", { code }, token),

  recoveryCode: (token: string) =>
    sendJson<{ recovery_code: string }>("POST", "/v1/account/recovery_code", {}, token),

  devicesList: (token: string) =>
    getJson<{ devices: DeviceInfo[]; limit: number }>("/v1/devices", {}, undefined, token),

  deviceRemove: (token: string, id: string) =>
    sendJson<{ ok: boolean }>("DELETE", `/v1/devices/${encodeURIComponent(id)}`, undefined, token),

  deviceSignOut: (token: string) =>
    sendJson<{ ok: boolean }>("POST", "/v1/devices/self/signout", {}, token),

  deviceSignOutAll: (token: string) =>
    sendJson<{ ok: boolean }>("POST", "/v1/devices/all/signout", {}, token),

  playbackPoll: (token: string, state: PlaybackStateWire) =>
    sendJson<PlaybackPollResponse>(
      "POST",
      "/v1/devices/playback/poll",
      { ...state, track: state.track ? wireTrack(state.track) : null },
      token,
    ),

  playbackCommand: (token: string, body: { target: string; command: Record<string, unknown> }, confirm = true) =>
    sendJson<{ id?: string | null }>(
      "POST",
      confirm ? "/v1/devices/playback/command?confirm=1" : "/v1/devices/playback/command",
      body,
      token,
    ),

  playbackReceipt: (token: string, id: string) =>
    sendJson<PlaybackReceipt>("POST", "/v1/devices/playback/receipt", { id, done: false, error: null }, token),

  // ---- Sync ----
  syncPull: (token: string, since: number, limit = 1000) =>
    getJson<SyncPullResponse>("/v1/sync/pull", { since, limit }, undefined, token),

  syncPush: (token: string, items: SyncItem[]) =>
    sendJson<SyncPushResponse>("POST", "/v1/sync/push", { items }, token),

  // ---- Profile ----
  profileSettingsGet: (token: string) =>
    getJson<ProfileSettingsWire>("/v1/profile", {}, undefined, token),

  profileSettingsPatch: (token: string, patch: { public?: boolean; show_recent?: boolean; playlists?: string[]; sections?: string[] }) =>
    sendJson<ProfileSettingsWire>("PATCH", "/v1/profile", patch, token),

  async profilePutImage(token: string, kind: "avatar" | "banner", mime: string, bytes: Uint8Array): Promise<{ sha?: string; avatar?: string; banner?: string }> {
    const r = await fetch(`${BASE}/v1/profile/${kind}`, {
      method: "PUT",
      headers: { ...headers(false, token), "content-type": mime || "image/jpeg" },
      body: bytes as BufferSource,
      credentials: "omit",
    });
    if (!r.ok) return fail(r);
    return r.json();
  },

  profileDeleteImage: (token: string, kind: "avatar" | "banner") =>
    sendJson<void>("DELETE", `/v1/profile/${kind}`, undefined, token),

  profilePutRecent: (token: string, tracks: Track[]) =>
    sendJson<void>("PUT", "/v1/profile/recent", { tracks: tracks.slice(0, 20).map(wireTrack) }, token),

  publicProfile: (handle: string, token?: string) =>
    getJson<PublicProfile>(`/v1/profiles/${encodeURIComponent(handle.replace(/^@/, ""))}`, {}, undefined, token),

  publicProfilePlaylist: (handle: string, id: string, token?: string) =>
    getJson<PublicPlaylistFull>(`/v1/profiles/${encodeURIComponent(handle.replace(/^@/, ""))}/playlists/${encodeURIComponent(id)}`, {}, undefined, token),

  profileImageUrl: (handle: string, sha: string) =>
    `${serverOrigin()}/v1/profiles/${encodeURIComponent(handle.replace(/^@/, ""))}/images/${encodeURIComponent(sha)}`,

  // ---- Share ----
  shareCreate: (req: { kind: "song" | "playlist" | "album"; title: string; subtitle?: string | null; cover_url?: string | null; tracks: Track[] }, token?: string) =>
    sendJson<{ id: string; url: string; expires_at: number }>("POST", "/v1/share", { ...req, tracks: req.tracks.map(wireTrack) }, token),

  shareGet: (id: string) =>
    getJson<SharedItem>(`/v1/share/${encodeURIComponent(id)}`),

  // ---- MCP Relay ----
  mcpToken: (token: string, name?: string) =>
    sendJson<{ token: string; url: string; session_id?: string }>("POST", "/v1/mcp/token", { name: name?.trim() || undefined }, token),

  mcpSessions: (token: string) =>
    getJson<McpSession[]>("/v1/mcp/sessions", {}, undefined, token),

  mcpRenameSession: (token: string, id: string, name: string) =>
    sendJson<McpSession>("PATCH", `/v1/mcp/sessions/${encodeURIComponent(id)}`, { name }, token),

  mcpRevokeSession: (token: string, id: string) =>
    sendJson<void>("DELETE", `/v1/mcp/sessions/${encodeURIComponent(id)}`, undefined, token),

  mcpClaimOAuth: (token: string, id: string) =>
    sendJson<McpApproval>("POST", "/v1/mcp/oauth/claim", { id }, token),

  mcpPull: (token: string, signal?: AbortSignal) =>
    getJson<McpPullResponse>("/v1/mcp/pull", {}, signal, token),

  mcpApprove: (token: string, req: { id: string; allow: boolean }) =>
    sendJson<void>("POST", "/v1/mcp/approve", req, token),

  mcpResult: (token: string, req: { id: string; result: unknown }) =>
    sendJson<void>("POST", "/v1/mcp/result", req, token),
};
