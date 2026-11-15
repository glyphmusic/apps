// onboarding.rs helpers and the tour's state (the `Onboarding` struct): the
// pages' toys keep what the user set while moving between pages.

/** Progress 0..1 of a step `dur` long that starts at `start` (onboarding.rs k). */
export const k = (s: number, start: number, dur: number) => Math.min(1, Math.max(0, (s - start) / dur));
export const easeOut = (x: number) => 1 - Math.pow(1 - Math.min(1, Math.max(0, x)), 3);
export const easeInOut = (x: number) => { x = Math.min(1, Math.max(0, x)); return x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2; };
/** Overshoots a little, then settles: things pop in. */
export const backOut = (x: number) => { x = Math.min(1, Math.max(0, x)) - 1; const c = 1.7; return 1 + (c + 1) * x * x * x + c * x * x; };

/** Demo sources: name, hue (0..1) of their covers, and UI icon name. */
export const SOURCES: [string, number, string][] = [
  ["Audius", 0.83, "source-audius"],
  ["Archive", 0.58, "source-archive"],
  ["Deezer", 0.75, "source-deezer"],
  ["Jamendo", 0.08, "source-jamendo"],
  ["YouTube", 0, "source-youtube"],
];
export const QUERY = "let it happen";
/** Search results: title, artist, source, cover URL. */
export const SONGS: [string, string, number, string][] = [
  ["Let It Happen", "Tame Impala", 2, "https://cdn-images.dzcdn.net/images/cover/de5b9b704cd4ec36f8bf49beb3e17ba2/500x500-000000-80-0-0.jpg"],
  ["Let It Happen", "Tame Impala", 4, "https://cdn-images.dzcdn.net/images/cover/de5b9b704cd4ec36f8bf49beb3e17ba2/500x500-000000-80-0-0.jpg"],
  ["Let It Happen (Soulwax Remix)", "Tame Impala", 0, "https://cdn-images.dzcdn.net/images/cover/de5b9b704cd4ec36f8bf49beb3e17ba2/500x500-000000-80-0-0.jpg"],
  ["Let It Happen (Live)", "Tame Impala", 1, "https://cdn-images.dzcdn.net/images/cover/d8eb61bd4becf79a602a75b69eebde7d/500x500-000000-80-0-0.jpg"],
  ["The Less I Know The Better", "Tame Impala", 2, "https://cdn-images.dzcdn.net/images/cover/de5b9b704cd4ec36f8bf49beb3e17ba2/500x500-000000-80-0-0.jpg"],
  ["Borderline", "Tame Impala", 3, "https://cdn-images.dzcdn.net/images/cover/d8eb61bd4becf79a602a75b69eebde7d/500x500-000000-80-0-0.jpg"],
];
/** Library rows: title, artist, cover hue, cover URL. */
export const LIB_SONGS: [string, string, number, string][] = [
  ["Let It Happen", "Tame Impala", 0.83, "https://cdn-images.dzcdn.net/images/cover/de5b9b704cd4ec36f8bf49beb3e17ba2/500x500-000000-80-0-0.jpg"],
  ["Feel Good Inc.", "Gorillaz", 0.08, "https://cdn-images.dzcdn.net/images/cover/3dc29a565149240729afc08e1f251b46/500x500-000000-80-0-0.jpg"],
  ["Chamber Of Reflection", "Mac DeMarco", 0.55, "https://cdn-images.dzcdn.net/images/cover/fc8f82cf0eba7408386e365e538df8b2/500x500-000000-80-0-0.jpg"],
];
export const MOSAIC = [0.83, 0.08, 0.55, 0.3];
export const MOSAIC_COVERS = [
  "https://cdn-images.dzcdn.net/images/cover/de5b9b704cd4ec36f8bf49beb3e17ba2/500x500-000000-80-0-0.jpg",
  "https://cdn-images.dzcdn.net/images/cover/3dc29a565149240729afc08e1f251b46/500x500-000000-80-0-0.jpg",
  "https://cdn-images.dzcdn.net/images/cover/fc8f82cf0eba7408386e365e538df8b2/500x500-000000-80-0-0.jpg",
  "https://cdn-images.dzcdn.net/images/cover/751372bcbd63a38e6ec6ef8bd448d687/500x500-000000-80-0-0.jpg",
];
export const SHARE_CODE = "glyph:playlist:7Kq2mX";
export const BANDS = ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
export const BLOCKS = 10;
export const PRESETS: [string, number[]][] = [
  ["eq.preset.flat", Array(10).fill(0.5)],
  ["eq.preset.bass_boost", [0.95, 0.85, 0.72, 0.6, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5]],
  ["eq.preset.vocal", [0.35, 0.4, 0.45, 0.6, 0.75, 0.82, 0.72, 0.6, 0.5, 0.42]],
  ["eq.preset.electronic", [0.85, 0.75, 0.55, 0.42, 0.38, 0.5, 0.62, 0.72, 0.82, 0.88]],
];
/** Lyrics lines (their widths) and how long each one is sung. */
export const LINES = [0.62, 0.84, 0.5, 0.92, 0.7, 0.46, 0.8, 0.66, 0.9, 0.56];
export const LINE_S = 1.7;
export const DEVICES = ["laptop", "smartphone", "tv", "speaker"];

export const ob = $state({
  /** Search: sources switched on (bits of SOURCES); changed: results come in again. */
  sources: 0b11111,
  resultsAt: null as number | null,
  /** Library: liked rows (bits), copy pressed. */
  likes: 0b100,
  autoLiked: false,
  copiedAt: 0,
  /** Sound: band levels 0..1 (target), the preset picked. */
  eq: [...PRESETS[3][1]],
  preset: 3 as number | null,
  /** Lyrics: song time = base + time since `at`, or where it was paused. */
  lyricsBase: LINE_S * 3,
  lyricsAt: 0,
  lyricsPaused: null as number | null,
  /** Devices: the one playing, picked when. */
  device: 0,
  deviceAt: 0,
});

export function resetTour(now: number) {
  ob.sources = 0b11111; ob.resultsAt = null; ob.likes = 0b100; ob.autoLiked = false; ob.copiedAt = 0;
  ob.eq = [...PRESETS[3][1]]; ob.preset = 3; ob.lyricsBase = LINE_S * 3; ob.lyricsAt = now; ob.lyricsPaused = null; ob.device = 0; ob.deviceAt = 0;
}

export function songTime(now: number): number {
  return ob.lyricsPaused ?? ob.lyricsBase + (now - ob.lyricsAt) / 1000;
}
