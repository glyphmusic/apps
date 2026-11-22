// Demo backend handlers for music video search and playback:
//   video_search   { track, query? }
//   video_resolve  { id }
import type { DemoHandler } from "../demo";
import type {
  Track,
  VideoCandidate,
  VideoKind,
  VideoResolveResponse,
  VideoSearchResponse,
} from "../types";
import { trackKey } from "../types";
import { hash } from "../demo-data";

const catalog = new Map<string, VideoCandidate>();

function makeId(seed: string): string {
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
  let h1 = 0x811c9dc5;
  let h2 = 0x9e3779b1;
  for (let i = 0; i < seed.length; i++) {
    const c = seed.charCodeAt(i);
    h1 = Math.imul(h1 ^ c, 0x01000193) >>> 0;
    h2 = Math.imul(h2 ^ (c + i * 31), 0x85ebca6b) >>> 0;
  }
  let out = "";
  for (let i = 0; i < 11; i++) {
    h1 = Math.imul(h1 ^ (h2 >>> (i % 13)) ^ (i * 0x45d9f3b), 0x27d4eb2d) >>> 0;
    h2 = Math.imul(h2 ^ (h1 >>> 7), 0x165667b1) >>> 0;
    out += chars[(h1 ^ h2) % chars.length];
  }
  return out;
}

function escapeXml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

