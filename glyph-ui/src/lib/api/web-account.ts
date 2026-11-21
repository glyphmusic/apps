// Account sign-in, E2E keyring, library/stats sync, and public profiles for the
// browser player (/web). Mirrors glyph-core::{account, sync, e2e, e2e_sync} and
// glyph-tauri::account commands.
import { encodePicture, pictureCanvas } from "../picture";
import type {
  Account, BackendEvents, Config, Library, ListKey, Playlist, Profile, RecentItem, SearchItem, Track,
} from "./types";
import type { ProfileSettingsData } from "../settings/types";
import { listId, sameList, trackKey } from "./types";
import { adminPathOk } from "../admin/api";
import {
  b64url, DataKey, deriveLoginKey, derivePasswordKey, DeviceKey, openFromDevice, randomBytes,
  sealToDevice, splitPairCode, suspiciousSignInSalt, SYNC_KIND, type SyncItem, unwrapWithPassword, wrapWithPassword,
} from "./web-e2e";
import {
  ApiError, server, serverOrigin, setOptionalAuth, wireTrack,
  type AccountSession, type DeviceInfo, type ProfileSettingsWire, type PublicProfile,
} from "./web-server";
import type { MonthStats, WebStats } from "./web-stats";
import { installId, load, loadBig, remove, save } from "./web-store";

const MAX_PUSH_ITEMS = 500;
const MAX_PUSH_BYTES = 1_500_000;
const MAX_SINGLE_ITEM_BYTES = 980_000;

interface PushEntry {
  sealed: SyncItem;
  plain?: SyncItem;
}

function chunkPushEntries(entries: PushEntry[]): PushEntry[][] {
  const enc = new TextEncoder();
  const chunks: PushEntry[][] = [];
  let cur: PushEntry[] = [];
  let curBytes = 12; // `{"items":[]}` overhead
  for (const entry of entries) {
    const itemBytes = enc.encode(JSON.stringify(entry.sealed)).length;
    if (itemBytes > MAX_SINGLE_ITEM_BYTES) {
      console.warn("[sync] skipping oversized sync item:", entry.sealed.kind, itemBytes);
      continue;
    }
    const nextBytes = curBytes + (cur.length > 0 ? 1 : 0) + itemBytes;
    if (cur.length > 0 && (cur.length >= MAX_PUSH_ITEMS || nextBytes > MAX_PUSH_BYTES)) {
      chunks.push(cur);
      cur = [];
      curBytes = 12;
    }
    cur.push(entry);
    curBytes += (cur.length > 1 ? 1 : 0) + itemBytes;
  }
  if (cur.length > 0) chunks.push(cur);
  return chunks;
}

interface StoredAccountSession {
  access_token: string;
  refresh_token: string;
  expires_at: number; // unix seconds
  device_id: string;
  account: {
    id: string;
    handle: string;
    device_limit: number;
    created_at: number;
    has_password: boolean;
  };
}

interface SyncJournal {
  rev: number;
  /** `${kind}\0${key}` -> { hash, rev, updated_at } */
  known: Record<string, { hash: string; rev: number; updated_at: number }>;
  /** blinded hex -> [plainKind, plainKey] */
  blind: Record<string, [string, string]>;
  /** keyring items from server: key -> value */
  keyring: Record<string, any>;
}

interface KeyStore {
  device_key: string; // base64url 32-byte secret
  data_key?: string | null; // base64url 32-byte DataKey
  pw_key?: string | null; // base64url 32-byte vault key
}

interface GuestSnapshot {
  library: Library;
  stats: Record<string, MonthStats>;
}

const norm = (s: string) => s.trim().toLowerCase();
const nowSec = () => Math.floor(Date.now() / 1000);

function detectPlatform(): string {
  const ua = typeof navigator !== "undefined" ? navigator.userAgent.toLowerCase() : "";
  if (ua.includes("android")) return "android";
  if (ua.includes("iphone") || ua.includes("ipad") || ua.includes("ipod")) return "ios";
  if (ua.includes("win")) return "windows";
  if (ua.includes("mac")) return "macos";
  if (ua.includes("linux")) return "linux";
  return "other";
}

function detectDeviceName(): string {
  const ua = typeof navigator !== "undefined" ? navigator.userAgent : "";
  const nav = typeof navigator !== "undefined" ? (navigator as Navigator & { standalone?: boolean }) : undefined;
  const standalone = nav?.standalone === true
    || (typeof window !== "undefined" && window.matchMedia?.("(display-mode: standalone)").matches);
  if (standalone) {
    if (/iPhone|iPod/i.test(ua)) return "Glyph (iPhone)";
    if (/iPad/i.test(ua) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1)) return "Glyph (iPad)";
    if (/Android/i.test(ua)) return "Glyph (Android)";
    return "Glyph Web App";
  }
  const browser = ua.includes("Firefox/")
    ? "Firefox"
    : ua.includes("Edg/")
      ? "Edge"
      : ua.includes("Chrome/")
        ? "Chrome"
        : ua.includes("Safari/")
          ? "Safari"
          : "Browser";
  return `Glyph Web (${browser})`;
}

function trackSyncId(t: Track): string {
  return `${t.source}:${t.id}`;
}

function canonicalTrack(t: Track): Track {
  const w = wireTrack(t);
  const out: Track = { id: w.id, source: w.source, title: w.title, artist: w.artist };
  if (w.album != null) out.album = w.album;
  if (w.duration_ms != null) out.duration_ms = w.duration_ms;
  if (w.cover_url != null) out.cover_url = w.cover_url;
  if (w.isrc != null) out.isrc = w.isrc;
  if (w.permalink != null) out.permalink = w.permalink;
  if (w.external_ids && Object.keys(w.external_ids).length > 0) out.external_ids = w.external_ids;
  if (w.explicit) out.explicit = true;
  return out;
}

function playlistSyncValue(p: Playlist) {
  return {
    id: p.id,
    name: p.name,
    description: p.description ?? "",
    tracks: p.tracks.map(canonicalTrack),
    created_at: p.created_at ?? 0,
  };
}

function listSyncKey(l: ListKey): string {
  return listId(l);
}

function searchSyncKey(s: SearchItem): string {
  if (s.kind === "query") return `query:${norm(s.text)}`;
  if (s.kind === "artist") return `artist:${norm(s.name)}`;
  return `song:${trackSyncId(s.track)}`;
}

function legacySearchSyncKey(s: SearchItem): string {
  if (s.kind === "query") return `q:${norm(s.text)}`;
  if (s.kind === "artist") return `a:${norm(s.name)}`;
  return `t:${trackKey(s.track)}`;
}

function canonicalValue(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(canonicalValue);
  if (v && typeof v === "object") {
    const out: Record<string, unknown> = {};
    for (const k of Object.keys(v as Record<string, unknown>).sort()) {
      const val = (v as Record<string, unknown>)[k];
      if (val !== undefined) out[k] = canonicalValue(val);
    }
    return out;
  }
  return v;
}

function quickHash(v: unknown): string {
  return JSON.stringify(canonicalValue(v));
}

function pickJournalKey(journal: SyncJournal, kind: string, primary: string, fallback?: string): string {
  if (journal.known[`${kind}\0${primary}`]) return primary;
  if (fallback && journal.known[`${kind}\0${fallback}`]) return fallback;
  return primary;
}

function normalizeSections(raw?: readonly string[] | null): ("playlists" | "recent")[] {
  const out: ("playlists" | "recent")[] = [];
  for (const s of raw ?? []) {
    if ((s === "playlists" || s === "recent") && !out.includes(s)) out.push(s);
  }
  if (!out.includes("playlists")) out.push("playlists");
  if (!out.includes("recent")) out.push("recent");
  return out;
}

