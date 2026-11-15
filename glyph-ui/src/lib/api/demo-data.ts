// Sample music catalog for the demo backend (browser preview & product screenshots).
import type { ProviderId, Track } from "./types";

export interface DemoAlbumDef {
  artist: string;
  album: string;
  year: number;
  kind?: "album" | "ep" | "single";
  cover: string;
  tracks: [title: string, duration_s: number, explicit?: boolean][];
}

export const DEMO_ALBUMS: DemoAlbumDef[] = [
  {
    artist: "Tame Impala",
    album: "Currents",
    year: 2015,
    cover: "https://cdn-images.dzcdn.net/images/cover/de5b9b704cd4ec36f8bf49beb3e17ba2/500x500-000000-80-0-0.jpg",
    tracks: [
      ["Let It Happen", 467, false],
      ["The Less I Know The Better", 216, true],
      ["New Person, Same Old Mistakes", 363, false],
      ["Eventually", 319, false],
    ],
  },
  {
    artist: "Tame Impala",
    album: "The Slow Rush",
    year: 2020,
    cover: "https://cdn-images.dzcdn.net/images/cover/d8eb61bd4becf79a602a75b69eebde7d/500x500-000000-80-0-0.jpg",
    tracks: [
      ["Borderline", 238, false],
      ["One More Hour", 433, false],
      ["Breathe Deeper", 372, false],
      ["Lost In Yesterday", 250, false],
    ],
  },
  {
    artist: "Tame Impala",
    album: "Deadbeat",
    year: 2025,
    cover: "https://cdn-images.dzcdn.net/images/cover/23b006b2e956536d97612847bbd7a3b7/500x500-000000-80-0-0.jpg",
    tracks: [
      ["Loser", 223, true],
      ["Dracula", 205, true],
      ["My Old Ways", 298, true],
    ],
  },
  {
    artist: "Gorillaz",
    album: "Demon Days",
    year: 2005,
    cover: "https://cdn-images.dzcdn.net/images/cover/3dc29a565149240729afc08e1f251b46/500x500-000000-80-0-0.jpg",
    tracks: [
      ["Feel Good Inc.", 222, false],
      ["DARE", 245, false],
      ["Dirty Harry", 230, false],
    ],
  },
  {
    artist: "Gorillaz",
    album: "Plastic Beach",
    year: 2010,
    cover: "https://cdn-images.dzcdn.net/images/cover/4ddf15e6d4fa3cf61fdc8271cdec4815/500x500-000000-80-0-0.jpg",
    tracks: [
      ["On Melancholy Hill", 233, false],
      ["Rhinestone Eyes", 200, false],
      ["Empire Ants", 283, false],
    ],
  },
  {
    artist: "Gorillaz",
    album: "Gorillaz",
    year: 2001,
    cover: "https://cdn-images.dzcdn.net/images/cover/f4d581f4b86c869547704d7db9aa2c43/500x500-000000-80-0-0.jpg",
    tracks: [
      ["Clint Eastwood", 340, true],
    ],
  },
  {
    artist: "MGMT",
    album: "Oracular Spectacular",
    year: 2007,
    cover: "https://cdn-images.dzcdn.net/images/cover/751372bcbd63a38e6ec6ef8bd448d687/500x500-000000-80-0-0.jpg",
    tracks: [
      ["Kids", 303, false],
      ["Electric Feel", 229, false],
      ["Time to Pretend", 261, false],
    ],
  },
  {
    artist: "MGMT",
    album: "Little Dark Age",
    year: 2018,
    cover: "https://cdn-images.dzcdn.net/images/cover/494429a8ec9251591dea6c1f10ade166/500x500-000000-80-0-0.jpg",
    tracks: [
      ["Little Dark Age", 299, false],
      ["Me and Michael", 289, false],
    ],
  },
  {
    artist: "Mac DeMarco",
    album: "Salad Days",
    year: 2014,
    cover: "https://cdn-images.dzcdn.net/images/cover/fc8f82cf0eba7408386e365e538df8b2/500x500-000000-80-0-0.jpg",
    tracks: [
      ["Chamber Of Reflection", 232, false],
      ["Salad Days", 146, false],
      ["Blue Boy", 126, false],
      ["Passing Out Pieces", 167, false],
    ],
  },
  {
    artist: "Unknown Mortal Orchestra",
    album: "Multi-Love",
    year: 2015,
    cover: "https://cdn-images.dzcdn.net/images/cover/2fcc851af502269efbd246157dc13a86/500x500-000000-80-0-0.jpg",
    tracks: [
      ["Multi-Love", 251, false],
      ["Can't Keep Checking My Phone", 256, false],
      ["Necessary Evil", 317, false],
    ],
  },
  {
    artist: "Unknown Mortal Orchestra",
    album: "II",
    year: 2013,
    cover: "https://cdn-images.dzcdn.net/images/cover/8273734b25282b938a00b859b36bca3c/500x500-000000-80-0-0.jpg",
    tracks: [
      ["So Good at Being in Trouble", 230, false],
    ],
  },
  {
    artist: "Blood Orange",
    album: "Coastal Grooves",
    year: 2011,
    cover: "https://cdn-images.dzcdn.net/images/cover/ab9a7a8b5ee98acc02dcdd5e9d9d93cf/500x500-000000-80-0-0.jpg",
    tracks: [
      ["Champagne Coast", 292, false],
    ],
  },
  {
    artist: "Blood Orange",
    album: "Negro Swan",
    year: 2018,
    cover: "https://cdn-images.dzcdn.net/images/cover/fa5abb6f1cfd800592e740d71988d4d5/500x500-000000-80-0-0.jpg",
    tracks: [
      ["Charcoal Baby", 242, false],
    ],
  },
  {
    artist: "Men I Trust",
    album: "Oncle Jazz",
    year: 2019,
    cover: "https://cdn-images.dzcdn.net/images/cover/231cf5ffe0750987a4e1dba689c03c42/500x500-000000-80-0-0.jpg",
    tracks: [
      ["Show Me How", 215, false],
      ["Numb", 219, false],
      ["Seven", 225, false],
      ["Say, Can You Hear", 162, false],
    ],
  },
  {
    artist: "Daft Punk",
    album: "Random Access Memories",
    year: 2013,
    cover: "https://cdn-images.dzcdn.net/images/cover/311bba0fc112d15f72c8b5a65f0456c1/500x500-000000-80-0-0.jpg",
    tracks: [
      ["Instant Crush (feat. Julian Casablancas)", 337, false],
      ["Get Lucky (feat. Pharrell Williams & Nile Rodgers)", 369, false],
    ],
  },
  {
    artist: "Daft Punk",
    album: "Discovery",
    year: 2001,
    cover: "https://cdn-images.dzcdn.net/images/cover/5718f7c81c27e0b2417e2a4c45224f8a/500x500-000000-80-0-0.jpg",
    tracks: [
      ["One More Time", 320, false],
      ["Veridis Quo", 345, false],
    ],
  },
  {
    artist: "Beach House",
    album: "Depression Cherry",
    year: 2015,
    cover: "https://cdn-images.dzcdn.net/images/cover/399cffa2434760d22746a10a44c80bc2/500x500-000000-80-0-0.jpg",
    tracks: [
      ["Space Song", 320, false],
      ["PPP", 368, false],
    ],
  },
  {
    artist: "Beach House",
    album: "Bloom",
    year: 2012,
    cover: "https://cdn-images.dzcdn.net/images/cover/0616ef61572cbc0fb5d35e039e3022a3/500x500-000000-80-0-0.jpg",
    tracks: [
      ["Myth", 258, false],
      ["Lazuli", 302, false],
    ],
  },
];

