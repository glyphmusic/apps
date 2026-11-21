// Demo backend handlers for the "Not the right one?" wrong-song workflow:
//   wrong_song_alternatives  { track, query? }
//   wrong_song_preview       { track, preview_url? }
//   wrong_song_preview_stop  {}
//   wrong_song_select        { original, chosen, wrong_provider?, wrong_id? }
import type { DemoCtx, DemoHandler } from "../demo";
import type {
  MatchAlternative,
  MatchAlternativesResponse,
  ProviderId,
  Track,
  WrongMatchReportResponse,
} from "../types";
import { trackKey } from "../types";
import { hash } from "../demo-data";

const PREVIEW_MAX = 30;
const THRESHOLD = 3;

const pinned = new Map<string, Track>();
const votes = new Map<string, number>();

let previewTimer = 0;
let previewKey: string | null = null;
let previewPos = 0;

function stopTimer(ctx?: DemoCtx) {
  if (previewTimer) {
    clearInterval(previewTimer);
    previewTimer = 0;
  }
  previewKey = null;
  previewPos = 0;
  ctx?.emit("wrong_song_preview", {
    key: null,
    status: "stopped",
    position: 0,
    duration: PREVIEW_MAX,
    error: null,
  });
}

function makeCandidates(track: Track, query?: string | null): MatchAlternativesResponse {
  const key = trackKey(track);
  const baseDur = track.duration_ms ?? 214_000;
  const h = hash(key);
  const ytBase = (salt: string) =>
    (hash(`${key}:${salt}`).toString(36) + "abcdefghijk").slice(0, 11);

  const cur = pinned.get(key) ?? {
    ...track,
    source: "youtube" as ProviderId,
    id: ytBase("cur"),
  };

  const candidates: MatchAlternative[] = [
    {
      track: {
        ...track,
        id: ytBase("topic"),
        source: "youtube",
        title: track.title,
        artist: `${track.artist} - Topic`,
        duration_ms: baseDur,
      },
      confidence: 0.98,
      preview_url: null,
      tag: cur.id === ytBase("topic") ? "current" : "topic",
    },
    {
      track: {
        ...track,
        id: ytBase("official"),
        source: "youtube",
        title: `${track.title} (Official Audio)`,
        artist: track.artist,
        duration_ms: baseDur + 2_000,
      },
      confidence: 0.93,
      preview_url: null,
      tag: cur.id === ytBase("official") ? "current" : "official_audio",
    },
    {
      track: {
        ...track,
        id: `au-${(h % 90000) + 10000}`,
        source: "audius",
        title: `${track.title} (Original Mix)`,
        artist: track.artist,
        duration_ms: baseDur - 1_000,
      },
      confidence: 0.87,
      preview_url: null,
      tag: cur.id === `au-${(h % 90000) + 10000}` ? "current" : null,
    },
    {
      track: {
        ...track,
        id: ytBase("cur"),
        source: "youtube",
        title: `${track.title} (Live Session / Music Video)`,
        artist: `${track.artist} Live`,
        duration_ms: baseDur + 24_000,
      },
      confidence: 0.76,
      preview_url: null,
      tag: cur.id === ytBase("cur") ? "current" : null,
    },
    {
      track: {
        ...track,
        id: `${(h % 900000) + 100000}`,
        source: "soundcloud",
        title: `${track.title} (Remastered)`,
        artist: track.artist,
        duration_ms: baseDur + 4_000,
      },
      confidence: 0.72,
      preview_url: null,
      tag: cur.id === `${(h % 900000) + 100000}` ? "current" : null,
    },
  ];

  const q = query?.trim().toLowerCase();
  const filtered = q
    ? [
        {
          track: {
            ...track,
            id: ytBase(`q:${q}`),
            source: "youtube" as ProviderId,
            title: `${track.title} (${query!.trim()})`,
            artist: track.artist,
            duration_ms: baseDur + 5_000,
          },
          confidence: 0.95,
          preview_url: null,
          tag: "official_audio",
        },
        ...candidates,
      ]
    : candidates;

  return {
    current_source: cur.source,
    current_id: cur.id,
    alternatives: filtered,
  };
}

const call = (ctx: DemoCtx, cmd: string, args: Record<string, unknown>) =>
  (ctx as unknown as { call: (c: string, a: Record<string, unknown>) => Promise<unknown> }).call(cmd, args);

export const handlers: Record<string, DemoHandler> = {
  wrong_song_alternatives: async (_, a) => {
    await new Promise((r) => setTimeout(r, 180));
    return makeCandidates(a.track as Track, a.query as string | null | undefined);
  },

  wrong_song_preview: (ctx, a) => {
    const tr = a.track as Track;
    const k = `${tr.source}:${tr.id}`;
    stopTimer();
    if (ctx.player.status === "playing") {
      ctx.player.status = "paused";
      ctx.emit("player", ctx.player);
    }
    previewKey = k;
    previewPos = 0;
    ctx.emit("wrong_song_preview", {
      key: k,
      status: "playing",
      position: 0,
      duration: PREVIEW_MAX,
      error: null,
    });
    previewTimer = window.setInterval(() => {
      previewPos = +(previewPos + 0.15).toFixed(2);
      if (previewPos >= PREVIEW_MAX) {
        stopTimer(ctx);
        return;
      }
      ctx.emit("wrong_song_preview", {
        key: previewKey,
        status: "playing",
        position: previewPos,
        duration: PREVIEW_MAX,
        error: null,
      });
    }, 150);
  },

  wrong_song_preview_stop: (ctx) => {
    stopTimer(ctx);
  },

  wrong_song_select: async (ctx, a): Promise<WrongMatchReportResponse> => {
    stopTimer(ctx);
    const original = a.original as Track;
    const chosen = a.chosen as Track;
    const key = trackKey(original);

    // 1. Save personal override locally on this client.
    pinned.set(key, chosen);

    // 2. Remove old downloaded version & immediately download the chosen version.
    ctx.library.downloads = ctx.library.downloads.filter((x) => trackKey(x) !== key);
    ctx.saveLibrary();
    void call(ctx, "download", { tracks: [{ ...original, source: chosen.source, id: chosen.id }] });

    // 3. Record vote (single vote does NOT promote globally until threshold reached).
    const voteKey = `${key}->${chosen.source}:${chosen.id}`;
    const count = (votes.get(voteKey) ?? 0) + 1;
    votes.set(voteKey, count);
    const promoted = count >= THRESHOLD;

    ctx.emit("toast", {
      text: "",
      key: promoted ? "toast.wrong_song_promoted" : "toast.wrong_song_saved",
      sub: original.title,
      kind: "ok",
    });

    return {
      recorded: true,
      promoted,
      votes: count,
      threshold: THRESHOLD,
    };
  },
};