function wireToProfileSettings(handle: string, w: Partial<ProfileSettingsWire>, fallback?: string[]): ProfileSettingsData {
  const av = w.avatar ?? null;
  const bn = w.banner ?? null;
  return {
    public: Boolean(w.public),
    show_recent: w.show_recent ?? true,
    playlists: w.playlists ?? [],
    sections: normalizeSections(w.sections?.length ? w.sections : fallback),
    avatar: av,
    banner: bn,
    avatar_url: av ? server.profileImageUrl(handle, av) : null,
    banner_url: bn ? server.profileImageUrl(handle, bn) : null,
  };
}

export interface WebAccountHost {
  getConfig: () => Config;
  setConfig: (c: Config) => void;
  getLibrary: () => Library;
  setLibrary: (l: Library) => void;
  getLyricsOffsets: () => Record<string, number>;
  setLyricsOffsets: (o: Record<string, number>) => void;
  stats: WebStats;
  emit: <K extends keyof BackendEvents>(ev: K, p: BackendEvents[K]) => void;
  toast: (key: string, kind?: "ok" | "info", vars?: Record<string, string | number>) => void;
  onSessionChange?: () => void;
  onBeforeAccountSwitch?: (prevAccountId: string | null) => void;
  onAfterAccountSwitch?: (nextAccountId: string | null, signOut: boolean) => void;
}

export class WebAccountManager {
  private session: StoredAccountSession | null;
  private devices: DeviceInfo[] = [];
  /** Space the account uses on the server (null: unknown or no limit there). */
  private storage: { used_bytes: number; limit_bytes: number } | null = null;
  private syncing = false;
  private syncError: string | null = null;
  private lastSyncedAt: number | null = null;
  private keysLocked = false;
  private accountsAvailable = true;
  private syncTimer: ReturnType<typeof setTimeout> | null = null;
  private recentTimer: ReturnType<typeof setTimeout> | null = null;
  private syncCooldownUntil = 0;
  private resyncPending = false;
  private webLogins: { domain: string; at: number }[] = [];
  private refreshing: Promise<string | null> | null = null;

  constructor(private host: WebAccountHost) {
    this.session = load<StoredAccountSession>("account") ?? null;
    this.webLogins = load<{ domain: string; at: number }[]>("web_logins") ?? [];
    setOptionalAuth(() => this.optionalToken());
    if (this.session) {
      // Load cached devices and kick off an initial refresh + sync
      this.devices = load<DeviceInfo[]>(`devices.${this.session.account.id}`) ?? [];
      setTimeout(() => {
        void this.refreshAccountAndSync();
      }, 150);
    }
  }

  /** Cached non-expired access token for public/member requests (never blocks on refresh). */
  optionalToken(): string | undefined {
    if (!this.accountsAvailable || !this.session) return undefined;
    if (this.session.expires_at - nowSec() <= 15) return undefined;
    return this.session.access_token;
  }

  setAccountsAvailable(ok: boolean) {
    this.accountsAvailable = ok;
  }

  isSignedIn(): boolean {
    return this.session !== null;
  }

  uiAccount(): Account | null {
    if (!this.session) return null;
    return {
      handle: this.session.account.handle,
      syncing: this.syncing,
    };
  }

  /** Refreshes the access token once (deduped) or clears a revoked session cleanly. */
  private async refreshSessionToken(): Promise<string | null> {
    if (this.refreshing) return this.refreshing;
    const job = (async () => {
      const s = this.session;
      if (!s) return null;
      try {
        const next = await server.tokenRefresh(s.refresh_token);
        this.saveSession(next);
        return next.access_token;
      } catch (e) {
        if (e instanceof ApiError && (e.status === 401 || e.code === "unauthorized")) {
          this.clearSessionAndRestoreGuest();
          return null;
        }
        return s.access_token;
      } finally {
        this.refreshing = null;
      }
    })();
    this.refreshing = job;
    return job;
  }

  /** Valid access token, refreshing automatically when within 60s of expiry. */
  async validToken(): Promise<string> {
    const s = this.session;
    if (!s) throw new Error("signed_out");
    if (s.expires_at - nowSec() > 60) return s.access_token;
    const next = await this.refreshSessionToken();
    if (!next || !this.session) throw new Error("signed_out");
    return next;
  }

  /** Runs `fn` with a valid access token, refreshing once on 401. */
  withToken<T>(fn: (token: string) => Promise<T>): Promise<T> {
    return this.runAuthed(fn);
  }

  private isUnauthorized(e: unknown): boolean {
    return e instanceof ApiError && (e.status === 401 || e.code === "unauthorized");
  }

  private async runAuthed<T>(fn: (token: string) => Promise<T>): Promise<T> {
    const token = await this.validToken();
    try {
      return await fn(token);
    } catch (e) {
      if (!this.isUnauthorized(e)) throw e;
      const prevToken = token;
      const fresh = await this.refreshSessionToken();
      if (!fresh || !this.session) throw new Error("signed_out");
      if (fresh === prevToken) {
        this.clearSessionAndRestoreGuest();
        throw new Error("signed_out");
      }
      try {
        return await fn(fresh);
      } catch (e2) {
        if (this.isUnauthorized(e2)) {
          this.clearSessionAndRestoreGuest();
          throw new Error("signed_out");
        }
        throw e2;
      }
    }
  }

  private saveSession(ws: AccountSession) {
    this.session = {
      access_token: ws.access_token,
      refresh_token: ws.refresh_token,
      expires_at: nowSec() + (ws.access_expires_in_s || 3600),
      device_id: ws.device_id,
      account: {
        id: ws.account.id,
        handle: ws.account.handle,
        device_limit: ws.account.device_limit ?? 3,
        created_at: ws.account.created_at,
        has_password: Boolean(ws.account.has_password),
      },
    };
    save("account", this.session);
  }

  private getKeyStore(accountId: string): { store: KeyStore; dk: DeviceKey; dataKey: DataKey | null } {
    let store = load<KeyStore>(`keys.${accountId}`);
    let dk = store?.device_key ? DeviceKey.fromB64(store.device_key) : null;
    if (!dk) {
      dk = DeviceKey.generate();
      store = { ...store, device_key: dk.toB64() };
      save(`keys.${accountId}`, store);
    }
    const dataKey = store?.data_key ? DataKey.fromB64(store.data_key) : null;
    return { store: store!, dk, dataKey };
  }

  private saveDataKey(accountId: string, k: DataKey, pwKey?: Uint8Array) {
    const { store } = this.getKeyStore(accountId);
    store.data_key = k.toB64();
    if (pwKey) store.pw_key = b64url(pwKey);
    save(`keys.${accountId}`, store);
    this.keysLocked = false;
  }

  private getJournal(accountId: string): SyncJournal {
    return (
      load<SyncJournal>(`sync.${accountId}`) ?? {
        rev: 0,
        known: {},
        blind: {},
        keyring: {},
      }
    );
  }

  private saveJournal(accountId: string, j: SyncJournal) {
    save(`sync.${accountId}`, j);
  }

  // ---- Guest library backup & switch ----

  private stashGuestBeforeSignIn(accountId: string) {
    this.host.onBeforeAccountSwitch?.(null);
    const curLib = this.host.getLibrary();
    const curStats = this.host.stats.ownMonths();
    const guestCount = curLib.liked.length + curLib.playlists.length + curLib.saved_albums.length + curLib.followed.length;
    if (guestCount > 0 || Object.keys(curStats).length > 0) {
      save("guest_backup", { library: curLib, stats: curStats });
    }
    const cachedLib = load<Library>(`account_lib.${accountId}`);
    if (cachedLib) {
      this.host.setLibrary(cachedLib);
    } else {
      // Too big for localStorage: the IndexedDB copy fills in as soon as it is read.
      void loadBig<Library>(`account_lib.${accountId}`).then((lib) => {
        const cur = this.host.getLibrary();
        if (lib && !cur.liked.length && !cur.playlists.length) this.host.setLibrary(lib);
      });
      this.host.setLibrary({
        liked: [],
        recent: [],
        playlists: [],
        saved_albums: [],
        pinned: [],
        recent_lists: [],
        followed: [],
        downloads: [],
        search_history: [],
        hidden_sidebar: [],
      });
    }
    this.host.onAfterAccountSwitch?.(accountId, false);
  }

