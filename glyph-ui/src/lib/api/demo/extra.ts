// Demo handlers for the profile, statistics, local files, Connect / devices,
// Jam and tour screens (helper 4). Everything here is made up.
//
// NEW BACKEND COMMANDS (the Rust / Tauri side implements them from this list;
// `backend().call(name, args)` -> result). Errors are thrown / rejected with a
// message that contains one of the codes named below.
//
// Profile (ui/profile.rs)
//   profile            { handle }                     -> Profile & { private?: boolean; recent?: Track[] } ; errors: signed_out, not_found, rate_limited
//   profile_playlist   { handle, id }                 -> Track[]  (songs of a profile playlist)
//   profile_follow     { handle, on }                 -> void
//   profile_set_image  { banner: boolean, image: string | null }  -> void  (image = cropped JPEG data URL: 512x512 picture, 1500x500 banner; null removes) ; errors: too_big
// Statistics (ui/stats.rs; types in lib/pages/stats/types.ts)
//   stats_summary      { range: "month"|"last_month"|"year"|"all" } -> StatsSummary
//   stats_recap        { year: string }                -> StatsRecap
//   share_songs        { title, tracks: Track[], kind: "playlist" } -> string (link; also copied)
// Local files (ui/local.rs, app/local.rs; types in lib/pages/local/local.svelte.ts)
//   pick_folders       { prompt }                      -> string[] | null   (native folder dialog, several allowed)
//   local_scan         { folders: string[] }           -> { files, read, failed }
//   local_page         { tab, dir, artist, album, query } -> LocalPage   (tab 0 folders, 1 artists, 2 albums, 3 songs; query lower-case)
//   local_play         { tab, dir, artist, album, query, shuffle } -> void  (plays what local_page lists; everything while it lists folders / artists / albums)
// Connect panel (ui/connect.rs; type DeviceX in lib/connect.ts)
//   devices_rescan     { now }                         -> DeviceX[]  (search the network; resolves when the search is over)
//   device_select      { id }                          -> DeviceX[]  (play on a found device)
//   device_connect     { id }                          -> DeviceX[]  (control / move playback to another Glyph app of the account)
//   device_use_local   {}                              -> DeviceX[]  (back to this computer)
//   open_url           { url }                         -> void       (system browser)
// Jam (ui/jam.rs, app/jam.rs; types in lib/jam.svelte.ts). Errors: jam_ended, not_in_jam, jam_removed, jam_denied, jam_full, not_found, signed_out
//   jam_state          { queue_all }                   -> { jam: JamState | null; waiting: string | null }   (polled once a second while the Jam page is open)
//   jam_start / jam_join { code } / jam_set_perms { perms } / jam_set_approve { on } / jam_admit { handle, allow } / jam_kick { handle }
//   jam_vote { id } / jam_remove { id } / jam_react { reaction } / jam_set_listen_here { here }   -> { jam, waiting } (the new state)
//   jam_leave          { code }                        -> void
//   account_devices_open { open }                    -> void  (the Connect panel opened / closed)
//   artist_photo       { name }                        -> string | null  (picture for artist tiles without opening the page)
//   artist_verified    { name }                        -> boolean        (verified badge)
//   set_playing_from   { list }                      -> void  (queue started from this list; Tauri only)
// Tour (ui/onboarding.rs)
//   onboarding_seen    {}                              -> void  (writes the "seen" marker)
//
// Also wanted from the shell: `app.devices` filled by the "devices" event or by the three device commands.
import type { DemoCtx, DemoHandler } from "../demo";
import type { Playlist, Profile, Track } from "../types";
import { ARTISTS, TRACKS, byArtist, demoCover, hash } from "../demo-data";
import type { StatsRange, StatsRecap, StatsSummary } from "../../pages/stats/types";
import type { LocalPage } from "../../pages/local/local.svelte";
import type { DeviceX } from "../../connect";
import type { JamItem, JamSnapshot, JamState } from "../../jam.svelte";

const OWN = "demo_user";

