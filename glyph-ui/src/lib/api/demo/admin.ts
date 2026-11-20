// Demo handler for the server dashboard (`npm run dev`, open with `?admin` to
// be signed in as a demo admin). Everything here is made up.
//
//   admin_call { method: "GET"|"POST"|"PUT", path, body? } -> the admin API's JSON
//     (`/admin/v1/<path>`); errors: not_admin, owner_only, bad_request: <why>
import type { DemoHandler } from "../demo";

const DAY = 86_400_000;
const NAMES = ["mira", "jonas_beats", "lena", "theo", "kaito", "sam", "noor", "felix", "ava", "rio", "juno", "pia"];
const seed = (i: number) => ((i * 9301 + 49297) % 233280) / 233280;

const demoSha = (tag: string, i: number) => `${tag}${i.toString(16).padStart(62, "0")}`.slice(0, 64);
const DEMO_COLORS = ["#f97316", "#3987e5", "#199e70", "#9085e9", "#d95926", "#c98500"];

const accounts = NAMES.map((handle, i) => ({
  id: `u_${i}`,
  handle,
  created_at: Date.now() - (40 - i * 3) * DAY,
  device_limit: 5,
  device_limit_set: null as number | null,
  devices: 1 + Math.floor(seed(i) * 3),
  last_seen: Date.now() - Math.floor(seed(i + 3) * 6 * DAY),
  sync_bytes: Math.floor(seed(i + 7) ** 2 * 38_000_000) + 200_000,
  banned: i === 9,
  admin: i === 0,
  avatar: i % 2 === 0 ? demoSha("aa", i) : null as string | null,
  banner: i % 3 === 0 ? demoSha("bb", i) : null as string | null,
}));

const days = Array.from({ length: 29 }, (_, i) => {
  const d = new Date(Date.now() - (29 - i) * DAY).toISOString().slice(0, 10);
  const n = 4 + Math.round(i * 0.3);
  return {
    day: d, accounts: n, devices: n * 2, active_1d: Math.max(1, Math.round(n * (0.4 + seed(i) * 0.4))),
    audio_bytes: 1_400_000_000 + i * 62_000_000, db_bytes: 210_000_000 + i * 3_500_000, sync_bytes: 30_000_000 + i * 1_900_000,
  };
});

let limits = {
  require_account: false, rate_limit_per_min: 120, youtube_daily_budget_units: 10_000, import_max_pages: 5,
  playlist_max_tracks: 5000, companion_enabled: true, mcp_relay: true, audio_cache_days: 15, audio_cache_max_gb: 10,
  d1_max_mb: 500, account_max_mb: 50, max_accounts: 25, youtube_per_day: 0, youtube_require_account: false,
  min_app_version: "0.12.0" as string | null, stable_app_version: "0.12.10" as string | null, latest_app_version: null as string | null,
};
const releases = ["0.12.12", "0.12.11", "0.12.10", "0.12.9", "0.12.8", "0.12.7", "0.12.6", "0.12.5", "0.12.4", "0.12.0"].map((version, i) => ({
  version, prerelease: version !== "0.12.10" && version !== "0.12.0", published_at: new Date(Date.now() - (i * 2 + 0.5) * DAY).toISOString(),
}));
let bans = [{ id: "b1", kind: "account", value: "u_9", label: "@rio", reason: null, created_at: Date.now() - 3 * DAY, expires_at: Date.now() + 4 * DAY }];
let blocks = [
  { id: "blk_1", reason: "abuse_block", kind: "account", label: "@theo (IP)", handle: "theo", user_id: "u_3", scope: "api", created_at: Date.now() - 3 * 60_000, expires_at: Date.now() + 12 * 60_000, hits: 14, clearable: true },
  { id: "blk_2", reason: "rate_limited", kind: "ip", label: "IP 4a8f12c9", handle: null as string | null, user_id: null as string | null, scope: "media", created_at: Date.now() - 15_000, expires_at: Date.now() + 45_000, hits: 3, clearable: false },
];

function account(handle: string) {
  const a = accounts.find((x) => x.handle === handle);
  if (!a) throw new Error("not_found");
  return {
    ...a,
    used_bytes: a.sync_bytes,
    devices: Array.from({ length: a.devices }, (_, i) => ({
      id: `d_${a.id}_${i}`,
      name: ["Pixel 8", "Desktop", "MacBook"][i] ?? "Phone",
      platform: ["android", "windows", "macos"][i] ?? "android",
      last_seen: a.last_seen ?? Date.now(),
      app_version: ["0.12.13", "0.12.12", "0.12.10"][i] ?? "0.12.13",
    })),
  };
}