  private checkGuestMergePrompt() {
    const backup = load<GuestSnapshot>("guest_backup");
    if (!backup) return;
    const songs = backup.library.liked.length;
    const playlists = backup.library.playlists.length;
    const statsMonths = Object.keys(backup.stats ?? {}).length;
    if (songs > 0 || playlists > 0 || statsMonths > 0) {
      this.host.emit("account_merge", { needed: true });
    } else {
      remove("guest_backup");
    }
  }

  private clearSessionAndRestoreGuest(accountDeleted = false) {
    const prevId = this.session?.account.id ?? null;
    if (!accountDeleted) {
      this.host.onBeforeAccountSwitch?.(prevId);
    }
    if (prevId && !accountDeleted) {
      save(`account_lib.${prevId}`, this.host.getLibrary());
    }
    this.session = null;
    this.devices = [];
    this.syncing = false;
    this.syncError = null;
    this.keysLocked = false;
    remove("account");
    const backup = load<GuestSnapshot>("guest_backup");
    if (backup) {
      this.host.setLibrary(backup.library);
      remove("guest_backup");
    } else {
      this.host.setLibrary({
        liked: [],
        recent: [],
        playlists: [],
        saved_albums: [],
        pinned: [],
        recent_lists: [],
        followed: [],
        downloads: [],
        search_history: [],
        hidden_sidebar: [],
      });
    }
    this.host.onAfterAccountSwitch?.(null, true);
    this.host.emit("account", null);
    this.host.emit("account_reset", { signed_in: false });
    this.host.onSessionChange?.();
  }

  private async finishSignIn(ws: AccountSession, password?: string) {
    const prevId = this.session?.account.id;
    const isNewSignIn = prevId !== ws.account.id;
    if (isNewSignIn) {
      this.stashGuestBeforeSignIn(ws.account.id);
    }
    this.saveSession(ws);
    this.host.emit("account", this.uiAccount());
    this.host.emit("account_reset", { signed_in: true });
    if (password) {
      try {
        const pwKey = await derivePasswordKey(password, ws.account.id);
        const { store } = this.getKeyStore(ws.account.id);
        store.pw_key = b64url(pwKey);
        save(`keys.${ws.account.id}`, store);
      } catch { /* non-fatal */ }
    }
    this.host.onSessionChange?.();
    await this.refreshAccountAndSync();
    if (isNewSignIn) {
      this.checkGuestMergePrompt();
    }
  }

  async refreshAccountAndSync() {
    if (!this.session) return;
    try {
      const acc = await this.runAuthed((t) => server.accountGet(t)).catch((e) => {
        if (String((e as Error)?.message ?? e) === "signed_out") throw e;
        return null;
      });
      if (!this.session) return;
      if (acc) {
        this.session.account = {
          id: acc.id,
          handle: acc.handle,
          device_limit: acc.device_limit ?? this.session.account.device_limit,
          created_at: acc.created_at,
          has_password: Boolean(acc.has_password),
        };
        save("account", this.session);
      }
      const devs = await this.runAuthed((t) => server.devicesList(t)).catch(() => null);
      if (devs && this.session) {
        this.devices = devs.devices;
        this.session.account.device_limit = devs.limit || this.session.account.device_limit;
        save(`devices.${this.session.account.id}`, this.devices);
      }
      this.storage = await this.runAuthed((t) => server.accountStorage(t)).catch(() => null);
      if (this.session) await this.syncNow();
    } catch { /* offline or signed out cleanly */ }
  }

  // ---- E2E Keyring Resolution & Sync ----

  private async resolveDataKey(accountId: string, deviceId: string, hasPassword: boolean, keyring: Record<string, any>): Promise<DataKey | null> {
    const { store, dk, dataKey: existing } = this.getKeyStore(accountId);
    const main = keyring.main;

    if (existing) {
      if (!main || !main.kid || (await existing.kid()) === main.kid) {
        this.keysLocked = false;
        return existing;
      }
    }

    // 1. Check `env:<deviceId>` sealed to our device's X25519 key
    const envVal = keyring[`env:${deviceId}`];
    if (envVal) {
      const opened = await openFromDevice(deviceId, dk, envVal);
      if (opened && (!main?.kid || (await opened.kid()) === main.kid)) {
        this.saveDataKey(accountId, opened);
        return opened;
      }
    }

    // 2. Check `main.escrow` (passkey-only accounts without password)
    if (main?.escrow && typeof main.escrow === "string") {
      const opened = DataKey.fromB64(main.escrow);
      if (opened && (!main.kid || (await opened.kid()) === main.kid)) {
        this.saveDataKey(accountId, opened);
        return opened;
      }
    }

    // 3. Check `main.pw` with cached `pw_key`
    if (main?.pw && main?.kid && store.pw_key) {
      const pwKeyBytes = DataKey.fromB64(store.pw_key)?.raw;
      if (pwKeyBytes) {
        const opened = await unwrapWithPassword(pwKeyBytes, accountId, main.kid, main.pw);
        if (opened) {
          this.saveDataKey(accountId, opened);
          return opened;
        }
      }
    }

    // 4. Brand-new account with no `main` entry yet: create DataKey
    if (!main) {
      const created = existing ?? DataKey.generate();
      this.saveDataKey(accountId, created);
      return created;
    }

    // Otherwise, the account has an E2E key we need the password (or device approval) to unlock
    if (hasPassword || main.pw) {
      this.keysLocked = true;
    }
    return null;
  }

  private async buildKeyringOutbox(accountId: string, deviceId: string, hasPassword: boolean, k: DataKey, journal: SyncJournal): Promise<SyncItem[]> {
    const { store, dk } = this.getKeyStore(accountId);
    const out: SyncItem[] = [];
    const now = Date.now();
    const kid = await k.kid();

    const pushKeyring = (key: string, val: any) => {
      const prev = journal.keyring[key];
      if (JSON.stringify(prev) === JSON.stringify(val)) return;
      journal.keyring[key] = val;
      const kKey = `${SYNC_KIND.KEYRING}\0${key}`;
      out.push({
        kind: SYNC_KIND.KEYRING,
        key,
        value: val,
        updated_at: now,
        deleted: false,
        rev: journal.known[kKey]?.rev ?? 0,
      });
    };

    // Publish our device public key `dev:<deviceId>`
    const pubB64 = dk.publicB64();
    const mac = await k.deviceMac(deviceId, dk.pub);
    pushKeyring(`dev:${deviceId}`, { v: 1, pk: pubB64, kid, mac });

    // Ensure `main` exists
    const main = journal.keyring.main;
    if (!main || main.kid !== kid) {
      if (hasPassword && store.pw_key) {
        const pwKeyBytes = DataKey.fromB64(store.pw_key)?.raw;
        if (pwKeyBytes) {
          const pwBox = await wrapWithPassword(pwKeyBytes, accountId, k);
          pushKeyring("main", { v: 1, kid, pw: pwBox });
        }
      } else if (!hasPassword) {
        pushKeyring("main", { v: 1, kid, escrow: k.toB64() });
      }
    }

    // Seal DataKey to any other device that published `dev:<otherId>` without a current `env:<otherId>`
    for (const [kName, devVal] of Object.entries(journal.keyring)) {
      if (!kName.startsWith("dev:")) continue;
      const otherId = kName.slice(4);
      if (!otherId || typeof devVal?.pk !== "string") continue;
      const existingEnv = journal.keyring[`env:${otherId}`];
      if (existingEnv && existingEnv.kid === kid) continue;
      const sealedEnv = await sealToDevice(otherId, devVal.pk, k, deviceId);
      if (sealedEnv) pushKeyring(`env:${otherId}`, sealedEnv);
    }

    return out;
  }

