// Demo handlers for the Home / Search / Now Playing / queue screens.
//
// New backend commands (the Rust / Tauri side implements them):
//   add_suggestions { id: string }                 -> Track[]
//       "Add songs" panel, tab Suggested: songs similar to the playlist's own
//       (a few spread over it, merged by score, max 3 per artist, max 50,
//       minus what is in the playlist; liked songs seed an empty playlist;
//       Home feeds when there is nothing to go on). app/menus.rs load_add_suggestions.
//   lyrics_alternatives { track: Track }           -> Lyrics[]
//       Other versions of the song's lyrics (b.lyrics_candidates).
//   lyrics_pick { track: Track, lyrics: Lyrics }   -> void
//       Keeps the chosen version for this song (b.pin_lyrics).
//   lyrics_offset { track: Track, offset_ms: number } -> void
//       Lyrics timing offset of the song, saved in the library (ms; a line is
//       active when position + offset >= its time).
//   set_radio { on: boolean }                      -> void
//       Queue panel switch "continue with similar songs".
//   set_sleep { mins: number | null, end_of_song: boolean } -> void
//       Sleep timer (app/sleep.rs): pause after `mins` minutes or at the end
//       of the song; null / false = off. Also fades the volume over the last
//       30 s. The UI keeps `app.sleep` itself and clears it when it runs out.
import type { DemoHandler } from "../demo";
import type { Lyrics, Track } from "../types";
import { TRACKS, byArtist, demoLyrics } from "../demo-data";
import { trackKey } from "../types";

const version = (t: Track, source: string, score: number, synced: boolean, shift: number): Lyrics => {
  const lines = demoLyrics(t).map((l) => ({ time_ms: synced ? l.time_ms + shift : null, text: l.text }));
  return { synced, lines, score, source, offset_ms: 0 };
};

export const handlers: Record<string, DemoHandler> = {
  add_suggestions: (ctx, a) => {
    const pl = ctx.library.playlists.find((p) => p.id === a.id);
    const own = pl?.tracks ?? [];
    const have = new Set(own.map(trackKey));
    const pool = own.length ? own : ctx.library.liked;
    const artists = [...new Set(pool.map((t) => t.artist))];
    const found = artists.flatMap((ar) => TRACKS.filter((t) => t.artist !== ar && (t.album ?? "").length % 2 === ar.length % 2)).filter((t) => !have.has(trackKey(t)));
    const per: Record<string, number> = {};
    return [...new Map(found.map((t) => [trackKey(t), t])).values()].filter((t) => (per[t.artist] = (per[t.artist] ?? 0) + 1) <= 3).slice(0, 50)
      .concat(byArtist("Tame Impala").filter((t) => !have.has(trackKey(t)))).slice(0, 50);
  },
  lyrics_alternatives: (_, a) => {
    const t = a.track as Track;
    return [version(t, "LRCLIB", 0.96, true, 0), version(t, "Musixmatch", 0.84, true, 500), version(t, "Genius", 0.65, false, 0)];
  },
  lyrics_pick: () => undefined,
  lyrics_offset: () => undefined,
  set_radio: () => undefined,
  set_sleep: () => undefined,
};