// ---------------------------------------------------------------- profile
const own: { avatar: string | null; banner: string | null } = { avatar: demoCover("own-avatar"), banner: null };
const NAMES = ["Rowan Fell", "Ida Marsh", "Teo Lindqvist", "Nora Kade", "Pim Okafor", "Sol Varga"];
/** Handles that exist in the demo (search finds people by exact handle). */
const KNOWN = new Set([OWN, "demo", "quiet_one", "rowan", "ida_m", "teo", "nora", "pim", "sol_v", "maria", "mailo"]);

function profile(handle: string): Profile & { private?: boolean; recent?: Track[] } {
  const h = handle.toLowerCase();
  if (!KNOWN.has(h)) throw new Error("not_found");
  const isMe = h === OWN;
  const n = hash(h);
  const pick = (start: number, len: number) => Array.from({ length: len }, (_, i) => TRACKS[(start + i * 3) % TRACKS.length]);
  const playlists: Playlist[] = ["Evening Loop", "Gym Mix", "Slow Mornings"].slice(0, 1 + (n % 3)).map((name, i) => ({
    id: `pp-${h}-${i}`, name, tracks: pick(n % 17 + i * 5, 4 + i * 2), cover: i === 2 ? demoCover(`${h}-pl-${i}`) : null,
  }));
  return {
    handle: h,
    name: isMe ? "You (demo)" : NAMES[n % NAMES.length],
    avatar: isMe ? own.avatar : n % 4 === 0 ? null : demoCover(`${h}-avatar`),
    banner: isMe ? own.banner : n % 3 === 0 ? null : demoCover(`${h}-banner`),
    bio: n % 2 === 0 ? "Analog synths, late night drives, and old records." : null,
    followers: n % 900, following: n % 120, playlists,
    top_artists: ARTISTS.slice(n % 4, (n % 4) + 4),
    top_tracks: pick(n % 11, 6),
    is_me: isMe, follows: false,
    private: h === "quiet_one",
    recent: pick(n % 7 + 2, 8),
    sections: ["playlists", "recent"],
  };
}

// ---------------------------------------------------------------- stats
const SCALE: Record<StatsRange, number> = { month: 1, last_month: 0.9, year: 9, all: 15 };
function summary(range: StatsRange): StatsSummary {
  const k = SCALE[range], seed = hash(range);
  const artists = [...ARTISTS].sort((a, b) => hash(a + range) - hash(b + range));
  const top_artists = artists.slice(0, 10).map((name, i) => ({ name, ms: Math.round((9 - i * 0.7 + (hash(name) % 5) / 10) * 3_600_000 * k * 0.35), plays: Math.round((60 - i * 4) * k), track: byArtist(name)[0] }));
  const tracks = [...TRACKS].sort((a, b) => hash(a.id + range) - hash(b.id + range)).slice(0, 10);
  const top_tracks = tracks.map((track, i) => ({ track, plays: Math.round((24 - i * 2) * k), ms: Math.round((24 - i * 2) * k * (track.duration_ms ?? 180_000)) }));
  const hours = Array.from({ length: 24 }, (_, h) => Math.round(Math.max(0, 6 * Math.sin(((h - 7) / 24) * Math.PI * 2) + 3 + (hash(`${seed}h${h}`) % 4)) * 600_000 * k));
  const weekdays = Array.from({ length: 7 }, (_, d) => Math.round((14 + (hash(`${seed}d${d}`) % 14) + (d > 4 ? 8 : 0)) * 600_000 * k));
  const now = new Date();
  const months = Array.from({ length: 14 }, (_, i) => {
    const d = new Date(now.getFullYear(), now.getMonth() - (13 - i), 1);
    return { month: `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`, ms: Math.round((30 + (hash(`m${i}`) % 90)) * 3_600_00 * 3) };
  });
  const ms = hours.reduce((a, b) => a + b, 0);
  return {
    range, ms, plays: Math.round(212 * k), skips: Math.round(31 * k), artists: ARTISTS.length, songs: Math.min(TRACKS.length, Math.round(18 * k) + 6),
    active_days: Math.max(1, Math.round(Math.min(26, 9 * k))), top_artists, top_tracks, hours, weekdays, months,
    years: [String(now.getFullYear()), String(now.getFullYear() - 1)],
  };
}
function recap(year: string): StatsRecap {
  const s = summary("year");
  return { year, ms: s.ms, plays: s.plays, artists: s.artists, songs: s.songs, top_artists: s.top_artists, top_tracks: s.top_tracks, hours: s.hours, weekdays: s.weekdays, months: s.months };
}