export const ARTIST_META: Record<string, { photo: string; bio: string; fans: number; listeners: number; tags: string[] }> = {
  "Tame Impala": {
    photo: "https://cdn-images.dzcdn.net/images/artist/879015e713cc6ad6ffaeec154c027505/1000x1000-000000-80-0-0.jpg",
    bio: "Tame Impala is the psychedelic music project of Australian multi-instrumentalist Kevin Parker, blending analog synth textures, phaser-drenched guitars, and hypnotic disco-funk grooves.",
    fans: 1_840_000,
    listeners: 26_400_000,
    tags: ["psychedelic pop", "indie rock", "neo-psychedelia", "synth-pop"],
  },
  "Gorillaz": {
    photo: "https://cdn-images.dzcdn.net/images/artist/ebbb1c133ed0220c714b9ed5d254561f/1000x1000-000000-80-0-0.jpg",
    bio: "Created by musician Damon Albarn and artist Jamie Hewlett, Gorillaz is a virtual band revolving around 2-D, Murdoc Niccals, Noodle, and Russel Hobbs, crossing alternative rock, hip-hop, dub, and electronic pop.",
    fans: 4_620_000,
    listeners: 31_200_000,
    tags: ["alternative", "electronic", "trip hop", "art pop"],
  },
  "MGMT": {
    photo: "https://cdn-images.dzcdn.net/images/artist/0260c3e6e76812d73f50719292985f01/1000x1000-000000-80-0-0.jpg",
    bio: "Formed in Middletown, Connecticut by Andrew VanWyngarden and Ben Goldwasser, MGMT broke through with kaleidoscopic synth-pop anthems before expanding into ornate psychedelic rock and dark wave.",
    fans: 1_290_000,
    listeners: 16_800_000,
    tags: ["indie pop", "psychedelic pop", "synth-pop", "indietronica"],
  },
  "Mac DeMarco": {
    photo: "https://cdn-images.dzcdn.net/images/artist/b468329467ac01256a85e0925026715d/1000x1000-000000-80-0-0.jpg",
    bio: "Canadian singer-songwriter and multi-instrumentalist Mac DeMarco is known for his warbly chorus-pedal guitars, laid-back analog tape recordings, and intimate bedroom-pop songwriting.",
    fans: 980_000,
    listeners: 18_500_000,
    tags: ["indie rock", "jangle pop", "lo-fi", "bedroom pop"],
  },
  "Unknown Mortal Orchestra": {
    photo: "https://cdn-images.dzcdn.net/images/artist/bdb3dd11f7e503a3ba5d933b252ad91a/1000x1000-000000-80-0-0.jpg",
    bio: "Led by Ruban Nielson, Unknown Mortal Orchestra crafts gritty, lo-fi psychedelic R&B and funk-inflected indie rock built around intricate guitar work and warm analog production.",
    fans: 410_000,
    listeners: 5_200_000,
    tags: ["psychedelic rock", "indie rock", "lo-fi", "neo-psychedelia"],
  },
  "Blood Orange": {
    photo: "https://cdn-images.dzcdn.net/images/artist/63ee5c28add70150aaff4f33371ab5a5/1000x1000-000000-80-0-0.jpg",
    bio: "Blood Orange is the R&B and art-pop project of British musician, producer, and composer Devonté Hynes, weaving hazy guitars, drum machines, and lush vocal arrangements.",
    fans: 360_000,
    listeners: 7_900_000,
    tags: ["alternative r&b", "bedroom pop", "synth-pop", "indie"],
  },
  "Men I Trust": {
    photo: "https://cdn-images.dzcdn.net/images/artist/6ba5514f705f2d77a99ee62a28aa358a/1000x1000-000000-80-0-0.jpg",
    bio: "Founded in Montreal by Jessy Caron, Dragos Chiriac, and Emmanuelle Proulx, Men I Trust produces silky dream-pop and indie-funk anchored by melodic basslines and hushed vocals.",
    fans: 520_000,
    listeners: 11_400_000,
    tags: ["dream pop", "indie pop", "chillwave", "bedroom pop"],
  },
  "Daft Punk": {
    photo: "https://cdn-images.dzcdn.net/images/artist/638e69b9caaf9f9f3f8826febea7b543/1000x1000-000000-80-0-0.jpg",
    bio: "French electronic duo Guy-Manuel de Homem-Christo and Thomas Bangalter shaped modern dance and pop music across three decades, fusing French house, vocoders, disco, and live studio instrumentation.",
    fans: 6_900_000,
    listeners: 34_100_000,
    tags: ["electronic", "french house", "disco", "synth-pop"],
  },
  "Beach House": {
    photo: "https://cdn-images.dzcdn.net/images/artist/bab7f5aedc71370b598262e065d3a0ef/1000x1000-000000-80-0-0.jpg",
    bio: "Baltimore duo Victoria Legrand and Alex Scally define modern dream pop with hypnotic organ chords, slide guitar, and cinematic, reverberant arrangements.",
    fans: 670_000,
    listeners: 14_300_000,
    tags: ["dream pop", "shoegaze", "indie pop", "chamber pop"],
  },
};

