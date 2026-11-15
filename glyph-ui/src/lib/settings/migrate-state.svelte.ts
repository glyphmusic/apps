// State of Settings > Transfer music (ui/migrate.rs MigrateUi + the Glyph
// methods). Nobody logs in: Spotify / YouTube Music hand over what was copied
// in their app (clipboard) or a Google Takeout file, Deezer and SoundCloud a
// public profile, Last.fm a user name. Then: choose what comes over, run in
// the background (the backend does the work, we poll its progress so the
// move keeps going when settings are closed), result.
import { backend } from "../api/backend";
import { app } from "../state.svelte";
import { t } from "../i18n.svelte";
import { errText, pickFiles, readClipboard } from "./util";
import type { MigrateOverview, MigrateProgress, MigrateResume, MigrateService, MigrateTarget } from "./types";

export const SERVICES: { id: MigrateService; name: string; connect: "paste" | "takeout" | "profile" | "username"; exportUrl?: string }[] = [
  { id: "spotify", name: "Spotify", connect: "paste" },
  { id: "youtube", name: "YouTube Music", connect: "takeout", exportUrl: "https://takeout.google.com/settings/takeout/custom/youtube" },
  { id: "soundcloud", name: "SoundCloud", connect: "profile" },
  { id: "deezer", name: "Deezer", connect: "profile" },
  { id: "lastfm", name: "Last.fm", connect: "username" },
];
export const serviceOf = (id: MigrateService) => SERVICES.find((s) => s.id === id)!;

type Step = "pick" | "connect" | "link" | "loading" | "choose" | "running" | "done";

export const mg = $state({
  step: "pick" as Step,
  service: null as MigrateService | null,
  error: null as string | null,
  profile: "",
  pasted: null as string | null,
  pastedCounts: [0, 0] as [number, number],
  files: [] as { name: string; text: string }[],
  /** Unfinished run from an earlier start (undefined: not looked at yet). */
  resume: undefined as MigrateResume | null | undefined,
  overview: null as MigrateOverview | null,
  picked: {} as Record<string, boolean>,
  /** Destination per track collection id: `"liked" | "new" | "pl:<id>"`. */
  targets: {} as Record<string, string>,
  /** Optional custom playlist name per collection id when destination is `"new"`. */
  targetNames: {} as Record<string, string>,
  progress: null as MigrateProgress | null,
  finished: null as MigrateProgress | null,
});

/** A move is running (sidebar / library show its progress). */
export const migrateRunning = () => (mg.step === "running" ? mg.progress : null);

export function pick(service: MigrateService, profile = "") {
  mg.service = service; mg.profile = profile; mg.pasted = null; mg.pastedCounts = [0, 0]; mg.files = []; mg.error = null; mg.step = "connect";
}

export function openLink() {
  mg.error = null;
  mg.step = "link";
}

export function clearPasted() {
  mg.pasted = null;
  mg.pastedCounts = [0, 0];
  mg.error = null;
}

export function back() {
  mg.error = null;
  mg.resume = undefined;
  if (mg.step === "choose" && mg.service) { mg.step = "connect"; return; }
  mg.step = "pick";
}

export function toPick() {
  if (mg.step === "running") return;
  mg.error = null;
  mg.resume = undefined;
  mg.step = "pick";
}

async function setPasted(text: string) {
  const [songs, lists] = (await backend().call<[number, number]>("migrate_pasted", { text })) ?? [0, 0];
  if (songs + lists === 0) { mg.error = t("migrate.error.clipboard"); return; }
  mg.error = null; mg.pastedCounts = [songs, lists]; mg.pasted = text;
}

/** Takes the links copied in the other app from the clipboard. */
export async function pasteClipboard() { await setPasted(await readClipboard()); }

/** A text with several links was pasted into the search bar: open Transfer music with it. */
export async function openPasted(text: string) {
  pick(/youtube\.com|youtu\.be/i.test(text) ? "youtube" : "spotify");
  await setPasted(text);
  app.openSettings("migrate");
}

export async function pickPlaylistFiles() {
  const got = await pickFiles(".csv,.json,.m3u,.m3u8,.xspf,.txt", true);
  for (const f of got) if (!mg.files.some((x) => x.name === f.name)) mg.files.push(f);
  if (got.length) mg.error = null;
}

export async function connect() {
  const s = mg.service;
  if (!s) return;
  mg.error = null;
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
  if (r) load(r.service, { resume: true });
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
    mg.targetNames = {};
    mg.step = "choose";
  } catch (e) { mg.error = errText(e); mg.step = "connect"; }
}

export function toggle(id: string) { if (mg.picked[id]) delete mg.picked[id]; else mg.picked[id] = true; }

export function setTarget(id: string, value: string) {
  mg.targets[id] = value;
  mg.picked[id] = true;
}

export function setTargetName(id: string, name: string) {
  mg.targetNames[id] = name;
}

/** Ticks (or unticks) every playlist. */
export function allPlaylists(on: boolean) {
  for (const c of mg.overview?.collections ?? []) if (c.kind === "playlist") { if (on) mg.picked[c.id] = true; else delete mg.picked[c.id]; }
}

let timer = 0;
async function poll() {
  const p = await backend().call<MigrateProgress>("migrate_progress");
  if (!p) return;
  if (p.state === "running") { mg.progress = p; mg.step = "running"; return; }
  if (p.state === "finished" && mg.step === "running") {
    clearInterval(timer); timer = 0;
    mg.finished = p; mg.progress = null; mg.step = "done";
    if (p.cancelled) app.toastInfo(t("migrate.stopped"));
    else app.toast(`${t("migrate.finished")} · ${serviceOf(mg.service!).name} · ${pct(p)}%`);
  }
}
const pct = (p: MigrateProgress) => (p.totals.songs === 0 ? 100 : Math.floor((p.totals.matched * 100) / p.totals.songs));
export { pct as migratePercent };

function watch() { if (!timer) timer = window.setInterval(poll, 500); }

/** Settings page opened: a move may be running from before. */
export async function syncRunning() {
  const p = await backend().call<MigrateProgress>("migrate_progress");
  if (p?.state === "running") { mg.progress = p; mg.step = "running"; watch(); }
}

export async function start() {
  if (!mg.overview || !mg.service) return;
  const order = { liked: 0, playlist: 1, albums: 2, artists: 3 } as const;
  const cols = mg.overview.collections.filter((c) => mg.picked[c.id]).sort((a, b) => order[a.kind] - order[b.kind]);
  const selected = cols.map((c) => c.id);
  if (!selected.length) return;
  const targets: Record<string, MigrateTarget> = {};
  for (const c of cols) {
    if (c.kind !== "liked" && c.kind !== "playlist") continue;
    const raw = mg.targets[c.id] ?? (c.kind === "liked" ? "liked" : "new");
    if (raw === "liked") targets[c.id] = { kind: "liked" };
    else if (raw.startsWith("pl:")) targets[c.id] = { kind: "playlist", id: raw.slice(3) };
    else {
      const custom = (mg.targetNames[c.id] ?? "").trim();
      targets[c.id] = { kind: "new_playlist", name: custom || null };
    }
  }
  mg.resume = undefined;
  try {
    await backend().call("migrate_start", { service: mg.service, selected, targets });
    app.toast(`${t("migrate.started")} · ${serviceOf(mg.service).name}`);
    mg.step = "running";
    mg.progress = null;
    watch();
    poll();
  } catch (e) { mg.error = errText(e); }
}

export async function cancel() {
  if (mg.progress) mg.progress.stopping = true;
  await backend().call("migrate_cancel");
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