// ---------------------------------------------------------------- local files
interface LocalFile { track: Track; dir: string; root: string }
const trimEnd = (p: string) => p.replace(/[\\/]+$/, "");
function localFiles(ctx: DemoCtx): LocalFile[] {
  const roots = ctx.config.local.folders;
  if (!roots.length) return [];
  return TRACKS.map((t) => {
    const root = roots[hash(t.id) % roots.length];
    return { track: { ...t, id: `file:${t.id}` }, dir: `${trimEnd(root)}/${t.artist}/${t.album}`, root };
  });
}
const matches = (t: Track, q: string) => !q || t.title.toLowerCase().includes(q) || t.artist.toLowerCase().includes(q) || (t.album ?? "").toLowerCase().includes(q);
const base = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;
let scanned = 0;
let pickN = 0;

function localTracks(ctx: DemoCtx, a: Record<string, any>): Track[] {
  const files = localFiles(ctx), q = String(a.query ?? "");
  if (a.tab === 0) {
    const inside = a.dir ? files.filter((f) => f.dir === a.dir || f.dir.startsWith(`${a.dir}/`)) : files;
    return (a.dir ? inside : files).map((f) => f.track).filter((t) => matches(t, q));
  }
  if (a.tab === 1) return files.map((f) => f.track).filter((t) => (a.artist ? t.artist === a.artist : !q || t.artist.toLowerCase().includes(q)));
  if (a.tab === 2) return files.map((f) => f.track).filter((t) => (a.album ? t.album === a.album[0] && t.artist === a.album[1] : true));
  return files.map((f) => f.track).filter((t) => matches(t, q));
}

function localPage(ctx: DemoCtx, a: Record<string, any>): LocalPage {
  const files = localFiles(ctx), q = String(a.query ?? "");
  const page: LocalPage = { total: files.length, progress: scanned > Date.now() ? [Math.round((1 - (scanned - Date.now()) / 1400) * files.length), files.length] : null, dirs: [], artists: [], albums: [], tracks: [] };
  if (a.tab === 0) {
    if (!a.dir) {
      page.dirs = ctx.config.local.folders.map((r) => ({ path: r, name: base(r), count: files.filter((f) => f.root === r).length }));
    } else {
      const here = files.filter((f) => f.dir === a.dir || f.dir.startsWith(`${a.dir}/`));
      if (!q) {
        const sub = new Map<string, number>();
        for (const f of here) if (f.dir !== a.dir) { const first = f.dir.slice(String(a.dir).length + 1).split("/")[0]; sub.set(first, (sub.get(first) ?? 0) + 1); }
        page.dirs = [...sub].map(([name, count]) => ({ path: `${a.dir}/${name}`, name, count }));
        page.tracks = here.filter((f) => f.dir === a.dir).map((f) => f.track);
      } else page.tracks = here.map((f) => f.track).filter((t) => matches(t, q));
    }
  } else if (a.tab === 1) {
    if (a.artist) page.tracks = files.map((f) => f.track).filter((t) => t.artist === a.artist && matches(t, q));
    else {
      const m = new Map<string, number>();
      for (const f of files) m.set(f.track.artist, (m.get(f.track.artist) ?? 0) + 1);
      page.artists = [...m].filter(([n]) => !q || n.toLowerCase().includes(q)).map(([name, count]) => ({ name, count })).sort((x, y) => x.name.localeCompare(y.name));
    }
  } else if (a.tab === 2) {
    const m = new Map<string, LocalPage["albums"][number]>();
    for (const f of files) {
      const key = `${f.track.album}|${f.track.artist}`, e = m.get(key);
      if (e) e.count++; else m.set(key, { title: f.track.album ?? "", artist: f.track.artist, count: 1, cover: f.track });
    }
    if (a.album) page.tracks = files.map((f) => f.track).filter((t) => t.album === a.album[0] && t.artist === a.album[1] && matches(t, q));
    else page.albums = [...m.values()].filter((x) => !q || x.title.toLowerCase().includes(q) || x.artist.toLowerCase().includes(q)).sort((x, y) => x.title.localeCompare(y.title));
  } else page.tracks = files.map((f) => f.track).filter((t) => matches(t, q));
  return page;
}