export const DEMO_NEW_RELEASES = [
  {
    id: "nr-0",
    title: "Deadbeat",
    artist: "Tame Impala",
    kind: "album" as const,
    year: 2025,
    date: "2025-10-17",
    cover_url: "https://cdn-images.dzcdn.net/images/cover/23b006b2e956536d97612847bbd7a3b7/500x500-000000-80-0-0.jpg",
  },
  {
    id: "nr-1",
    title: "Dracula (with JENNIE)",
    artist: "Tame Impala",
    kind: "single" as const,
    year: 2026,
    date: "2026-02-06",
    cover_url: "https://cdn-images.dzcdn.net/images/cover/b868399da682f34dcd7d98af1c0de80b/500x500-000000-80-0-0.jpg",
  },
  {
    id: "nr-2",
    title: "Dracula",
    artist: "Tame Impala",
    kind: "single" as const,
    year: 2025,
    date: "2025-09-26",
    cover_url: "https://cdn-images.dzcdn.net/images/cover/b428fa5da7496083cd2c2e87b94e2ceb/500x500-000000-80-0-0.jpg",
  },
  {
    id: "nr-3",
    title: "Oncle Jazz",
    artist: "Men I Trust",
    kind: "album" as const,
    year: 2019,
    date: "2019-09-13",
    cover_url: "https://cdn-images.dzcdn.net/images/cover/231cf5ffe0750987a4e1dba689c03c42/500x500-000000-80-0-0.jpg",
  },
  {
    id: "nr-4",
    title: "Little Dark Age",
    artist: "MGMT",
    kind: "album" as const,
    year: 2018,
    date: "2018-02-09",
    cover_url: "https://cdn-images.dzcdn.net/images/cover/494429a8ec9251591dea6c1f10ade166/500x500-000000-80-0-0.jpg",
  },
  {
    id: "nr-5",
    title: "Negro Swan",
    artist: "Blood Orange",
    kind: "album" as const,
    year: 2018,
    date: "2018-08-24",
    cover_url: "https://cdn-images.dzcdn.net/images/cover/fa5abb6f1cfd800592e740d71988d4d5/500x500-000000-80-0-0.jpg",
  },
];