/** Crisp 16:9 widescreen thumbnail SVG tailored to each track & video style. */
function demoThumb(seed: string, kind: VideoKind, title: string, artist: string): string {
  const h = hash(seed);
  const hueA = h % 360;
  const hueB = (hueA + (kind === "live" ? 38 : kind === "lyric" ? 160 : 54)) % 360;
  const sat = kind === "topic" ? 32 : 76;
  const shortTitle = escapeXml(title.length > 26 ? `${title.slice(0, 25)}…` : title);
  const shortArtist = escapeXml(artist.length > 24 ? `${artist.slice(0, 23)}…` : artist);

  const motif =
    kind === "live"
      ? `<polygon points="90,0 140,0 215,180 45,180" fill="hsla(${hueB},90%,70%,0.22)" />
         <polygon points="210,0 255,0 305,180 140,180" fill="hsla(${hueA},90%,68%,0.18)" />
         <circle cx="160" cy="112" r="24" fill="rgba(255,255,255,0.08)" />
         <rect x="157" y="96" width="6" height="32" rx="3" fill="rgba(255,255,255,0.45)" />`
      : kind === "lyric"
        ? `<text x="24" y="76" fill="rgba(255,255,255,0.34)" font-family="Inter,system-ui,sans-serif" font-size="14" font-weight="600">${shortArtist}</text>
           <text x="24" y="106" fill="rgba(255,255,255,0.92)" font-family="Inter,system-ui,sans-serif" font-size="18" font-weight="700">${shortTitle}</text>
           <rect x="24" y="122" width="110" height="4" rx="2" fill="hsla(${hueA},85%,68%,0.75)" />`
        : kind === "topic"
          ? `<rect x="116" y="46" width="88" height="88" rx="8" fill="hsl(${hueA},${sat}%,32%)" />
             <circle cx="160" cy="90" r="24" fill="rgba(255,255,255,0.14)" />
             <circle cx="160" cy="90" r="7" fill="rgba(0,0,0,0.45)" />`
          : `<circle cx="96" cy="74" r="76" fill="hsla(${hueA},85%,62%,0.36)" />
             <circle cx="242" cy="116" r="88" fill="hsla(${hueB},82%,56%,0.32)" />
             <rect x="0" y="0" width="320" height="16" fill="rgba(0,0,0,0.58)" />
             <rect x="0" y="164" width="320" height="16" fill="rgba(0,0,0,0.58)" />
             <text x="20" y="148" fill="rgba(255,255,255,0.84)" font-family="Inter,system-ui,sans-serif" font-size="12" font-weight="600" letter-spacing="0.06em">${shortArtist.toUpperCase()}</text>`;

  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 180">
    <defs>
      <linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0%" stop-color="hsl(${hueA},${sat}%,16%)"/>
        <stop offset="100%" stop-color="hsl(${hueB},${Math.max(24, sat - 18)}%,9%)"/>
      </linearGradient>
    </defs>
    <rect width="320" height="180" fill="url(#g)"/>
    ${motif}
  </svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

function buildDemoVideos(track: Track, query?: string | null): VideoSearchResponse {
  const key = trackKey(track);
  const baseDur = track.duration_ms ?? 218_000;
  const cleanArtist = track.artist.trim() || "Artist";
  const cleanTitle = track.title.trim() || "Track";
  const q = query?.trim() ?? "";

  const base: VideoCandidate[] = [
    {
      id: makeId(`${key}:official`),
      title: `${cleanArtist} - ${cleanTitle} (Official Music Video)`,
      channel: cleanArtist,
      duration_ms: baseDur + 18_000,
      thumbnail: demoThumb(`${key}:official`, "official", cleanTitle, cleanArtist),
      kind: "official",
    },
    {
      id: makeId(`${key}:live`),
      title: `${cleanArtist} - ${cleanTitle} (Live Session)`,
      channel: "COLORS Studio",
      duration_ms: baseDur + 44_000,
      thumbnail: demoThumb(`${key}:live`, "live", cleanTitle, cleanArtist),
      kind: "live",
    },
    {
      id: makeId(`${key}:lyric`),
      title: `${cleanArtist} - ${cleanTitle} (Official Lyric Video)`,
      channel: cleanArtist,
      duration_ms: baseDur + 4_000,
      thumbnail: demoThumb(`${key}:lyric`, "lyric", cleanTitle, cleanArtist),
      kind: "lyric",
    },
    {
      id: makeId(`${key}:acoustic`),
      title: `${cleanArtist} - ${cleanTitle} (Acoustic Live at Studio 4)`,
      channel: "BBC Radio 1 Live Lounge",
      duration_ms: baseDur + 26_000,
      thumbnail: demoThumb(`${key}:acoustic`, "live", `${cleanTitle} Acoustic`, cleanArtist),
      kind: "live",
    },
    {
      id: makeId(`${key}:cut`),
      title: `${cleanArtist} - ${cleanTitle} (Director's Cut)`,
      channel: `${cleanArtist.replace(/\s+/g, "")}VEVO`,
      duration_ms: baseDur + 62_000,
      thumbnail: demoThumb(`${key}:cut`, "video", `${cleanTitle} Cut`, cleanArtist),
      kind: "video",
    },
    {
      id: makeId(`${key}:topic`),
      title: cleanTitle,
      channel: `${cleanArtist} - Topic`,
      duration_ms: baseDur,
      thumbnail: demoThumb(`${key}:topic`, "topic", cleanTitle, cleanArtist),
      kind: "topic",
    },
  ];

  let results = base;
  if (q) {
    const low = q.toLowerCase();
    const kind: VideoKind = low.includes("live") || low.includes("acoustic") || low.includes("concert")
      ? "live"
      : low.includes("lyric")
        ? "lyric"
        : low.includes("official")
          ? "official"
          : "video";
    const custom: VideoCandidate = {
      id: makeId(`${key}:q:${low}`),
      title: `${cleanArtist} - ${cleanTitle} (${q})`,
      channel: kind === "live" ? "Live Stage Archive" : cleanArtist,
      duration_ms: baseDur + 31_000,
      thumbnail: demoThumb(`${key}:q:${low}`, kind, `${cleanTitle} · ${q}`, cleanArtist),
      kind,
    };
    const matching = base.filter(
      (v) => v.title.toLowerCase().includes(low) || v.channel.toLowerCase().includes(low) || v.kind === kind,
    );
    const rest = base.filter((v) => !matching.includes(v));
    results = [custom, ...matching, ...rest];
  }

  for (const item of results) catalog.set(item.id, item);

  return {
    query: q || `${cleanArtist} ${cleanTitle} official music video`,
    results,
  };
}

export const handlers: Record<string, DemoHandler> = {
  video_search: (_ctx, args) => {
    const track = args.track as Track;
    const query = (args.query as string | null | undefined) ?? null;
    return buildDemoVideos(track, query);
  },

  video_resolve: (_ctx, args): VideoResolveResponse => {
    const id = String(args.id ?? "");
    const item = catalog.get(id);
    const dur = item?.duration_ms ?? 236_000;
    const kind = item?.kind ?? "official";
    const mkUrl = (q: string) => `demo://video/${encodeURIComponent(id)}?q=${q}&kind=${kind}`;
    return {
      id,
      title: item?.title ?? "Music Video",
      channel: item?.channel ?? "Glyph Demo",
      duration_ms: dur,
      url: mkUrl("720p"),
      audio_url: null,
      mime: "video/mp4",
      qualities: [
        { label: "1080p", height: 1080, url: mkUrl("1080p"), audio_url: null, mime: "video/mp4" },
        { label: "720p", height: 720, url: mkUrl("720p"), audio_url: null, mime: "video/mp4" },
        { label: "360p", height: 360, url: mkUrl("360p"), audio_url: null, mime: "video/mp4" },
      ],
    };
  },
};