// ---------------------------------------------------------------- devices
let devs: DeviceX[] = [
  { id: "this", name: "This computer", kind: "this", active: true },
  { id: "acc-1", name: "Studio laptop", kind: "account", active: false, platform: "windows", playing: true, track: "Tame Impala · Let It Happen" },
  { id: "acc-2", name: "Pocket phone", kind: "account", active: false, platform: "android", playing: false },
  { id: "net-1", name: "Living room TV", kind: "cast", active: false, protocol: "Google Cast", icon: "tv" },
  { id: "net-2", name: "Kitchen speaker", kind: "upnp", active: false, protocol: "DLNA", icon: "speaker" },
  { id: "net-3", name: "Bedroom stereo", kind: "airplay", active: false, protocol: "AirPlay", icon: "speaker" },
];
const activate = (ctx: DemoCtx, id: string) => { devs = devs.map((d) => ({ ...d, active: d.id === id })); ctx.emit("devices", devs); return devs; };

// ---------------------------------------------------------------- jam
let J: (JamState & { _start: number; _base: number }) | null = null;
const MEMBERS = ["river_fox", "pixel_moth", "late_comet"];
const jamClone = (queueAll: boolean): JamState | null => {
  if (!J) return null;
  const now = J.now ? { ...J.now, position_ms: Math.min(J.now.track.duration_ms ?? Infinity, J._base + (J.now.playing ? Date.now() - J._start : 0)) } : null;
  const { _start, _base, ...rest } = J;
  void _start; void _base;
  return { ...rest, now, queue: (queueAll ? rest.queue : rest.queue.slice(0, 20)).map((i) => ({ ...i, voted: i.votes.includes(rest.me) })), server_ms: Date.now() };
};
const snap = (queueAll = false): JamSnapshot => ({ jam: jamClone(queueAll), waiting: null });
const ev = (who: string, what: JamState["feed"][number]["what"], extra: Partial<JamState["feed"][number]> = {}) => ({ at_ms: Date.now(), who, what, ...extra });
const perms = { add: true, play_now: true, skip: true, seek: true, pause: true };
function newJam(host: boolean, code: string, ctx: DemoCtx): void {
  const cur = ctx.player.current ?? TRACKS[0];
  const queue: JamItem[] = TRACKS.slice(5, 10).map((track, i) => ({ id: track.id, track, by: i % 2 ? MEMBERS[i % 3] : host ? OWN : MEMBERS[0], votes: i === 1 ? [MEMBERS[1], MEMBERS[2]] : [] }));
  J = {
    code, host, me: OWN, host_handle: host ? OWN : MEMBERS[0], link: `https://glyph.my/jam/${code}`,
    members: [{ handle: host ? OWN : MEMBERS[0], host: true, avatar: null }, ...(host ? MEMBERS.slice(0, 2) : [OWN, MEMBERS[1]]).map((handle) => ({ handle, host: false, avatar: demoCover(handle) }))],
    perms: { ...perms }, approve_joins: host, listen_here: true, unavailable: false, host_online: true,
    now: { track: host ? cur : TRACKS[11], playing: true, position_ms: 42_000 }, queue, queue_len: queue.length,
    knocks: host ? [{ handle: "moss_lane", avatar: demoCover("moss_lane") }] : [],
    feed: [ev(MEMBERS[1], "joined"), ev(MEMBERS[0], "added", { title: queue[0].track.title }), ev(MEMBERS[1], "react", { reaction: "fire" })],
    server_ms: Date.now(), _start: Date.now(), _base: 42_000,
  };
}
const jamHandlers: Record<string, DemoHandler> = {
  jam_state: (_, a) => snap(!!a.queue_all),
  account_devices_open: () => undefined,
  set_playing_from: () => undefined,
  jam_start: (ctx) => { newJam(true, "K7P2QX", ctx); return snap(); },
  jam_join: (ctx, a) => { newJam(false, String(a.code), ctx); return snap(); },
  jam_leave: () => { J = null; },
  jam_set_perms: (_, a) => { if (J) J.perms = a.perms; return snap(); },
  jam_set_approve: (_, a) => { if (J) J.approve_joins = a.on; return snap(); },
  jam_admit: (_, a) => {
    if (J) {
      J.knocks = J.knocks.filter((k) => k.handle !== a.handle);
      if (a.allow) { J.members.push({ handle: a.handle, host: false, avatar: demoCover(a.handle) }); J.feed.push(ev(a.handle, "joined")); }
    }
    return snap();
  },
  jam_kick: (_, a) => { if (J) J.members = J.members.filter((m) => m.handle !== a.handle); return snap(); },
  jam_vote: (_, a) => {
    const it = J?.queue.find((i) => i.id === a.id);
    if (J && it) it.votes = it.votes.includes(J.me) ? it.votes.filter((v) => v !== J!.me) : [...it.votes, J.me];
    return snap();
  },
  jam_remove: (_, a) => { if (J) { J.queue = J.queue.filter((i) => i.id !== a.id); J.queue_len = J.queue.length; } return snap(); },
  jam_react: (_, a) => { if (J) J.feed.push(ev(J.me, "react", { reaction: a.reaction })); return snap(); },
  jam_set_listen_here: (_, a) => { if (J) J.listen_here = a.here; return snap(); },
};

