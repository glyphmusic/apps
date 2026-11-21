// State of Settings > Transfer music (transfer.rs + glyph_core::import::accounts).
// Supports quick paste/drop, accumulative clipboard batching, real-time pushed
// progress events (with gentle fallback polling for demo mode), background status
// across the app, and one-click retry of failed lists.
import { backend } from "../api/backend";
import { app } from "../state.svelte";
import { t, tf } from "../i18n.svelte";
import { createDraftPlaylists, draftIdOf, draftValue, isDraftValue } from "../draft-playlists.svelte";
import { errText, pickFiles, readClipboard } from "./util";
import type { MigrateOverview, MigrateProgress, MigrateResume, MigrateRunItem, MigrateService, MigrateTarget } from "./types";

export interface ServiceDef {
  id: MigrateService;
  name: string;
  connect: "paste" | "takeout" | "profile" | "username";
  color: string;
  exportUrl?: string;
}

export const SERVICES: ServiceDef[] = [
  { id: "spotify", name: "Spotify", connect: "paste", color: "#1ed760" },
  { id: "youtube", name: "YouTube Music", connect: "takeout", color: "#ff4e45", exportUrl: "https://takeout.google.com/settings/takeout/custom/youtube" },
  { id: "soundcloud", name: "SoundCloud", connect: "profile", color: "#ff7700" },
  { id: "deezer", name: "Deezer", connect: "profile", color: "#a238ff" },
  { id: "lastfm", name: "Last.fm", connect: "username", color: "#e31b23" },
];

export const serviceOf = (id: MigrateService): ServiceDef => {
  if (id === "files") {
    return { id: "files", name: t("migrate.service.files"), connect: "paste", color: "rgba(255,255,255,.7)" };
  }
  return SERVICES.find((s) => s.id === id) ?? SERVICES[0];
};

export type MigrateStep = "pick" | "connect" | "link" | "loading" | "choose" | "running" | "done";

export const ALLOWED_FILE_EXTS = /\.(csv|tsv|json|m3u|m3u8|xspf|txt)$/i;

export const mg = $state({
  step: "pick" as MigrateStep,
  /** Where "loading" should return on error ("pick" for quick transfer, "connect" for service flow). */
  returnStep: "connect" as "pick" | "connect",
  service: null as MigrateService | null,
  error: null as string | null,
  profile: "",
  pasted: null as string | null,
  pastedCounts: [0, 0] as [number, number],
  /** Brief highlight pulse after appending pasted links. */
  justAdded: false,
  files: [] as { name: string; text: string }[],
  /** Unfinished run from an earlier start (undefined: not looked at yet). */
  resume: undefined as MigrateResume | null | undefined,
  overview: null as MigrateOverview | null,
  picked: {} as Record<string, boolean>,
  /** Destination per track collection id: `"liked" | "new" | "draft:<id>" | "pl:<id>"`. */
  targets: {} as Record<string, string>,
  progress: null as MigrateProgress | null,
  finished: null as MigrateProgress | null,
  /** Live running tick (ms) so the elapsed clock ticks smoothly during transfers. */
  clockNow: 0,
  runStartedClient: 0,
});

/** Temporary new playlists created from the Choose destination dropdown; auto-pruned when unused. */
export const migrateDrafts = createDraftPlaylists();

function pruneUnusedDrafts() {
  const used = new Set<string>();
  for (const v of Object.values(mg.targets)) {
    const did = draftIdOf(v);
    if (did) used.add(did);
  }
  migrateDrafts.prune(used);
}

let pulseTimer = 0;
let fallbackTimer = 0;
let clockTimer = 0;
let eventsInstalled = false;
let receivedPush = false;

/** Active background transfer info (shown in Library, Settings nav, Sidebar). */
export function migrateRunning(): MigrateProgress | null {
  return mg.step === "running" && mg.progress ? mg.progress : null;
}

export function formatCounts(songs: number, lists: number): string {
  if (songs > 0 && lists > 0) return tf("migrate.pasted", { songs, lists });
  if (songs === 1) return t("migrate.one_song");
  if (songs > 1) return tf("migrate.songs_only", { songs });
  if (lists === 1) return t("migrate.one_list");
  if (lists > 1) return tf("migrate.lists_only", { lists });
  return "";
}

