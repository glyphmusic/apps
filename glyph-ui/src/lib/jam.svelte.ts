// Jam ("listen together") state for the UI (app/jam.rs JamUi / Jam). The
// loops that keep a jam alive (host publishing, guest sync, clock) belong
// to the core; the UI asks `jam_state` once a second while a jam is on or
// the Jam page is open and shows what comes back. Commands are listed at
// the top of api/demo/extra.ts.
import { backend } from "./api/backend";
import { app } from "./state.svelte";
import { t } from "./i18n.svelte";
import type { Track } from "./api/types";

export type Reaction = "heart" | "fire" | "sparkles" | "thumbs_up";
export const REACTIONS: { id: Reaction; icon: string; emoji: string; label: string }[] = [
  { id: "heart", icon: "heart", emoji: "❤️", label: "jam.r.heart" },
  { id: "fire", icon: "flame", emoji: "🔥", label: "jam.r.fire" },
  { id: "sparkles", icon: "sparkles", emoji: "✨", label: "jam.r.sparkles" },
  { id: "thumbs_up", icon: "thumbs-up", emoji: "👍", label: "jam.r.thumbs_up" },
];

export interface JamBurst {
  id: number;
  icon: string;
  size: number;
  accent: boolean;
  x: number;
  dx: number;
  rise: number;
  rot0: number;
  rot1: number;
  scale: number;
  dur: number;
  delay: number;
}

export interface JamPerms { add: boolean; play_now: boolean; skip: boolean; seek: boolean; pause: boolean }
export interface JamMember { handle: string; avatar?: string | null; host: boolean }
export interface JamItem { id: string; track: Track; by: string; votes: string[]; voted?: boolean }
export interface JamEvent { at_ms: number; who: string; what: "joined" | "left" | "added" | "played" | "react" | "host"; title?: string; reaction?: Reaction }

/** Everything the Jam page shows (glyph_protocol::jam::State seen from this app). */
export interface JamState {
  code: string;
  host: boolean;
  /** Own handle and the host's. */
  me: string;
  host_handle: string;
  link: string;
  members: JamMember[];
  perms: JamPerms;
  approve_joins: boolean;
  /** Guest: the music plays on this device. */
  listen_here: boolean;
  /** Guest: the host's song has no source for this app. */
  unavailable: boolean;
  host_online: boolean;
  now: { track: Track; playing: boolean; position_ms: number } | null;
  /** The jam's queue (first 20 unless asked for all) and how many songs it holds. */
  queue: JamItem[];
  queue_len: number;
  /** Host: people waiting to be let in. */
  knocks: { handle: string; avatar?: string | null }[];
  feed: JamEvent[];
  /** Server clock (ms) when this was sent. */
  server_ms: number;
}

export interface JamSnapshot { jam: JamState | null; waiting: string | null }

export type JamPage = "main" | "invite" | "settings";

export const jam = $state({
  state: null as JamState | null,
  waiting: null as string | null,
  busy: false,
  error: null as string | null,
  queueAll: false,
  copied: 0,
  copiedKind: null as "code" | "link" | null,
  page: "main" as JamPage,
  bursts: [] as JamBurst[],
  /** Local clock (performance.now) when `state` arrived, to run the host's position on. */
  at: 0,
});

let burstSeq = 0;
const seenReacts = new Set<string>();
const localReacts: { reaction: Reaction; until: number }[] = [];