const call: DemoHandler = (_ctx, a) => {
  const path = String(a.path ?? "");
  const [root, q] = path.split("?");
  const sum = accounts.reduce((n, x) => n + x.sync_bytes, 0);
  if (root === "me") return { owner: false, handle: "demo", keys: false, updates: false, roles: false };
  if (root === "stats") {
    return {
      uptime_s: null,
      storage: { audio_bytes: 3_210_000_000, audio_files: 1843, audio_cap_bytes: 10_000_000_000, db_bytes: 312_000_000, db_cap_bytes: 500_000_000, sync_bytes: sum },
      accounts: { accounts: accounts.length, devices: accounts.reduce((n, x) => n + x.devices, 0), active_1d: 7, active_7d: 10, active_30d: 12 },
      max_accounts: limits.max_accounts, bans: bans.length, cache: { entries: 15_204, hits: 81_220, misses: 19_410 }, companion_enabled: true, youtube_per_day: 0,
    };
  }
  if (root === "history") return { days };
  if (root === "accounts") {
    const needle = new URLSearchParams(q ?? "").get("q") ?? "";
    const list = accounts.filter((x) => x.handle.includes(needle));
    return { accounts: list, total: list.length, limit: 500, offset: 0 };
  }
  if (root === "limits") {
    if (a.method === "PUT") {
      limits = { ...limits, ...(a.body as object) };
      for (const k of ["min_app_version", "stable_app_version", "latest_app_version"] as const) if (!limits[k]) limits[k] = null;
    }
    return limits;
  }
  if (root === "app-releases") return { releases };
  if (root === "bans") {
    if (a.method === "POST") {
      const handle = String((a.body as { value?: string })?.value ?? "");
      const acc = accounts.find((x) => x.handle === handle);
      if (acc) { acc.banned = true; bans = [...bans, { id: `b${bans.length + 2}`, kind: "account", value: acc.id, label: `@${handle}`, reason: null, created_at: Date.now(), expires_at: null as unknown as number }]; }
    }
    return { bans, changed: [], dry_run: false };
  }
  if (root === "bans/remove") {
    const target = String((a.body as { target?: string })?.target ?? "");
    const acc = accounts.find((x) => x.handle === target || x.id === target);
    bans = bans.filter((b) => b.id !== target && b.label !== `@${target}` && b.value !== acc?.id);
    if (acc) acc.banned = false; else accounts.forEach((x) => { if (!bans.some((b) => b.value === x.id)) x.banned = false; });
    return { bans, changed: [], dry_run: false };
  }
  if (root === "blocks") return { blocks: blocks.filter((b) => b.expires_at > Date.now()) };
  if (root === "blocks/clear") {
    const id = String((a.body as { id?: string })?.id ?? "");
    blocks = blocks.filter((b) => b.id !== id);
    return { blocks: blocks.filter((b) => b.expires_at > Date.now()) };
  }
  if (root === "cache/clear") return { cleared: true };
  if (root.startsWith("accounts/")) {
    const rest = root.slice(9);
    const handle = rest.replace(/\/(profile\/reset|revoke|admin)$/, "");
    const acc = accounts.find((x) => x.handle === handle);
    if (acc && rest.endsWith("/profile/reset")) {
      const b = (a.body as { avatar?: boolean; banner?: boolean } | null) ?? {};
      if (!b.avatar && !b.banner) throw new Error("bad_request: set avatar and/or banner to true");
      if (b.avatar) acc.avatar = null;
      if (b.banner) acc.banner = null;
    } else if (acc && rest.endsWith("/revoke")) {
      const dev = (a.body as { device?: string } | undefined)?.device;
      acc.devices = dev ? Math.max(0, acc.devices - 1) : 0;
    }
    else if (acc && a.method === "PUT") { const n = (a.body as { device_limit?: number | null })?.device_limit; acc.device_limit_set = n ?? null; acc.device_limit = n ?? 5; }
    return account(handle);
  }
  throw new Error("not_found");
};

const demoImage: DemoHandler = (_ctx, a) => {
  const handle = String(a.handle ?? "").replace(/^@/, "");
  const sha = String(a.sha ?? "");
  const idx = Math.max(0, NAMES.indexOf(handle));
  const c1 = DEMO_COLORS[idx % DEMO_COLORS.length];
  const c2 = DEMO_COLORS[(idx + 2) % DEMO_COLORS.length];
  const svg = sha.startsWith("bb")
    ? `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 300 100"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${c1}"/><stop offset="1" stop-color="${c2}"/></linearGradient></defs><rect width="300" height="100" fill="url(#g)"/></svg>`
    : `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${c1}"/><stop offset="1" stop-color="${c2}"/></linearGradient></defs><rect width="64" height="64" fill="url(#g)"/><text x="32" y="41" text-anchor="middle" font-family="sans-serif" font-size="28" font-weight="700" fill="#fff">${(handle[0] ?? "?").toUpperCase()}</text></svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
};

export const handlers: Record<string, DemoHandler> = { admin_call: call, profile_image: demoImage };