export function formatElapsed(ms: number): string {
  const sec = Math.max(0, ms) / 1000;
  if (sec < 10) return `${sec.toFixed(1)} s`;
  const s = Math.round(sec);
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  const rem = s % 60;
  return `${m}:${String(rem).padStart(2, "0")}`;
}

function startClock(initialElapsed = 0) {
  mg.runStartedClient = Date.now() - initialElapsed;
  mg.clockNow = Date.now();
  if (!clockTimer) {
    clockTimer = window.setInterval(() => {
      mg.clockNow = Date.now();
    }, 250);
  }
}

function stopClock() {
  if (clockTimer) {
    clearInterval(clockTimer);
    clockTimer = 0;
  }
}

function stopFallback() {
  if (fallbackTimer) {
    clearInterval(fallbackTimer);
    fallbackTimer = 0;
  }
}

function handleProgress(p: MigrateProgress | null | undefined, fromPush = false) {
  if (!p) return;
  if (fromPush) {
    receivedPush = true;
    stopFallback();
  }
  if (p.service) mg.service = p.service;
  if (p.state === "running") {
    const wasRunning = mg.step === "running";
    mg.progress = p;
    mg.step = "running";
    if (!wasRunning) startClock(p.elapsed_ms ?? 0);
    return;
  }
  if (p.state === "finished" && mg.step === "running") {
    stopFallback();
    stopClock();
    for (const it of p.items ?? []) {
      if (it.status !== "done" || !it.playlist_id) continue;
      const prev = mg.targets[it.id];
      if (!prev || !isDraftValue(prev)) continue;
      for (const k of Object.keys(mg.targets)) {
        if (mg.targets[k] === prev) mg.targets[k] = `pl:${it.playlist_id}`;
      }
    }
    const failedDrafts = new Set<string>();
    for (const it of p.items ?? []) {
      if (it.status !== "failed") continue;
      const did = draftIdOf(mg.targets[it.id] ?? "");
      if (did) failedDrafts.add(did);
    }
    migrateDrafts.prune(failedDrafts);
    mg.finished = p;
    mg.progress = null;
    mg.step = "done";
    const svcName = mg.service ? serviceOf(mg.service).name : "";
    if (p.cancelled) {
      app.toastInfo(t("migrate.stopped"));
    } else {
      const checked = p.totals.songs - (p.totals.deferred ?? 0);
      const tail = checked > 0
        ? `${pct(p)}%`
        : p.totals.songs > 0
          ? formatCounts(p.totals.songs, 0)
          : "";
      app.toast([t("migrate.finished"), svcName, tail].filter(Boolean).join(" · "));
    }
  }
}

/** Subscribes once to pushed `migrate_progress` events and syncs any in-flight run. */
export function installMigrateEvents(): () => void {
  if (eventsInstalled) {
    void syncRunning();
    return () => {};
  }
  eventsInstalled = true;
  const off = backend().on("migrate_progress", (p) => handleProgress(p, true));
  void syncRunning();
  return () => {
    eventsInstalled = false;
    off();
    stopFallback();
    stopClock();
  };
}

export function pick(service: MigrateService, profile = "") {
  migrateDrafts.clear();
  mg.service = service;
  mg.profile = profile;
  mg.pasted = null;
  mg.pastedCounts = [0, 0];
  mg.files = [];
  mg.error = null;
  mg.returnStep = "connect";
  mg.step = "connect";
}

export function openLink() {
  migrateDrafts.clear();
  mg.error = null;
  mg.step = "link";
}

export function clearPasted() {
  mg.pasted = null;
  mg.pastedCounts = [0, 0];
  mg.error = null;
}

export function back() {
  migrateDrafts.clear();
  mg.error = null;
  mg.resume = undefined;
  if ((mg.step === "choose" || mg.step === "loading") && mg.service && mg.returnStep === "connect") {
    mg.step = "connect";
    return;
  }
  mg.step = "pick";
}

export function toPick() {
  if (mg.step === "running") return;
  migrateDrafts.clear();
  mg.error = null;
  mg.resume = undefined;
  mg.step = "pick";
}