  private async withSyncRetry<T>(fn: () => Promise<T>): Promise<T> {
    for (let attempt = 0; ; attempt++) {
      try {
        return await fn();
      } catch (e) {
        if (!this.session) throw new Error("signed_out");
        if (e instanceof ApiError && e.status === 429) {
          if (e.code === "temporarily_blocked") {
            const waitMs = Math.min(900_000, Math.max(5_000, (e.retryAfter ?? 60) * 1000));
            this.syncCooldownUntil = Math.max(this.syncCooldownUntil, Date.now() + waitMs);
            throw e;
          }
          if (attempt < 4) {
            const waitMs = Math.min(65_000, Math.max(1_000, (e.retryAfter ?? 5) * 1000));
            await new Promise((r) => setTimeout(r, waitMs));
            if (!this.session) throw new Error("signed_out");
            continue;
          }
          this.syncCooldownUntil = Math.max(
            this.syncCooldownUntil,
            Date.now() + Math.min(65_000, Math.max(2_000, (e.retryAfter ?? 10) * 1000)),
          );
          throw e;
        }
        if (e instanceof ApiError && (e.status === 502 || e.status === 503 || e.status === 504) && attempt < 3) {
          await new Promise((r) => setTimeout(r, (1 << attempt) * 1500));
          if (!this.session) throw new Error("signed_out");
          continue;
        }
        throw e;
      }
    }
  }

  scheduleSync(delayMs = 1500) {
    if (!this.session) return;
    if (this.syncing) {
      this.resyncPending = true;
      return;
    }
    if (this.syncTimer) clearTimeout(this.syncTimer);
    const wait = Math.max(delayMs, this.syncCooldownUntil - Date.now());
    this.syncTimer = setTimeout(() => {
      this.syncTimer = null;
      void this.syncNow();
    }, wait);
  }

  scheduleRecentUpdate() {
    if (!this.session) return;
    if (this.recentTimer) clearTimeout(this.recentTimer);
    this.recentTimer = setTimeout(async () => {
      this.recentTimer = null;
      if (!this.session) return;
      try {
        const songs = this.host
          .getLibrary()
          .recent.flatMap((r) => (r.kind === "song" ? [r.track] : []))
          .slice(0, 12);
        await this.runAuthed((token) => server.profilePutRecent(token, songs));
      } catch { /* ignore */ }
    }, 2500);
  }

  async syncNow(): Promise<void> {
    if (!this.session) return;
    if (this.syncing) {
      this.resyncPending = true;
      return;
    }
    this.syncing = true;
    this.syncError = null;
    const s = this.session;
    const accountId = s.account.id;
    try {
      const journal = this.getJournal(accountId);

      // 1. Pull remote changes since journal.rev
      let pull = await this.runAuthed((t) => this.withSyncRetry(() => server.syncPull(t, journal.rev, 1000)));
      if (pull.reset) {
        journal.rev = 0;
        journal.known = {};
        pull = await this.runAuthed((t) => this.withSyncRetry(() => server.syncPull(t, 0, 1000)));
      }
      const allPulled: SyncItem[] = [...pull.items];
      let latestRev = pull.rev;
      let guard = 0;
      while (pull.more && guard++ < 200) {
        pull = await this.runAuthed((t) => this.withSyncRetry(() => server.syncPull(t, latestRev, 1000)));
        allPulled.push(...pull.items);
        latestRev = pull.rev;
      }

      // First collect keyring updates
      for (const it of allPulled) {
        if (it.kind === SYNC_KIND.KEYRING) {
          if (it.deleted || it.value == null) delete journal.keyring[it.key];
          else journal.keyring[it.key] = it.value;
          journal.known[`${it.kind}\0${it.key}`] = {
            hash: quickHash(it.value),
            rev: it.rev ?? latestRev,
            updated_at: it.updated_at,
          };
        }
      }

      const dataKey = await this.resolveDataKey(accountId, s.device_id, s.account.has_password, journal.keyring);

      // Decrypt and apply remote non-keyring items
      const plainRemote: SyncItem[] = [];
      for (const it of allPulled) {
        if (it.kind === SYNC_KIND.KEYRING) continue;
        if (it.kind === SYNC_KIND.E2E || it.kind === SYNC_KIND.E2E_PLAYLIST) {
          if (!dataKey) continue;
          const opened = await dataKey.openItem(it, (b) => journal.blind[b] ?? null);
          if (opened.ok && opened.item) {
            journal.blind[it.key] = [opened.item.kind, opened.item.key];
            plainRemote.push({ ...opened.item, rev: it.rev });
          }
        } else {
          plainRemote.push(it);
        }
      }

      if (plainRemote.length > 0) {
        this.applyRemoteItems(plainRemote, journal, latestRev);
      }
      journal.rev = latestRev;

      // 2. Build local outbox (only if we have DataKey)
      if (dataKey) {
        const keyringOut = await this.buildKeyringOutbox(accountId, s.device_id, s.account.has_password, dataKey, journal);
        const plainDiff = this.collectLocalDiff(journal);
        const pushEntries: PushEntry[] = keyringOut.map((sealed) => ({ sealed }));

        for (const pItem of plainDiff) {
          const sealed = await dataKey.sealItem(pItem);
          journal.blind[sealed.key] = [pItem.kind, pItem.key];
          const blindKnown = journal.known[`${sealed.kind}\0${sealed.key}`];
          sealed.rev = blindKnown?.rev ?? pItem.rev ?? 0;
          pushEntries.push({ sealed, plain: pItem });
        }

        for (const chunk of chunkPushEntries(pushEntries)) {
          const pushed = await this.runAuthed((t) => this.withSyncRetry(() => server.syncPush(t, chunk.map((e) => e.sealed))));
          journal.rev = pushed.rev;
          for (const entry of chunk) {
            if (entry.plain) {
              const pItem = entry.plain;
              if (pItem.kind === SYNC_KIND.LIKED_CLEAR && !pItem.deleted) {
                const likedPrefix = `${SYNC_KIND.LIKED}\0`;
                for (const k of Object.keys(journal.known)) {
                  if (k.startsWith(likedPrefix)) delete journal.known[k];
                }
              }
              const kKey = `${pItem.kind}\0${pItem.key}`;
              if (pItem.deleted) delete journal.known[kKey];
              else journal.known[kKey] = { hash: quickHash(pItem.value), rev: pushed.rev, updated_at: pItem.updated_at };
            }
            const sItem = entry.sealed;
            const kKey = `${sItem.kind}\0${sItem.key}`;
            if (sItem.deleted) delete journal.known[kKey];
            else journal.known[kKey] = { hash: quickHash(sItem.value), rev: pushed.rev, updated_at: sItem.updated_at };
          }
          this.saveJournal(accountId, journal);
        }
      }

      this.saveJournal(accountId, journal);
      save(`account_lib.${accountId}`, this.host.getLibrary());
      this.lastSyncedAt = nowSec();
      this.storage = await this.runAuthed((t) => server.accountStorage(t)).catch(() => this.storage);
    } catch (e) {
      this.syncError = String((e as Error)?.message ?? e);
    } finally {
      this.syncing = false;
      if (this.resyncPending && this.session) {
        this.resyncPending = false;
        this.scheduleSync(1500);
      }
    }
  }