export function spawnBurst(reaction: Reaction) {
  const icon = REACTIONS.find((r) => r.id === reaction)?.icon ?? "heart";
  const count = 6;
  const created: JamBurst[] = [];
  let maxEnd = 0;
  for (let i = 0; i < count; i++) {
    const id = ++burstSeq;
    const slot = (i + 0.2 + Math.random() * 0.6) / count;
    const x = 8 + slot * 84;
    const rot0 = (Math.random() - 0.5) * 44;
    const dur = 1550 + Math.round(Math.random() * 450);
    const delay = i * 48 + Math.round(Math.random() * 40);
    maxEnd = Math.max(maxEnd, dur + delay);
    created.push({
      id,
      icon,
      size: 16 + Math.round(Math.random() * 7),
      accent: i % 2 === 0,
      x,
      dx: (Math.random() - 0.5) * 64,
      rise: 88 + Math.random() * 68,
      rot0,
      rot1: rot0 + (Math.random() - 0.5) * 60,
      scale: 0.92 + Math.random() * 0.34,
      dur,
      delay,
    });
  }
  const ids = new Set(created.map((b) => b.id));
  jam.bursts = [...jam.bursts.slice(-42), ...created];
  setTimeout(() => {
    jam.bursts = jam.bursts.filter((b) => !ids.has(b.id));
  }, maxEnd + 120);
}

export const QUEUE_SHOWN = 20;
export const FEED_SHOWN = 8;
export const jamActive = () => jam.state != null;

/** problem(): server error codes to texts. */
export function jamProblem(e: unknown): string {
  const m = String((e as Error)?.message ?? e ?? "");
  const code = ["jam_ended", "not_in_jam", "jam_removed", "jam_denied", "jam_full", "not_found", "signed_out"].find((c) => m.includes(c));
  switch (code) {
    case "jam_ended": return t("jam.err.ended");
    case "not_in_jam": case "jam_removed": return t("jam.err.removed");
    case "jam_denied": return t("jam.err.denied");
    case "jam_full": return t("jam.err.full");
    case "not_found": return t("jam.err.server");
    case "signed_out": return t("jam.sign_in");
    default: return m || t("jam.err.server");
  }
}

