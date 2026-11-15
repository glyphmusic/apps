// app/lyrics.rs for the UI: other versions of the lyrics of the playing
// song (wrong lyrics: pick the right ones; the backend keeps the choice),
// and the timing offset (ms, saved per song). Reset whenever the song changes.
import { app } from "../../state.svelte";
import { backend } from "../../api/backend";
import type { Lyrics } from "../../api/types";

export const lyr = $state<{ key: string | null; alts: Lyrics[]; loading: boolean; loaded: boolean; error: string | null }>({ key: null, alts: [], loading: false, loaded: false, error: null });

/** Starts over for the song whose lyrics are in `app.lyrics`. */
export function syncKey() {
  if (lyr.key === app.lyrics.key) return;
  lyr.key = app.lyrics.key; lyr.alts = []; lyr.loading = false; lyr.loaded = false; lyr.error = null;
}

const same = (a: Lyrics, b: Lyrics) => a.source === b.source && a.synced === b.synced && a.lines.length === b.lines.length && a.lines[0]?.text === b.lines[0]?.text;

export async function loadAlternatives() {
  const cur = app.player.current, key = app.lyrics.key;
  if (!cur || lyr.loading || lyr.loaded) return;
  lyr.loading = true;
  const r = [...((await backend().call<Lyrics[]>("lyrics_alternatives", { track: cur })) ?? []), ...((await backend().call<Lyrics[] | undefined>("plugin_lyrics", { key })) ?? [])];
  if (lyr.key !== key) return;
  const shown = app.lyrics.lyrics;
  // What is shown now stays one of the choices (first if unknown).
  lyr.alts = shown && !r.some((a) => same(a, shown)) ? [$state.snapshot(shown) as Lyrics, ...r] : r;
  lyr.loading = false; lyr.loaded = true;
}

export function altIndex(): number | null {
  const cur = app.lyrics.lyrics;
  if (!cur) return null;
  const i = lyr.alts.findIndex((a) => same(a, cur));
  return i < 0 ? null : i;
}

export function pick(i: number) {
  const l = lyr.alts[i], cur = app.player.current;
  if (!l || !cur) return;
  backend().call("lyrics_pick", { track: cur, lyrics: $state.snapshot(l) });
  app.lyrics = { ...app.lyrics, lyrics: $state.snapshot(l) as Lyrics };
  lyr.error = null;
}

export function cycle(step: number) {
  if (!lyr.loaded) return void loadAlternatives();
  const n = lyr.alts.length;
  if (n < 2) return;
  pick((((altIndex() ?? 0) + step) % n + n) % n);
}

export const offset = () => app.lyrics.lyrics?.offset_ms ?? 0;

export function setOffset(ms: number) {
  const cur = app.player.current;
  if (!cur || !app.lyrics.lyrics) return;
  app.lyrics.lyrics.offset_ms = ms;
  backend().call("lyrics_offset", { track: cur, offset_ms: ms });
}
export const shiftOffset = (d: number) => setOffset(offset() + d);
