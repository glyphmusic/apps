// The server dashboard talks to the server's admin API (`/admin/v1/*`) with
// the app sign-in. Only accounts the server owner made admins get in; keys,
// updates, logs and handing out the role stay with the owner's password.
// Same list as `admin_path_ok` in crates/glyph-app/src/account.rs.

const ROOTS = ["me", "stats", "history", "status", "limits", "accounts", "account-defaults", "bans", "blocks", "jams", "cache/stats", "cache/clear", "companion/status", "app-releases"];

/** Admin API paths the dashboard may call. */
export function adminPathOk(path: string): boolean {
  const root = path.split("?")[0];
  return (
    path.length <= 200 &&
    /^[A-Za-z0-9_\-./?=&@%]*$/.test(path) &&
    !path.includes("..") &&
    ROOTS.some((r) => root === r || root.startsWith(`${r}/`))
  );
}

export interface AdminMe { owner: boolean; handle?: string | null; keys: boolean; updates: boolean; roles: boolean }

export interface StorageUsage {
  audio_bytes?: number | null; audio_files?: number | null; audio_cap_bytes: number;
  db_bytes?: number | null; db_cap_bytes: number; sync_bytes?: number | null;
}

export interface ServerStats {
  uptime_s?: number | null;
  storage?: StorageUsage | null;
  accounts?: { accounts: number; devices: number; active_1d: number; active_7d: number; active_30d: number } | null;
  max_accounts: number;
  bans: number;
  cache?: { entries: number; hits: number; misses: number; disk_entries?: number | null } | null;
  companion_enabled: boolean;
  youtube_per_day: number;
}

export interface DayStats { day: string; accounts: number; devices: number; active_1d: number; audio_bytes: number; db_bytes: number; sync_bytes: number }

export interface AdminAccountRow {
  id: string; handle: string; created_at: number; device_limit: number; device_limit_set?: number | null;
  devices: number; last_seen?: number | null; sync_bytes: number; banned: boolean; admin: boolean;
  avatar?: string | null; banner?: string | null;
}

export interface AdminDevice { id: string; name: string; platform: string; last_seen: number; app_version?: string | null }

export interface AdminAccount {
  id: string; handle: string; created_at: number; device_limit: number; device_limit_set?: number | null;
  devices: AdminDevice[]; used_bytes: number; banned: boolean; admin: boolean;
  avatar?: string | null; banner?: string | null;
}

export interface Limits {
  require_account: boolean; rate_limit_per_min: number; youtube_daily_budget_units: number; import_max_pages: number;
  playlist_max_tracks: number; companion_enabled: boolean; mcp_relay: boolean; audio_cache_days: number;
  audio_cache_max_gb: number; d1_max_mb: number; account_max_mb: number; max_accounts: number;
  youtube_per_day: number; youtube_require_account: boolean; min_app_version?: string | null;
  stable_app_version?: string | null; latest_app_version?: string | null;
}

export interface AppRelease { version: string; prerelease: boolean; published_at?: string | null }

export interface Ban {
  id: string; kind: string; value: string; label?: string | null; reason?: string | null;
  created_at: number; expires_at?: number | null; origin?: string | null;
}

export type BlockReason = "abuse_block" | "rate_limited" | "youtube_limit" | "login_lockout";

export interface ActiveBlock {
  id: string;
  reason: BlockReason;
  kind: string;
  label: string;
  handle?: string | null;
  user_id?: string | null;
  scope?: string | null;
  created_at: number;
  expires_at: number;
  hits: number;
  clearable: boolean;
}

export interface AdminJam {
  code: string;
  host: string;
  members: string[];
  member_count: number;
  host_online: boolean;
  created_at: number;
  last_seen: number;
  expires_at: number;
  track?: string | null;
  playing: boolean;
  queue_len: number;
}

/** One part of a ring chart. */
export interface Slice { label: string; value: number; color: string }

/** Series colours after the app's accent (dark steps of the checked default
 *  palette, fixed order); the rest folds into "others" in grey. */
export const SERIES = ["var(--accent)", "#3987e5", "#d95926", "#199e70", "#c98500", "#9085e9"];
export const OTHER = "rgba(255,255,255,.28)";

/** Bytes as "12.3 MB" (1000-based, like the server's limits). */
export function fmtBytes(n: number | null | undefined): string {
  const b = Math.max(0, n ?? 0);
  if (b < 1000) return `${b} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = b / 1000, i = 0;
  while (v >= 1000 && i < units.length - 1) { v /= 1000; i++; }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}