/** Merges new pasted tokens with existing pasted text (deduplicated, preserving order). */
function mergePastedText(existing: string | null, incoming: string): string {
  if (!existing || !existing.trim()) return incoming.trim();
  const seen = new Set<string>();
  const out: string[] = [];
  for (const part of `${existing}\n${incoming}`.split(/\s+/)) {
    const clean = part.trim();
    if (!clean) continue;
    if (!seen.has(clean)) {
      seen.add(clean);
      out.push(clean);
    }
  }
  return out.join("\n");
}

function flashAdded() {
  mg.justAdded = true;
  clearTimeout(pulseTimer);
  pulseTimer = window.setTimeout(() => { mg.justAdded = false; }, 1600);
}

/** Appends links from `text` onto the current pasted batch (or replaces when `replace` is true). */
export async function appendPasted(text: string, replace = false): Promise<boolean> {
  const trimmed = text.trim();
  if (!trimmed) {
    mg.error = t("migrate.error.clipboard");
    return false;
  }
  const [newSongs, newLists] = (await backend().call<[number, number]>("migrate_pasted", { text: trimmed })) ?? [0, 0];
  if (newSongs + newLists === 0) {
    mg.error = t("migrate.error.clipboard");
    return false;
  }
  const combined = replace ? trimmed : mergePastedText(mg.pasted, trimmed);
  const [songs, lists] = (await backend().call<[number, number]>("migrate_pasted", { text: combined })) ?? [newSongs, newLists];
  mg.error = null;
  mg.pasted = combined;
  mg.pastedCounts = [songs, lists];
  flashAdded();
  return true;
}

/** Takes links copied in the other app from the clipboard and appends them to the batch. */
export async function pasteClipboard() {
  await appendPasted(await readClipboard(), false);
}

/** A text with several links was pasted into the search bar: open Transfer music with it. */
export async function openPasted(text: string) {
  pick(/youtube\.com|youtu\.be/i.test(text) ? "youtube" : "spotify");
  await appendPasted(text, true);
  app.openSettings("migrate");
}

export function addFiles(got: { name: string; text: string }[]) {
  let added = 0;
  for (const f of got) {
    if (!mg.files.some((x) => x.name === f.name)) {
      mg.files.push(f);
      added++;
    }
  }
  if (added > 0) {
    mg.error = null;
    flashAdded();
  }
}

export function removeFile(name: string) {
  mg.files = mg.files.filter((f) => f.name !== name);
}

export async function pickPlaylistFiles() {
  const got = await pickFiles(".csv,.tsv,.json,.m3u,.m3u8,.xspf,.txt", true);
  addFiles(got);
}

export async function readDroppedFiles(list: FileList | File[]): Promise<{ name: string; text: string }[]> {
  const arr = [...list].filter((f) => ALLOWED_FILE_EXTS.test(f.name));
  return Promise.all(arr.map(async (f) => ({ name: f.name, text: await f.text() })));
}

