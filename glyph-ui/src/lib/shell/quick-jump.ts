// Instant local search & command-palette matcher for SearchField (Ctrl+K).
import { trackKey, type ListKey, type Playlist, type Track } from "../api/types";

export type QuickJumpPage = {
  id: string;
  label: string;
  keywords?: string[];
  icon: string;
};

export type QuickJumpItem =
  | { kind: "list"; list: ListKey; title: string; sub: string; score: number }
  | { kind: "artist"; name: string; track: Track | null; score: number }
  | { kind: "song"; track: Track; score: number }
  | { kind: "page"; id: string; label: string; icon: string; score: number };

export interface QuickJumpInput {
  query: string;
  playlists: readonly Playlist[];
  liked: readonly Track[];
  savedAlbums: readonly ListKey[];
  followed: readonly string[];
  builtInLists: readonly { list: ListKey; title: string; count: number; sub: string }[];
  pages: readonly QuickJumpPage[];
  limit?: number;
}

/**
 * Scores how well `needle` (already trimmed & lowercased) matches `haystack`.
 * Returns `0` for no match; higher scores indicate tighter matches.
 */
export function scoreMatch(haystack: string, needle: string): number {
  if (!needle || !haystack) return 0;
  const h = haystack.toLowerCase();
  if (h === needle) return 120;
  if (h.startsWith(needle)) return 95;
  // Word-start match (e.g. "morning" in "Sunday Morning")
  const idx = h.indexOf(needle);
  if (idx < 0) return 0;
  if (idx > 0 && /[\s\-_/(·]/.test(h[idx - 1])) return 75;
  return 50;
}

/**
 * Builds ranked instant quick-jump results across the user's library and app pages.
 * Pure & fast: safe to compute on every keystroke.
 */
export function buildQuickJump(input: QuickJumpInput): QuickJumpItem[] {
  const q = input.query.trim().toLowerCase();
  if (!q) return [];
  const limit = input.limit ?? 8;
  const results: QuickJumpItem[] = [];
  const seenArtists = new Set<string>();
  const seenSongs = new Set<string>();

  // 1. User playlists (+ slight boost so jumping to your own playlists feels instant)
  for (const p of input.playlists) {
    const s = scoreMatch(p.name, q);
    if (s > 0) {
      results.push({
        kind: "list",
        list: { kind: "playlist", id: p.id },
        title: p.name,
        sub: String(p.tracks.length),
        score: s + 12,
      });
    }
  }

  // 2. Built-in lists (Liked Songs, Downloads, Recently Played)
  for (const b of input.builtInLists) {
    const s = scoreMatch(b.title, q);
    if (s > 0) {
      results.push({
        kind: "list",
        list: b.list,
        title: b.title,
        sub: b.sub,
        score: s + 10,
      });
    }
  }

  // 3. Saved albums
  for (const al of input.savedAlbums) {
    if (al.kind !== "album") continue;
    const sTitle = scoreMatch(al.title, q);
    const sArtist = scoreMatch(al.artist, q) * 0.8;
    const s = Math.max(sTitle, sArtist);
    if (s > 0) {
      results.push({
        kind: "list",
        list: al,
        title: al.title,
        sub: al.artist,
        score: s + 6,
      });
    }
  }

  // 4. Followed artists
  for (const name of input.followed) {
    const k = name.toLowerCase();
    if (seenArtists.has(k)) continue;
    const s = scoreMatch(name, q);
    if (s > 0) {
      seenArtists.add(k);
      const sample = input.liked.find((tr) => tr.artist.toLowerCase().includes(k)) ?? null;
      results.push({ kind: "artist", name, track: sample, score: s + 8 });
    }
  }

  // 5. Pages / commands (Home, Library, Settings, Transfer, Stats, Local)
  for (const pg of input.pages) {
    let best = scoreMatch(pg.label, q);
    if (pg.keywords) {
      for (const kw of pg.keywords) best = Math.max(best, scoreMatch(kw, q) * 0.9);
    }
    if (best > 0) {
      results.push({ kind: "page", id: pg.id, label: pg.label, icon: pg.icon, score: best + 4 });
    }
  }

  // 6. Local library songs (liked first, then playlist tracks), deduplicated
  const considerTrack = (tr: Track, boost: number) => {
    const k = trackKey(tr);
    if (seenSongs.has(k)) return;
    const st = scoreMatch(tr.title, q);
    const sa = scoreMatch(tr.artist, q) * 0.85;
    const sal = tr.album ? scoreMatch(tr.album, q) * 0.65 : 0;
    const best = Math.max(st, sa, sal);
    if (best > 0) {
      seenSongs.add(k);
      results.push({ kind: "song", track: tr, score: best + boost });
    }
  };
  for (const tr of input.liked) considerTrack(tr, 4);
  for (const p of input.playlists) {
    for (const tr of p.tracks) considerTrack(tr, 0);
  }

  results.sort((a, b) => b.score - a.score);
  return results.slice(0, limit);
}