export const handlers: Record<string, DemoHandler> = {
  artist_photo: () => null,
  artist_verified: () => false,
  profile: (_, a) => profile(String(a.handle).trim().replace(/^@/, "")),
  profile_playlist: (_, a) => profile(String(a.handle)).playlists.find((p) => p.id === a.id)?.tracks ?? [],
  profile_follow: () => {},
  profile_set_image: (_, a) => { if (a.banner) own.banner = a.image; else own.avatar = a.image; },
  stats_summary: (_, a) => summary(a.range as StatsRange),
  stats_recap: (_, a) => recap(String(a.year)),
  share_songs: async (ctx, a) => {
    const link = `https://glyph.my/p/${hash(String(a.title)).toString(36)}`;
    if (a.copy === false) return link;
    try { await navigator.clipboard.writeText(link); } catch { /* preview */ }
    ctx.emit("toast", { text: "Playlist code copied" });
    return link;
  },
  pick_folders: () => (pickN++ % 2 === 0 ? ["/home/demo/Music"] : ["/home/demo/More Music"]),
  local_scan: (ctx, a) => { scanned = Date.now() + 1400; const n = a.folders.length ? localFiles(ctx).length : 0; return new Promise((r) => setTimeout(() => r({ files: n, read: n, failed: 0 }), 1400)); },
  local_page: (ctx, a) => localPage(ctx, a),
  local_play: (ctx, a) => {
    let tracks = localTracks(ctx, a);
    if (a.shuffle) tracks = [...tracks].sort(() => Math.random() - 0.5);
    if (tracks.length) return (ctx as unknown as { call: (c: string, x: object) => Promise<unknown> }).call("play", { tracks, index: 0, context: "Local files" });
  },
  devices_rescan: () => new Promise((r) => setTimeout(() => r(devs), 700)),
  device_select: (ctx, a) => activate(ctx, a.id),
  device_connect: (ctx, a) => activate(ctx, a.id),
  device_use_local: (ctx) => activate(ctx, "this"),
  open_url: (_, a) => { window.open(String(a.url), "_blank", "noopener"); },
  share_system: async (ctx, a) => {
    const nav = navigator as Navigator & { share?: (d: ShareData) => Promise<void> };
    if (nav.share) { try { await nav.share({ url: String(a.text) }); return; } catch { /* cancelled */ } }
    try { await navigator.clipboard.writeText(String(a.text)); } catch { /* preview */ }
    ctx.emit("toast", { text: "Link copied" });
  },
  copy_text: async (_, a) => { try { await navigator.clipboard.writeText(String(a.text)); } catch { /* preview */ } },
  onboarding_seen: () => {},
  ...jamHandlers,
};