const SOURCES: ProviderId[] = ["youtube", "deezer", "audius", "jamendo", "archive"];

/** Deterministic number from a string. */
export function hash(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619) >>> 0;
  return h;
}

/** Generated SVG avatar/cover fallback for user profiles and custom playlists. */
export function demoCover(seed: string): string {
  const h = hash(seed), hue = h % 360, hue2 = (hue + 40 + (h >> 9) % 120) % 360, kind = (h >> 4) % 3;
  const shape = kind === 0
    ? `<circle cx="${30 + (h >> 3) % 40}" cy="${35 + (h >> 6) % 30}" r="${18 + (h >> 11) % 14}" fill="hsl(${hue2} 80% 70%)"/>`
    : kind === 1
      ? `<path d="M0 ${60 + (h >> 5) % 30} L100 ${20 + (h >> 7) % 40} L100 100 L0 100Z" fill="hsl(${hue2} 70% 55%)" opacity=".85"/>`
      : `<rect x="20" y="20" width="60" height="60" rx="4" fill="none" stroke="hsl(${hue2} 85% 72%)" stroke-width="5"/><rect x="34" y="34" width="32" height="32" fill="hsl(${hue2} 85% 72%)" opacity=".6"/>`;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue} 60% 22%)"/><stop offset="1" stop-color="hsl(${hue} 70% 45%)"/></linearGradient></defs><rect width="100" height="100" fill="url(#g)"/>${shape}</svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

export const TRACKS: Track[] = DEMO_ALBUMS.flatMap((alb, n) =>
  alb.tracks.map(([title, duration_s, explicit], i): Track => {
    const h = hash(alb.artist + title);
    const source: ProviderId = n < 12 ? "youtube" : SOURCES[(h >> 3) % SOURCES.length];
    return {
      id: `demo-${n}-${i}`,
      source,
      title,
      artist: alb.artist,
      album: alb.album,
      duration_ms: duration_s * 1000,
      cover_url: alb.cover,
      explicit: !!explicit,
      external_ids: source === "youtube" ? { deezer: `dz-${n}-${i}` } : {},
    };
  }),
);

export const ARTISTS = [...new Set(TRACKS.map((t) => t.artist))];

export function byArtist(a: string): Track[] {
  return TRACKS.filter((t) => t.artist.toLowerCase() === a.toLowerCase());
}

export function findAlbumDef(artist: string, title: string): DemoAlbumDef | undefined {
  return DEMO_ALBUMS.find(
    (a) => a.artist.toLowerCase() === artist.toLowerCase() && a.album.toLowerCase() === title.toLowerCase(),
  );
}

const LET_IT_HAPPEN_LYRICS: { time_ms: number; text: string }[] = [
  { time_ms: 28_460, text: "It's always around me, all this noise" },
  { time_ms: 32_930, text: "But not nearly as loud as the voice saying" },
  { time_ms: 39_750, text: "\"Let it happen, let it happen (it's gonna feel so good)" },
  { time_ms: 45_230, text: "Just let it happen, let it happen\"" },
  { time_ms: 51_440, text: "" },
  { time_ms: 57_420, text: "All this running around" },
  { time_ms: 60_940, text: "Tryin' to cover my shadow" },
  { time_ms: 64_900, text: "A notion growing inside" },
  { time_ms: 68_900, text: "Now all the others seem shallow" },
  { time_ms: 72_410, text: "All this running around" },
  { time_ms: 76_420, text: "Bearing down on my shoulders" },
  { time_ms: 80_830, text: "I can hear an alarm" },
  { time_ms: 84_590, text: "Must be a morning" },
  { time_ms: 89_150, text: "" },
  { time_ms: 104_980, text: "I heard about a whirlwind that's coming 'round" },
  { time_ms: 110_770, text: "It's gonna carry off all that isn't bound" },
  { time_ms: 115_650, text: "And when it happens, when it happens (I won't be holding on)" },
  { time_ms: 122_300, text: "So let it happen, let it happen" },
  { time_ms: 127_610, text: "" },
  { time_ms: 130_230, text: "All this running around" },
  { time_ms: 134_240, text: "I can't fight it much longer" },
  { time_ms: 137_760, text: "Something's tryin' to get out" },
  { time_ms: 141_690, text: "And it's never been closer" },
  { time_ms: 145_490, text: "If my take-off fails" },
  { time_ms: 149_000, text: "Make up some other story" },
  { time_ms: 152_990, text: "But if I never come back" },
  { time_ms: 157_160, text: "Tell my mother I'm sorry" },
  { time_ms: 161_790, text: "" },
  { time_ms: 329_240, text: "I cannot vanish, you will not scare me" },
  { time_ms: 333_090, text: "Try to get through it, try to bounce to it" },
  { time_ms: 336_490, text: "You were not thinking that I will not do it" },
  { time_ms: 340_530, text: "They be lovin' someone and I'm another story" },
  { time_ms: 344_400, text: "Take the next ticket, get the next train" },
  { time_ms: 348_260, text: "Why would I do it? Anyone'd think that" },
  { time_ms: 392_000, text: "Baby, now I'm ready, moving on" },
  { time_ms: 399_500, text: "Oh, but maybe I was ready all along" },
  { time_ms: 407_000, text: "Oh, I'm ready for the moment and the sound" },
  { time_ms: 414_500, text: "Oh, but maybe I was ready all along" },
];

/** Synced lyrics for the demo backend. */
export function demoLyrics(t: Track): { time_ms: number; text: string }[] {
  if (t.title.toLowerCase() === "let it happen") return LET_IT_HAPPEN_LYRICS;
  const lines = [
    "City lights are blurring through the window pane",
    "Static on the radio, a quiet refrain",
    "We were chasing echoes down an empty street",
    "",
    "Hold on to the rhythm while the night is young",
    "Every single word left on the tip of my tongue",
    "Let the colors wash across the ceiling slow",
    "Nowhere else to be and nowhere else to go",
    "",
    "Drifting out of focus as the morning nears",
    "Trading all our shadows for the atmosphere",
    "Turning up the dial till the speakers hum",
    "",
    "Hold on to the rhythm while the night is young",
    "Every single word left on the tip of my tongue",
    "Fading into daylight when the morning comes",
  ];
  const step = Math.max(3500, (t.duration_ms ?? 210_000) / (lines.length + 3));
  return lines.map((text, i) => ({ time_ms: Math.round(8000 + i * step), text }));
}