  private applyRemoteItems(items: SyncItem[], journal: SyncJournal, pullRev: number) {
    const lib = this.host.getLibrary();
    const offsets = this.host.getLyricsOffsets();
    const devId = installId();
    let libChanged = false;
    let offsetsChanged = false;
    const addedLikedKeys = new Set<string>();

    const recordKnown = (it: SyncItem, hashSource: unknown) => {
      const kKey = `${it.kind}\0${it.key}`;
      if (it.deleted || it.value == null) {
        delete journal.known[kKey];
      } else {
        journal.known[kKey] = {
          hash: quickHash(hashSource),
          rev: it.rev ?? pullRev,
          updated_at: it.updated_at,
        };
      }
    };

    for (const it of items) {
      let hashSource: unknown = it.value;

      switch (it.kind) {
        case SYNC_KIND.LIKED_CLEAR: {
          if (it.deleted) continue;
          const at = Number(it.value?.at) || it.updated_at || 0;
          const curCleared = lib.liked_cleared_at ?? 0;
          if (at > curCleared) {
            lib.liked_cleared_at = at;
            lib.liked = [];
            const likedPrefix = `${SYNC_KIND.LIKED}\0`;
            for (const k of Object.keys(journal.known)) {
              if (k.startsWith(likedPrefix)) delete journal.known[k];
            }
            addedLikedKeys.clear();
            hashSource = { at };
            libChanged = true;
          } else if (at === curCleared) {
            if (addedLikedKeys.size > 0) {
              lib.liked = lib.liked.filter((t) => !addedLikedKeys.has(trackSyncId(t)) && !addedLikedKeys.has(trackKey(t)));
              for (const k of addedLikedKeys) delete journal.known[`${SYNC_KIND.LIKED}\0${k}`];
              addedLikedKeys.clear();
              libChanged = true;
            }
            hashSource = { at };
          } else {
            continue;
          }
          break;
        }
        case SYNC_KIND.LIKED: {
          const clearedAt = lib.liked_cleared_at ?? 0;
          if (!it.deleted && clearedAt > 0 && it.updated_at <= clearedAt) {
            delete journal.known[`${SYNC_KIND.LIKED}\0${it.key}`];
            continue;
          }
          // A known song keeps its place (like the apps); a new one goes on top.
          const pos = lib.liked.findIndex((t) => trackSyncId(t) === it.key || trackKey(t) === it.key);
          lib.liked = lib.liked.filter((t) => trackSyncId(t) !== it.key && trackKey(t) !== it.key);
          if (!it.deleted && it.value?.id && it.value?.title) {
            const tr = canonicalTrack(it.value as Track);
            if (pos >= 0) lib.liked.splice(pos, 0, tr);
            else lib.liked.unshift(tr);
            hashSource = tr;
            addedLikedKeys.add(it.key);
          } else {
            addedLikedKeys.delete(it.key);
          }
          libChanged = true;
          break;
        }
        case SYNC_KIND.PLAYLIST: {
          // Update in place so a changed playlist keeps its spot in the library.
          const at = lib.playlists.findIndex((p) => p.id === it.key);
          lib.playlists = lib.playlists.filter((p) => p.id !== it.key);
          if (!it.deleted && it.value?.name) {
            const pl: Playlist = {
              id: it.key,
              name: String(it.value.name),
              description: it.value.description ?? "",
              tracks: Array.isArray(it.value.tracks) ? it.value.tracks.map(canonicalTrack) : [],
              created_at: Number(it.value.created_at) || 0,
              local_only: false,
            };
            if (at >= 0) lib.playlists.splice(at, 0, pl);
            else lib.playlists.push(pl);
            hashSource = playlistSyncValue(pl);
          }
          libChanged = true;
          break;
        }
        case SYNC_KIND.ALBUM: {
          lib.saved_albums = lib.saved_albums.filter(
            (a) => !(a.kind === "album" && (`${norm(a.title)}|${norm(a.artist)}` === it.key || `${norm(a.artist)}|${norm(a.title)}` === it.key)),
          );
          if (!it.deleted && it.value?.title && it.value?.artist) {
            const title = String(it.value.title);
            const artist = String(it.value.artist);
            lib.saved_albums.push({
              kind: "album",
              title,
              artist,
              tracks: [],
            });
            hashSource = { title, artist };
          }
          libChanged = true;
          break;
        }
        case SYNC_KIND.FOLLOWED: {
          const name = typeof it.value === "string" ? it.value : it.value?.name;
          lib.followed = lib.followed.filter((a) => norm(a) !== it.key);
          if (!it.deleted && typeof name === "string" && name.trim()) {
            const clean = name.trim();
            lib.followed.push(clean);
            hashSource = { name: clean };
          }
          libChanged = true;
          break;
        }
        case SYNC_KIND.RECENT: {
          const trRaw: Track | null = it.value?.track ?? (it.value?.id ? it.value : null);
          lib.recent = lib.recent.filter(
            (r) => !(r.kind === "song" && (`song:${trackSyncId(r.track)}` === it.key || trackSyncId(r.track) === it.key || trackKey(r.track) === it.key)),
          );
          if (!it.deleted && trRaw?.id && trRaw?.title) {
            const tr = canonicalTrack(trRaw);
            const playedAt = Number(it.value?.played_at) || 0;
            lib.recent.unshift({ kind: "song", track: tr, at: playedAt || it.updated_at * 1000 });
            lib.recent = lib.recent.slice(0, 100);
            hashSource = { track: tr, played_at: playedAt || it.updated_at * 1000 };
          }
          libChanged = true;
          break;
        }
        case SYNC_KIND.PINNED: {
          lib.pinned = lib.pinned.filter((l) => listSyncKey(l) !== it.key);
          if (!it.deleted && it.value?.kind) {
            lib.pinned.push(it.value as ListKey);
          }
          libChanged = true;
          break;
        }
        case SYNC_KIND.RECENT_LIST: {
          lib.recent_lists = lib.recent_lists.filter((l) => listSyncKey(l) !== it.key);
          if (!it.deleted && it.value?.kind) {
            lib.recent_lists.unshift(it.value as ListKey);
            lib.recent_lists = lib.recent_lists.slice(0, 20);
          }
          libChanged = true;
          break;
        }
        case SYNC_KIND.SEARCH: {
          lib.search_history = lib.search_history.filter((s) => searchSyncKey(s) !== it.key && legacySearchSyncKey(s) !== it.key);
          if (!it.deleted && it.value?.kind) {
            lib.search_history.unshift(it.value as SearchItem);
            lib.search_history = lib.search_history.slice(0, 20);
          }
          libChanged = true;
          break;
        }
        case SYNC_KIND.HIDDEN_SIDEBAR: {
          lib.hidden_sidebar = lib.hidden_sidebar.filter((id) => id !== it.key);
          if (!it.deleted) lib.hidden_sidebar.push(it.key);
          libChanged = true;
          break;
        }
        case SYNC_KIND.HIDDEN_ARTIST: {
          const ha = typeof it.value === "string" ? it.value : it.value?.name;
          lib.hidden_artists = (lib.hidden_artists ?? []).filter((a) => norm(a) !== it.key);
          if (!it.deleted && typeof ha === "string" && ha.trim()) {
            const clean = ha.trim();
            lib.hidden_artists.push(clean);
            hashSource = clean;
          }
          libChanged = true;
          break;
        }
        case SYNC_KIND.LYRICS_OFFSET: {
          if (it.deleted || typeof it.value !== "number") delete offsets[it.key];
          else offsets[it.key] = it.value;
          offsetsChanged = true;
          break;
        }
        case SYNC_KIND.STATS: {
          this.host.stats.applyRemote(devId, it.key, it.deleted ? null : it.value);
          break;
        }
      }

      recordKnown(it, hashSource);
    }

    if (libChanged) this.host.setLibrary(lib);
    if (offsetsChanged) this.host.setLyricsOffsets(offsets);
  }