/** Detects a profile link from quick input (Deezer/SoundCloud/Last.fm). */
export function detectQuickProfile(text: string): { service: MigrateService; profile: string } | null {
  const v = text.trim();
  if (!v || /\s/.test(v)) return null;
  if (/^https?:\/\/(www\.)?deezer\.com\/([a-z]{2}\/)?(profile|user)\/\d+/i.test(v)) {
    return { service: "deezer", profile: v };
  }
  if (/^https?:\/\/(www\.)?last\.fm\/user\/[^/?#]+/i.test(v)) {
    return { service: "lastfm", profile: v };
  }
  const sc = v.match(/^https?:\/\/(www\.|m\.)?soundcloud\.com\/([^/?#]+)\/?(likes)?\/?$/i);
  if (sc && !/^(discover|search|you|stream|upload|settings|messages|notifications|charts|pages|mobile|imprint|terms-of-use|pro)$/i.test(sc[2])) {
    return { service: "soundcloud", profile: v };
  }
  return null;
}

/** Infers the best service label for a quick paste/drop batch. */
function detectBatchService(text: string, files: { name: string; text: string }[]): MigrateService {
  const v = text.trim();
  if (v) {
    if (/spotify\.com|spotify:|spotify\.link/i.test(v)) return "spotify";
    if (/youtube\.com|youtu\.be/i.test(v)) return "youtube";
    if (/deezer\.com|deezer\.page\.link/i.test(v)) return "deezer";
    if (/soundcloud\.com|on\.soundcloud\.com/i.test(v)) return "soundcloud";
    if (/last\.fm|lastfm\./i.test(v)) return "lastfm";
  }
  if (files.length) {
    const first = files[0].text.slice(0, 400);
    if (/^video\s*id\b/im.test(first)) return "youtube";
    if (/spotify:track:|"playlists"\s*:\s*\[/i.test(first)) return "spotify";
    return "files";
  }
  return "spotify";
}

/** Opens the Choose flow directly from the Quick Transfer zone (links, copied songs, profiles, or files). */
export async function quickContinue(text: string, files: { name: string; text: string }[]) {
  mg.error = null;
  const trimmed = text.trim();
  if (!trimmed && !files.length) {
    mg.error = t("migrate.error.nothing");
    return;
  }
  if (trimmed && !files.length) {
    const prof = detectQuickProfile(trimmed);
    if (prof) {
      mg.returnStep = "pick";
      mg.profile = prof.profile;
      mg.pasted = null;
      mg.files = [];
      return load(prof.service, { profile: prof.profile });
    }
  }
  const svc = detectBatchService(trimmed, files);
  mg.returnStep = "pick";
  mg.pasted = trimmed || null;
  mg.files = files.slice();
  return load(svc, { pasted: trimmed, files: files.map((f) => ({ name: f.name, text: f.text })) });
}

export async function connect() {
  const s = mg.service;
  if (!s) return;
  mg.error = null;
  mg.returnStep = "connect";
  const kind = serviceOf(s).connect;
  if (kind === "paste" || kind === "takeout") {
    if (mg.pasted == null && !mg.files.length) { mg.error = t("migrate.error.nothing"); return; }
    return load(s, { pasted: mg.pasted ?? "", files: $state.snapshot(mg.files) });
  }
  const profile = mg.profile.trim();
  if (!profile) { mg.error = t("migrate.error.profile"); return; }
  return load(s, { profile });
}

/** Continues a stopped run with what was handed over then. */
export function resumeRun() {
  const r = mg.resume;
  if (r) {
    mg.returnStep = "pick";
    void load(r.service, { resume: true });
  }
}

async function load(service: MigrateService, login: Record<string, unknown>) {
  mg.service = service;
  mg.step = "loading";
  try {
    const o = await backend().call<MigrateOverview>("migrate_overview", { service, ...login });
    if (mg.step !== "loading") return;
    mg.overview = o;
    // A stopped run of the same service: pick what is left; else the defaults.
    mg.picked = Object.fromEntries(o.collections.filter((c) => (o.done_before.length ? !o.done_before.includes(c.id) : c.default_on)).map((c) => [c.id, true]));
    mg.targets = Object.fromEntries(
      o.collections
        .filter((c) => c.kind === "liked" || c.kind === "playlist")
        .map((c) => [c.id, c.kind === "liked" ? "liked" : "new"]),
    );
    migrateDrafts.clear();
    mg.step = "choose";
  } catch (e) {
    mg.error = errText(e);
    mg.step = mg.returnStep;
  }
}

export function toggle(id: string) { if (mg.picked[id]) delete mg.picked[id]; else mg.picked[id] = true; }

export function setTarget(id: string, value: string) {
  mg.targets[id] = value;
  mg.picked[id] = true;
  pruneUnusedDrafts();
}

export function promptNewTarget(id: string, initial = "") {
  migrateDrafts.promptCreate({
    initial,
    onCreated: (draft) => setTarget(id, draftValue(draft.id)),
  });
}

export function promptRenameTarget(id: string, initial = "") {
  const did = draftIdOf(mg.targets[id] ?? "");
  if (did && migrateDrafts.get(did)) {
    migrateDrafts.promptRename(did);
    return;
  }
  promptNewTarget(id, initial);
}

/** Ticks (or unticks) every playlist. */
export function allPlaylists(on: boolean) {
  for (const c of mg.overview?.collections ?? []) if (c.kind === "playlist") { if (on) mg.picked[c.id] = true; else delete mg.picked[c.id]; }
}

async function pollOnce() {
  const p = await backend().call<MigrateProgress>("migrate_progress");
  handleProgress(p, false);
}

const pct = (p: MigrateProgress) => {
  const checked = p.totals.songs - (p.totals.deferred ?? 0);
  return checked <= 0 ? 100 : Math.floor((p.totals.matched * 100) / checked);
};
export { pct as migratePercent };

/** Starts fallback polling only if the backend does not push `migrate_progress` events (e.g. demo). */
function watchFallback() {
  if (receivedPush || fallbackTimer) return;
  fallbackTimer = window.setInterval(pollOnce, 250);
}

/** Syncs any transfer already running in the backend. */
export async function syncRunning() {
  const p = await backend().call<MigrateProgress>("migrate_progress");
  if (p?.state === "running") {
    handleProgress(p, false);
    watchFallback();
  }
}

function buildTargets(cols: { id: string; kind: string }[]): Record<string, MigrateTarget> {
  const targets: Record<string, MigrateTarget> = {};
  for (const c of cols) {
    if (c.kind !== "liked" && c.kind !== "playlist") continue;
    const raw = mg.targets[c.id] ?? (c.kind === "liked" ? "liked" : "new");
    if (raw === "liked") targets[c.id] = { kind: "liked" };
    else if (raw.startsWith("pl:")) targets[c.id] = { kind: "playlist", id: raw.slice(3) };
    else if (isDraftValue(raw)) {
      const did = draftIdOf(raw)!;
      const draft = migrateDrafts.get(did);
      targets[c.id] = draft
        ? { kind: "new_playlist", name: draft.name, draft_id: draft.id }
        : { kind: "new_playlist", name: null };
    } else {
      targets[c.id] = { kind: "new_playlist", name: null };
    }
  }
  return targets;
}

export async function start() {
  if (!mg.overview || !mg.service) return;
  const order = { liked: 0, playlist: 1, albums: 2, artists: 3 } as const;
  const cols = mg.overview.collections.filter((c) => mg.picked[c.id]).sort((a, b) => order[a.kind] - order[b.kind]);
  const selected = cols.map((c) => c.id);
  if (!selected.length) return;
  const targets = buildTargets(cols);
  mg.resume = undefined;
  mg.error = null;
  try {
    await backend().call("migrate_start", { service: mg.service, selected, targets });
    const usedDrafts = new Set<string>();
    for (const c of cols) {
      const did = draftIdOf(mg.targets[c.id] ?? "");
      if (did) usedDrafts.add(did);
    }
    migrateDrafts.prune(usedDrafts);
    app.toast(`${t("migrate.started")} · ${serviceOf(mg.service).name}`);
    mg.step = "running";
    mg.progress = null;
    startClock(0);
    watchFallback();
    void pollOnce();
  } catch (e) { mg.error = errText(e); }
}

/** Retries only the collections that failed in the last run. */
export async function retryFailed() {
  if (!mg.finished || !mg.service) return;
  const failedItems = (mg.finished.items ?? []).filter((it) => it.status === "failed");
  if (!failedItems.length) return;
  const selected = failedItems.map((it) => it.id);
  const targets = buildTargets(failedItems);
  mg.error = null;
  try {
    await backend().call("migrate_start", { service: mg.service, selected, targets });
    mg.step = "running";
    mg.progress = null;
    startClock(0);
    watchFallback();
    void pollOnce();
  } catch (e) { mg.error = errText(e); }
}

export async function cancel() {
  if (mg.progress) mg.progress.stopping = true;
  await backend().call("migrate_cancel");
}

/** Opens a transferred playlist / Liked Songs / Library directly and closes Settings. */
export function openTransferred(item: MigrateRunItem) {
  app.settingsOpen = false;
  if (item.playlist_id) {
    app.openListView({ kind: "playlist", id: item.playlist_id });
  } else if (item.kind === "liked") {
    app.openListView({ kind: "liked" });
  } else {
    app.navigate("library");
  }
}

/** Forgets a stopped move. */
export async function discardResume() { await backend().call("migrate_discard"); mg.resume = null; }

export async function loadResume() {
  if (mg.resume === undefined) mg.resume = (await backend().call<MigrateResume | null>("migrate_resume_state")) ?? null;
}

/** Name of a collection (collection_name). */
export function collectionName(c: { kind: string; name: string; hint: string }): string {
  if (c.kind === "liked") return t("migrate.liked");
  if (c.kind === "albums") return t("migrate.albums");
  if (c.kind === "artists") return t("migrate.artists");
  return c.name.trim() ? c.name : c.hint;
}
