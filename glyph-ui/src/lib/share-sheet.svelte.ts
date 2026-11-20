// Android share sheet: "Share" or "Open with" Glyph from another app opens
// ShareActivity, a see-through window over that app whose page (`#share`)
// shows only the "Add to Glyph" sheet (overlays/AddShared.svelte). The
// sheet is up at once while the link or files are read; Done, closing or
// Back end the window and leave the other app where it was. What only the
// app can do (a jam, several links, a search) opens the app instead.
//
// Java hands over what was shared through `__GLYPH_ANDROID__.sharePayload()`
// (and `window.__glyphShare(..)` when it changes): `{ text }` or
// `{ files: [paths] }`, `{ loading: "files" }` while files are copied,
// `{ error: "files" }` when none could be copied.
import { app } from "./state.svelte";
import { backend } from "./api/backend";
import { t } from "./i18n.svelte";
import type { Track } from "./api/types";

type Payload = { text?: string; files?: string[]; loading?: string; error?: string };
type Read =
  | { kind: "songs"; tracks: Track[] }
  | { kind: "list"; name: string; tracks: Track[]; origin?: string | null; code?: string; link?: string; author?: string | null }
  | { kind: "app" }
  | { kind: "none"; error?: string };
type Bridge = { sharePayload?: () => string | null; closeSheet?: (message: string) => void; openInApp?: () => void };

const bridge = () => (window as unknown as { __GLYPH_ANDROID__?: Bridge }).__GLYPH_ANDROID__;

/** Songs found so far while a shared playlist is read. */
export const shareProgress = $state({ done: 0, total: 0 });

let last: Payload | null = null;
let seq = 0;
let closing = false;

/** A short line for what was shared: the link's site, or the file name. */
function labelOf(p: Payload): string {
  if (p.files?.length) {
    const name = p.files[0].split(/[\\/]/).pop() ?? "";
    return p.files.length > 1 ? `${name} +${p.files.length - 1}` : name;
  }
  const text = (p.text ?? "").trim();
  const link = text.split(/\s+/).find((w) => /^https?:\/\//i.test(w));
  if (link) {
    try { return new URL(link).hostname.replace(/^www\./, ""); } catch { /* not a URL */ }
  }
  return text.slice(0, 80);
}

async function take(p: Payload) {
  last = p;
  closing = false;
  const my = ++seq;
  shareProgress.done = 0;
  shareProgress.total = 0;
  app.ctx = null;
  app.pickSongs = { tracks: [], sheet: true, loading: true, files: !!(p.files?.length || p.loading === "files"), label: labelOf(p) };
  if (p.error) {
    // The shared files could not be copied (gone, no access, too big).
    app.pickSongs = { tracks: [], sheet: true, error: t("open.file_unreadable"), label: labelOf(p) };
    return;
  }
  if (p.loading || (!p.text && !p.files?.length)) return;
  let r: Read;
  try {
    r = p.files?.length
      ? await backend().call<Read>("share_read_files", { paths: p.files })
      : await backend().call<Read>("share_read", { text: p.text });
  } catch (e) {
    r = { kind: "none", error: String(e) };
  }
  if (my !== seq) return;
  if (r.kind === "app") {
    bridge()?.openInApp?.();
    return;
  }
  if (r.kind === "songs" && r.tracks.length) {
    app.pickSongs = { tracks: r.tracks, sheet: true, file: !!p.files?.length };
  } else if (r.kind === "list" && r.tracks.length) {
    app.pickSongs = {
      tracks: r.tracks,
      sheet: true,
      list: { name: r.name, origin: r.origin ?? null, code: r.code, link: r.link, author: r.author ?? null },
    };
  } else {
    const error = r.kind === "none" && r.error ? r.error : t("import.empty");
    app.pickSongs = { tracks: [], sheet: true, error, label: labelOf(p) };
  }
}

/** Starts the sheet page: reads what was shared and listens for more. */
export function startShareSheet() {
  (window as unknown as { __glyphShare?: (p: Payload) => void }).__glyphShare = (p) => void take(p);
  backend().on("share_progress", (pr) => {
    shareProgress.done = pr.done;
    shareProgress.total = pr.total;
  });
  let p: Payload | null = null;
  try {
    const raw = bridge()?.sharePayload?.();
    p = raw ? (JSON.parse(raw) as Payload) : null;
  } catch { /* nothing shared */ }
  void take(p ?? { loading: "text" });
}

export function retryShare() {
  if (last) void take(last);
}

/** Ends the share sheet window; Android says `message` once it is gone. */
export function closeShareSheet(message = "") {
  if (closing) return;
  closing = true;
  seq++;
  // Let the sheet slide out first.
  setTimeout(() => bridge()?.closeSheet?.(message), 200);
}