  private collectLocalDiff(journal: SyncJournal): SyncItem[] {
    const lib = this.host.getLibrary();
    const cfg = this.host.getConfig();
    const offsets = this.host.getLyricsOffsets();
    const devId = installId();
    const now = Date.now();
    const current = new Map<string, { kind: string; key: string; value: any }>();

    const put = (kind: string, key: string, value: any) => {
      if (!key) return;
      current.set(`${kind}\0${key}`, { kind, key, value });
    };

    const clearedAt = lib.liked_cleared_at ?? 0;
    const clearKey = `${SYNC_KIND.LIKED_CLEAR}\0all`;
    if (clearedAt > 0) {
      put(SYNC_KIND.LIKED_CLEAR, "all", { at: clearedAt });
    }
    const likedCleared = clearedAt > 0 && (!journal.known[clearKey] || journal.known[clearKey].hash !== quickHash({ at: clearedAt }));

    // Oldest first, like the apps: other devices put each new liked song on
    // top in arrival order, so newest-first would flip the list there.
    for (const t of [...lib.liked].reverse()) {
      const k = pickJournalKey(journal, SYNC_KIND.LIKED, trackSyncId(t), trackKey(t));
      put(SYNC_KIND.LIKED, k, canonicalTrack(t));
    }
    for (const p of lib.playlists) {
      if (p.local_only) continue;
      put(SYNC_KIND.PLAYLIST, p.id, playlistSyncValue(p));
    }
    for (const a of lib.saved_albums) {
      if (a.kind !== "album") continue;
      const k = pickJournalKey(journal, SYNC_KIND.ALBUM, `${norm(a.title)}|${norm(a.artist)}`, `${norm(a.artist)}|${norm(a.title)}`);
      put(SYNC_KIND.ALBUM, k, {
        title: a.title,
        artist: a.artist,
      });
    }
    for (const f of lib.followed) put(SYNC_KIND.FOLLOWED, norm(f), { name: f });
    for (const r of lib.recent.slice(0, 100)) {
      if (r.kind === "song") {
        const k = pickJournalKey(journal, SYNC_KIND.RECENT, `song:${trackSyncId(r.track)}`, trackKey(r.track));
        put(SYNC_KIND.RECENT, k, {
          track: canonicalTrack(r.track),
          played_at: r.at ?? 0,
        });
      }
    }
    for (const p of lib.pinned) put(SYNC_KIND.PINNED, listSyncKey(p), p);
    for (const rl of lib.recent_lists) put(SYNC_KIND.RECENT_LIST, listSyncKey(rl), rl);
    for (const s of lib.search_history) {
      const k = pickJournalKey(journal, SYNC_KIND.SEARCH, searchSyncKey(s), legacySearchSyncKey(s));
      put(SYNC_KIND.SEARCH, k, s);
    }
    for (const h of lib.hidden_sidebar) put(SYNC_KIND.HIDDEN_SIDEBAR, h, h);
    for (const ha of lib.hidden_artists ?? []) put(SYNC_KIND.HIDDEN_ARTIST, norm(ha), ha);
    for (const [tk, off] of Object.entries(offsets)) put(SYNC_KIND.LYRICS_OFFSET, tk, off);

    if (cfg.stats.enabled) {
      for (const [sKey, mStats] of this.host.stats.syncItems(devId)) {
        put(SYNC_KIND.STATS, sKey, mStats);
      }
    }

    const diff: SyncItem[] = [];
    for (const [kKey, item] of current.entries()) {
      const h = quickHash(item.value);
      const prev = journal.known[kKey];
      const forceLiked = likedCleared && item.kind === SYNC_KIND.LIKED;
      if (forceLiked || !prev || prev.hash !== h) {
        const updated_at = item.kind === SYNC_KIND.LIKED_CLEAR
          ? clearedAt
          : item.kind === SYNC_KIND.LIKED && clearedAt > 0
            ? Math.max(now, clearedAt + 1)
            : now;
        diff.push({
          kind: item.kind,
          key: item.key,
          value: item.value,
          updated_at,
          deleted: false,
          rev: prev?.rev ?? 0,
        });
      }
    }

    for (const [kKey, prev] of Object.entries(journal.known)) {
      const sep = kKey.indexOf("\0");
      if (sep < 0) continue;
      const kind = kKey.slice(0, sep);
      const key = kKey.slice(sep + 1);
      if (kind === SYNC_KIND.KEYRING || kind === SYNC_KIND.E2E || kind === SYNC_KIND.E2E_PLAYLIST || kind === SYNC_KIND.SETTINGS || kind === SYNC_KIND.TASTE || kind === SYNC_KIND.FEED_NEW_SEEN) {
        continue;
      }
      if (likedCleared && kind === SYNC_KIND.LIKED) continue;
      // For stats, only delete our own device's stats when stats are disabled
      if (kind === SYNC_KIND.STATS && !key.startsWith(`${devId}:`)) continue;
      if (!current.has(kKey)) {
        diff.push({
          kind,
          key,
          value: undefined,
          updated_at: now,
          deleted: true,
          rev: prev.rev,
        });
      }
    }

    return diff;
  }

  // ---- Handlers for WebBackend ----