/** The jam code in `s`: a bare code, a `glyph:jam:` code or a `.../jam/<code>` link (jam::normalize_code). */
export function normalizeCode(s: string): string | null {
  s = s.trim();
  let raw = s;
  if (s.startsWith("glyph:jam:")) raw = s.slice(10);
  else if (s.includes("/jam/")) raw = s.slice(s.indexOf("/jam/") + 5).split(/[?#/]/)[0];
  const code = raw.replace(/[- ]/g, "").toUpperCase();
  return code.length === 6 && /^[ABCDEFGHJKLMNPQRSTUVWXYZ23456789]+$/.test(code) ? code : null;
}

export function apply(s: JamSnapshot | null | undefined) {
  if (!s) return;
  const had = jam.state != null;
  jam.state = s.jam;
  jam.waiting = s.waiting;
  jam.at = performance.now();
  if (!s.jam) {
    if (had) { jam.queueAll = false; jam.page = "main"; }
    jam.bursts = [];
    seenReacts.clear();
    localReacts.length = 0;
    return;
  }
  const now = performance.now();
  while (localReacts.length && localReacts[0].until < now) localReacts.shift();
  for (const e of s.jam.feed) {
    if (e.what !== "react" || !e.reaction) continue;
    const key = `${e.at_ms}:${e.who}:${e.reaction}`;
    if (seenReacts.has(key)) continue;
    seenReacts.add(key);
    if (!had) continue;
    const echoIdx = localReacts.findIndex((x) => x.reaction === e.reaction);
    if (echoIdx >= 0) { localReacts.splice(echoIdx, 1); continue; }
    spawnBurst(e.reaction);
  }
  if (seenReacts.size > 200) {
    const keep = new Set(s.jam.feed.filter((e) => e.what === "react" && e.reaction).map((e) => `${e.at_ms}:${e.who}:${e.reaction}`));
    seenReacts.clear();
    for (const k of keep) seenReacts.add(k);
  }
  if (!s.jam.host && jam.page === "settings") jam.page = "main";
}

export async function refresh() {
  try { apply(await backend().call<JamSnapshot>("jam_state", { queue_all: jam.queueAll })); } catch { /* offline: keep what is shown */ }
}

let users = 0, timer = 0;
/** The Jam page (or the overlay) is on screen: keep the state fresh. Returns the stop function. */
export function watchJam(): () => void {
  users++;
  refresh();
  if (!timer) timer = window.setInterval(refresh, 1000);
  return () => { if (--users <= 0) { users = 0; clearInterval(timer); timer = 0; } };
}

async function run<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T | undefined> {
  try { return await backend().call<T>(cmd, args); } catch (e) { jam.error = jamProblem(e); return undefined; }
}

export async function start() {
  if (jam.busy) return;
  jam.busy = true; jam.error = null;
  const r = await run<JamSnapshot>("jam_start");
  jam.busy = false;
  // The Jam page itself shows that it runs: no toast on top.
  if (r) apply(r);
}

export async function join(text: string) {
  const code = normalizeCode(text);
  if (!code) { jam.error = t("jam.err.code"); return; }
  if (jam.state?.code === code) return;
  if (jam.busy) return;
  jam.busy = true; jam.error = null; jam.waiting = null;
  const r = await run<JamSnapshot>("jam_join", { code });
  jam.busy = false;
  // The Jam page shows whose Jam it is: no toast on top.
  if (r) apply(r);
}

export async function stopWaiting() {
  const code = jam.waiting;
  if (!code) return;
  jam.waiting = null;
  await run("jam_leave", { code });
}

export async function leave() {
  const j = jam.state;
  if (!j) return;
  jam.state = null; jam.error = null; jam.queueAll = false; jam.page = "main";
  await run("jam_leave", { code: j.code });
  // The real backends say "left" / "ended" themselves.
  if (backend().kind !== "tauri") app.toastInfo(t(j.host ? "jam.ended_you" : "jam.left"));
}

/** Copies the code or link; the button itself turns into "Copied" (no
 *  toast on top). On phones the link opens the share menu instead. */
export async function copy(link: boolean) {
  const j = jam.state;
  if (!j) return;
  if (link && app.compact) {
    app.shareSheet = { link: { url: j.link, title: t("jam.title"), sub: j.code, icon: "users" } };
    return;
  }
  const text = link ? j.link : `glyph:jam:${j.code}`;
  try { await navigator.clipboard.writeText(text); } catch {
    try { await backend().call("copy_text", { text }); } catch { /* no clipboard access */ }
  }
  jam.copied = performance.now();
  jam.copiedKind = link ? "link" : "code";
}

const after = async <T>(cmd: string, args: Record<string, unknown>) => { const r = await run<JamSnapshot>(cmd, args); if (r) apply(r); return r as T | undefined; };
export const setPerms = (perms: JamPerms) => { if (jam.state) jam.state.perms = perms; return after("jam_set_perms", { perms }); };
export const setApprove = (on: boolean) => { if (jam.state) jam.state.approve_joins = on; return after("jam_set_approve", { on }); };
export const admit = (handle: string, allow: boolean) => after("jam_admit", { handle, allow });
export const kick = (handle: string) => after("jam_kick", { handle });
export const vote = (item: JamItem) => after("jam_vote", { id: item.id });
export const removeItem = (item: JamItem) => after("jam_remove", { id: item.id });
export const react = (reaction: Reaction) => {
  spawnBurst(reaction);
  localReacts.push({ reaction, until: performance.now() + 4000 });
  return after("jam_react", { reaction });
};
export const setListenHere = (here: boolean) => { if (jam.state) jam.state.listen_here = here; return after("jam_set_listen_here", { here }); };
export function showAll() { jam.queueAll = true; refresh(); }

/** "now", "3 min", "2 h" since `atMs` (server time). */
export function ago(nowMs: number, atMs: number): string {
  const s = Math.max(0, nowMs - atMs) / 1000;
  return s < 60 ? t("jam.now") : s < 3600 ? `${Math.floor(s / 60)} min` : `${Math.floor(s / 3600)} h`;
}