  handlers(): Record<string, (a: Record<string, any>) => unknown> {
    return {
      account_status: async () => {
        if (!this.session) {
          return {
            signed_in: false,
            available: this.accountsAvailable,
            syncing: false,
          };
        }
        this.storage = await this.runAuthed((t) => server.accountStorage(t)).catch(() => this.storage);
        return {
          signed_in: true,
          available: true,
          handle: this.session.account.handle,
          account_id: this.session.account.id,
          device_id: this.session.device_id,
          device_limit: this.session.account.device_limit,
          created_at: this.session.account.created_at,
          has_password: this.session.account.has_password,
          keys_locked: this.keysLocked,
          syncing: this.syncing,
          last_synced_at: this.lastSyncedAt,
          sync_error: this.syncError,
          devices: this.devices,
          storage: this.storage,
        };
      },

      account_login_start: async (a) => {
        const res = await server.loginStart(detectDeviceName(), detectPlatform());
        const url = res.browser_url.startsWith("http")
          ? res.browser_url
          : `${serverOrigin()}${res.browser_url.startsWith("/") ? "" : "/"}${res.browser_url}`;
        if (a.open_browser) {
          window.open(url, "_blank", "noopener,noreferrer");
        }
        return {
          login_id: res.login_id,
          code: res.code,
          browser_url: url,
          expires_in_s: res.expires_in_s,
        };
      },

      account_login_poll: async (a) => {
        const res = await server.loginPoll(String(a.login_id));
        if (res.status === "done") {
          await this.finishSignIn(res.session);
          return { state: "done", handle: res.session.account.handle };
        }
        return { state: res.status };
      },

      account_password_submit: async (a) => {
        const handle = String(a.handle ?? "").trim().replace(/^@/, "");
        const password = String(a.password ?? "");
        const create = Boolean(a.create);
        const device_name = detectDeviceName();
        const platform = detectPlatform();

        if (create) {
          const salt = b64url(randomBytes(16));
          const key = await deriveLoginKey(password, salt);
          const reg = await server.passwordRegister({ handle, salt, key, device_name, platform });
          const session = "session" in reg ? reg.session : reg;
          await this.finishSignIn(session, password);
          return "recovery_code" in reg ? reg.recovery_code : undefined;
        }

        const saltRes = await server.passwordSalt(handle);
        const key = await deriveLoginKey(password, saltRes.salt);
        const session = await server.passwordLogin({ handle, key, device_name, platform });
        if (await suspiciousSignInSalt(saltRes.salt, session.account.id)) {
          throw new Error("suspicious_salt");
        }
        await this.finishSignIn(session, password);
      },

      account_unlock_keys: async (a) => {
        if (!this.session) throw new Error("signed_out");
        const password = String(a.password ?? "");
        const accountId = this.session.account.id;
        const journal = this.getJournal(accountId);
        const main = journal.keyring.main;
        const pwKey = await derivePasswordKey(password, accountId);
        if (main?.pw && main?.kid) {
          const opened = await unwrapWithPassword(pwKey, accountId, main.kid, main.pw);
          if (!opened) throw new Error("invalid_credentials");
          this.saveDataKey(accountId, opened, pwKey);
        } else {
          const { store } = this.getKeyStore(accountId);
          store.pw_key = b64url(pwKey);
          save(`keys.${accountId}`, store);
          this.keysLocked = false;
        }
        await this.syncNow();
      },

      account_change_password: async (a) => {
        if (!this.session) throw new Error("signed_out");
        const token = await this.validToken();
        const currentPassword = String(a.current_password ?? "");
        const newPassword = String(a.new_password ?? "");
        const handle = this.session.account.handle;
        const accountId = this.session.account.id;

        if (this.session.account.has_password && currentPassword) {
          const saltRes = await server.passwordSalt(handle);
          const currentKey = await deriveLoginKey(currentPassword, saltRes.salt);
          await server.passwordCheck(token, currentKey);
        }

        const newSalt = b64url(randomBytes(16));
        const newLoginKey = await deriveLoginKey(newPassword, newSalt);
        const updated = await server.passwordSet(token, { salt: newSalt, key: newLoginKey });
        this.session.account.has_password = Boolean(updated.has_password ?? true);
        save("account", this.session);

        // Re-wrap E2E DataKey with new password
        const newPwKey = await derivePasswordKey(newPassword, accountId);
        const { dataKey } = this.getKeyStore(accountId);
        const k = dataKey ?? DataKey.generate();
        this.saveDataKey(accountId, k, newPwKey);
        const journal = this.getJournal(accountId);
        const kid = await k.kid();
        const pwBox = await wrapWithPassword(newPwKey, accountId, k);
        const mainVal = { v: 1, kid, pw: pwBox };
        journal.keyring.main = mainVal;
        await server.syncPush(token, [
          {
            kind: SYNC_KIND.KEYRING,
            key: "main",
            value: mainVal,
            updated_at: Date.now(),
            deleted: false,
            rev: journal.known[`${SYNC_KIND.KEYRING}\0main`]?.rev ?? 0,
          },
        ]);
        this.saveJournal(accountId, journal);
      },

      account_link_passkey: async () => {
        const token = await this.validToken();
        const res = await server.passkeyLink(token);
        const url = res.url.startsWith("http")
          ? res.url
          : `${serverOrigin()}${res.url.startsWith("/") ? "" : "/"}${res.url}`;
        return { url };
      },

      account_rename: async (a) => {
        if (!this.session) throw new Error("signed_out");
        const token = await this.validToken();
        const handle = String(a.handle ?? "").trim().replace(/^@/, "");
        const info = await server.accountPatch(token, handle);
        this.session.account.handle = info.handle;
        save("account", this.session);
        this.host.emit("account", this.uiAccount());
      },

      account_approve: async (a) => {
        const token = await this.validToken();
        const parsed = splitPairCode(String(a.code ?? ""));
        const serverCode = parsed ? parsed[0] : String(a.code ?? "").trim().toUpperCase();
        await server.loginApprove(token, serverCode);
        await this.refreshAccountAndSync();
      },

      account_recovery: async () => {
        const token = await this.validToken();
        const res = await server.recoveryCode(token);
        return res.recovery_code;
      },

      admin_call: async (a) => {
        if (!this.session) throw new Error("signed_out");
        const path = String(a.path ?? "").replace(/^\/+/, "");
        const method = String(a.method ?? "GET").toUpperCase();
        if (!adminPathOk(path)) throw new Error("bad_path");
        if (method !== "GET" && method !== "POST" && method !== "PUT") throw new Error("bad_method");
        try {
          return await this.runAuthed((t) => server.admin(t, method as "GET" | "POST" | "PUT", path, a.body ?? undefined));
        } catch (e) {
          if (e instanceof ApiError) throw new Error(e.status === 400 ? e.message : e.code);
          throw e;
        }
      },

      account_remove_device: async (a) => {
        if (!this.session) throw new Error("signed_out");
        const token = await this.validToken();
        await server.deviceRemove(token, String(a.id));
        const devs = await server.devicesList(token).catch(() => null);
        if (devs) {
          this.devices = devs.devices;
          save(`devices.${this.session.account.id}`, this.devices);
        }
      },

      account_sign_out: async () => {
        if (this.session) {
          try {
            const token = await this.validToken();
            await server.deviceSignOut(token);
          } catch { /* ignore network error on sign out */ }
        }
        this.clearSessionAndRestoreGuest();
      },

      account_sign_out_all: async () => {
        if (!this.session) throw new Error("signed_out");
        const token = await this.validToken();
        try {
          await server.deviceSignOutAll(token);
        } catch (e: any) {
          if (e?.status !== 404 && e?.status !== 405) throw e;
          const devs = await server.devicesList(token);
          for (const d of devs.devices) {
            if (!d.current) {
              try {
                await server.deviceRemove(token, d.id);
              } catch (rmErr: any) {
                if (rmErr?.status !== 404) throw rmErr;
              }
            }
          }
          await server.deviceSignOut(token);
        }
        this.clearSessionAndRestoreGuest();
      },

      account_delete: async () => {
        if (!this.session) throw new Error("signed_out");
        const token = await this.validToken();
        const prevId = this.session.account.id;
        await server.accountDelete(token);
        remove(`account_lib.${prevId}`);
        remove(`playback.${prevId}`);
        remove(`sync.${prevId}`);
        remove(`keys.${prevId}`);
        remove(`devices.${prevId}`);
        this.clearSessionAndRestoreGuest(true);
      },

      account_sync_now: async () => {
        await this.refreshAccountAndSync();
      },

      account_merge: async (a) => {
        const backup = load<GuestSnapshot>("guest_backup");
        remove("guest_backup");
        if (!a.merge || !backup) return;
        const lib = this.host.getLibrary();
        const haveLiked = new Set(lib.liked.map(trackKey));
        for (const t of backup.library.liked) {
          if (!haveLiked.has(trackKey(t))) lib.liked.push(t);
        }
        const havePl = new Set(lib.playlists.map((p) => p.id));
        for (const p of backup.library.playlists) {
          if (!havePl.has(p.id)) lib.playlists.push({ ...p, local_only: false });
        }
        for (const al of backup.library.saved_albums) {
          if (!lib.saved_albums.some((x) => sameList(x, al))) lib.saved_albums.push(al);
        }
        for (const f of backup.library.followed) {
          if (!lib.followed.some((x) => norm(x) === norm(f))) lib.followed.push(f);
        }
        this.host.setLibrary(lib);
        if (backup.stats && Object.keys(backup.stats).length > 0) {
          this.host.stats.mergeGuestMonths(backup.stats);
        }
        await this.syncNow();
      },

      // ---- Profile ----
      profile: async (a) => {
        const handle = String(a.handle ?? "").trim().replace(/^@/, "");
        if (!handle) return null;
        if (!this.session) throw new Error("signed_out");
        const token = await this.validToken();
        let pub: PublicProfile;
        try {
          pub = await server.publicProfile(handle, token);
        } catch (e) {
          if (e instanceof ApiError && e.status === 404) return null;
          throw e;
        }
        const is_me = Boolean(pub.own || (this.session && norm(this.session.account.handle) === norm(pub.handle)));
        const follows = this.host.getLibrary().followed.some((f) => norm(f) === norm(`@${pub.handle}`) || norm(f) === norm(pub.handle));
        const playlists: Playlist[] = (pub.playlists ?? []).map((pl) => ({
          id: pl.id,
          name: pl.name,
          cover: pl.cover ?? pl.preview?.[0]?.cover_url ?? null,
          tracks: pl.preview?.length
            ? pl.preview
            : Array.from({ length: pl.songs || 0 }, (_, i) => ({
                id: `${pl.id}-${i}`,
                source: "deezer" as const,
                title: pl.name,
                artist: `@${pub.handle}`,
              })),
        }));
        const out: Profile & { private?: boolean; recent?: Track[] } = {
          handle: pub.handle,
          name: pub.handle,
          bio: "",
          avatar: pub.avatar ? server.profileImageUrl(pub.handle, pub.avatar) : null,
          banner: pub.banner ? server.profileImageUrl(pub.handle, pub.banner) : null,
          followers: 0,
          following: 0,
          follows,
          is_me,
          top_artists: [],
          top_tracks: (pub.recent ?? []).slice(0, 5),
          playlists,
          recent: pub.recent ?? [],
          sections: normalizeSections(pub.sections?.length ? pub.sections : is_me ? this.host.getConfig().sharing.profile_sections : undefined),
          private: Boolean(pub.private),
        };
        return out;
      },

      profile_playlist: async (a) => {
        const handle = String(a.handle ?? "").trim().replace(/^@/, "");
        const id = String(a.id ?? "");
        const token = this.session ? await this.validToken().catch(() => undefined) : undefined;
        const full = await server.publicProfilePlaylist(handle, id, token);
        return full.tracks ?? [];
      },

      profile_image: async (a) => {
        const handle = String(a.handle ?? "").trim().replace(/^@/, "");
        const sha = String(a.sha ?? "").trim();
        if (!handle || !/^[0-9a-fA-F]{64}$/.test(sha)) return null;
        return server.profileImageUrl(handle, sha);
      },

      profile_follow: (a) => {
        const h = `@${String(a.handle ?? "").replace(/^@/, "")}`;
        const lib = this.host.getLibrary();
        lib.followed = a.on ? [...lib.followed.filter((x) => norm(x) !== norm(h)), h] : lib.followed.filter((x) => norm(x) !== norm(h));
        this.host.setLibrary(lib);
        this.scheduleSync();
      },

      profile_settings: async () => {
        const token = await this.validToken();
        const handle = this.session!.account.handle;
        const w = await server.profileSettingsGet(token);
        return wireToProfileSettings(handle, w, this.host.getConfig().sharing.profile_sections);
      },

      profile_update: async (a) => {
        const token = await this.validToken();
        const handle = this.session!.account.handle;
        const patch: { public?: boolean; show_recent?: boolean; playlists?: string[]; sections?: string[] } = {};
        if (typeof a.public === "boolean") patch.public = a.public;
        if (typeof a.show_recent === "boolean") patch.show_recent = a.show_recent;
        if (Array.isArray(a.playlists)) patch.playlists = a.playlists;
        if (Array.isArray(a.sections)) {
          const sec = normalizeSections(a.sections as string[]);
          patch.sections = sec;
          const cfg = this.host.getConfig();
          cfg.sharing.profile_sections = sec;
          this.host.setConfig(cfg);
          this.scheduleSync();
        }
        const w = await server.profileSettingsPatch(token, patch);
        return wireToProfileSettings(handle, w, this.host.getConfig().sharing.profile_sections);
      },

      profile_reorder_playlist: async (a) => {
        const token = await this.validToken();
        const cur = await server.profileSettingsGet(token);
        const shown = new Set(cur.playlists ?? []);
        const order = Array.isArray(a.order) ? (a.order as string[]).filter((id) => shown.has(id)) : cur.playlists ?? [];
        await server.profileSettingsPatch(token, { playlists: order });
      },

      profile_toggle_playlist: async (a) => {
        const token = await this.validToken();
        const id = String(a.id ?? "");
        const lib = this.host.getLibrary();
        const pl = lib.playlists.find((p) => p.id === id);
        if (pl && pl.local_only) {
          pl.local_only = false;
          this.host.setLibrary(lib);
          await this.syncNow();
        }
        const cur = await server.profileSettingsGet(token);
        const list = [...(cur.playlists ?? [])];
        const idx = list.indexOf(id);
        if (idx >= 0) list.splice(idx, 1);
        else list.push(id);
        await server.profileSettingsPatch(token, { playlists: list });
      },

      profile_pick_image: async (a) => {
        const token = await this.validToken();
        const banner = Boolean(a.banner);
        let mime = "image/webp";
        const bytes = await new Promise<Uint8Array | null>((resolve, reject) => {
          const input = document.createElement("input");
          input.type = "file";
          input.accept = "image/jpeg,image/png,image/webp";
          input.onchange = () => {
            const file = input.files?.[0];
            if (!file) { resolve(null); return; }
            const url = URL.createObjectURL(file);
            const im = new Image();
            im.onload = () => {
              try {
                const [ow, oh] = banner ? [1500, 500] : [512, 512];
                const aspect = ow / oh;
                const w = im.naturalWidth, h = im.naturalHeight;
                const [cw, ch] = w >= h * aspect ? [h * aspect, h] : [w, w / aspect];
                const sx = Math.floor((w - cw) / 2);
                const sy = Math.floor((h - ch) / 2);
                const [cv, g] = pictureCanvas(ow, oh);
                g.drawImage(im, sx, sy, cw, ch, 0, 0, ow, oh);
                const dataUrl = encodePicture(cv, 0.86);
                mime = dataUrl.slice(5, dataUrl.indexOf(";"));
                const b64 = dataUrl.slice(dataUrl.indexOf(",") + 1);
                const bin = atob(b64);
                const out = new Uint8Array(bin.length);
                for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
                resolve(out);
              } catch (err) {
                reject(err);
              } finally {
                URL.revokeObjectURL(url);
              }
            };
            im.onerror = () => { URL.revokeObjectURL(url); reject(new Error("image")); };
            im.src = url;
          };
          input.oncancel = () => resolve(null);
          input.click();
        });
        if (!bytes) return;
        await server.profilePutImage(token, banner ? "banner" : "avatar", mime, bytes);
      },

      profile_set_image: async (a) => {
        const token = await this.validToken();
        const kind = a.banner ? "banner" : "avatar";
        if (!a.image) {
          await server.profileDeleteImage(token, kind);
          return;
        }
        const dataUrl = String(a.image);
        const comma = dataUrl.indexOf(",");
        const meta = comma >= 0 ? dataUrl.slice(0, comma) : "";
        const b64 = comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl;
        const mime = /data:([^;]+);/.exec(meta)?.[1] ?? "image/jpeg";
        const bin = atob(b64);
        const bytes = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
        await server.profilePutImage(token, kind, mime, bytes);
      },

      profile_remove_image: async (a) => {
        const token = await this.validToken();
        await server.profileDeleteImage(token, a.banner ? "banner" : "avatar");
      },

      // ---- Privacy report ----
      privacy_report: () => {
        const cfg = this.host.getConfig();
        return {
          mode: "Server (/web)",
          account: this.session ? `@${this.session.account.handle}` : null,
          install_id: installId(),
          direct_domains: ["api.audius.co", "api.jamendo.com", "archive.org", "www.youtube.com"],
          web_logins: this.webLogins,
          discord_on: false,
          last_synced_at: this.lastSyncedAt,
          data_dir: "localStorage (glyph.web.*)",
          files: [],
          total_bytes: 0,
          keys_in_keychain: 0,
          keys_in_config: Object.keys(cfg.keys ?? {}).length,
        };
      },

      privacy_clear: (a) => {
        const what = String(a.what ?? "");
        const lib = this.host.getLibrary();
        if (what === "search_history") {
          lib.search_history = [];
          this.host.setLibrary(lib);
        } else if (what === "recent") {
          lib.recent = [];
          lib.recent_lists = [];
          this.host.setLibrary(lib);
        } else if (what === "web_logins") {
          this.webLogins = [];
          remove("web_logins");
        }
      },

      privacy_record_web_login: (a) => {
        const domain = String(a.domain ?? "");
        if (!domain) return;
        this.webLogins = [{ domain, at: nowSec() }, ...this.webLogins.filter((x) => x.domain !== domain)].slice(0, 20);
        save("web_logins", this.webLogins);
      },
    };
  }
}
