import { rating, type Config, type ProviderId, type Track } from "../api/types";

export type ProvidersConfig = Config["providers"];

/** Playback sources the browser player can resolve and stream directly. */
export const WEB_SOURCES = new Set<ProviderId>(["audius", "jamendo", "archive", "youtube"]);

/** Playable full-track sources in fallback default order (including ones missing from older configs). */
export const FULL_SOURCES: ProviderId[] = ["youtube", "audius", "jamendo", "archive", "soundcloud"];

/** Catalog sources that can contribute search results. */
export const SEARCH_SOURCES: ProviderId[] = ["youtube", "deezer", "audius", "jamendo", "archive", "soundcloud"];

/** Sources allowed on the server/local search endpoints (matched by ProviderId). */
const SEARCHABLE = new Set<ProviderId>(SEARCH_SOURCES);

/** Enabled full-track sources in configured priority order (omitting web-unsupported ones in the browser). */
export function activeSources(pv: ProvidersConfig, web: boolean): ProviderId[] {
  const enabled = new Set(pv.enabled);
  const ordered = [
    ...pv.priority,
    ...FULL_SOURCES.filter((p) => !pv.priority.includes(p)),
  ];
  return ordered.filter((p) => enabled.has(p) && FULL_SOURCES.includes(p) && (!web || WEB_SOURCES.has(p)));
}

/** Enabled catalog search sources (includes Deezer even when it is preview-only). */
export function activeSearchSources(pv: ProvidersConfig, web: boolean): ProviderId[] {
  const enabled = new Set(pv.enabled);
  const ordered = [
    ...pv.priority.filter((p) => SEARCHABLE.has(p)),
    ...SEARCH_SOURCES.filter((p) => !pv.priority.includes(p)),
  ];
  return ordered.filter((p) => enabled.has(p) && (!web || WEB_SOURCES.has(p) || p === "deezer"));
}

/**
 * Filter a server/cached track against the enabled providers list.
 * Drops disabled IDs from `external_ids` and promotes an enabled searchable
 * source (for example a merged YouTube match when Deezer is disabled) before
 * returning null if nothing enabled remains.
 */
export function filterTrackByEnabled(track: Track, enabled: readonly ProviderId[]): Track | null {
  const allow = new Set(enabled);
  const keptRating = rating(track);
  const external_ids: Partial<Record<ProviderId, string>> = {};
  for (const [k, v] of Object.entries(track.external_ids ?? {}) as [ProviderId, string | undefined][]) {
    if (v && allow.has(k) && SEARCHABLE.has(k)) external_ids[k] = v;
  }
  const withRating = (out: Track): Track =>
    rating(out) === keptRating ? out : { ...out, explicit: keptRating === "explicit", explicit_rating: keptRating };
  if (allow.has(track.source)) {
    return withRating({ ...track, external_ids });
  }
  for (const p of SEARCH_SOURCES) {
    const id = external_ids[p];
    if (!id || !allow.has(p)) continue;
    return withRating({
      ...track,
      id,
      source: p,
      permalink: p === "youtube" ? `https://www.youtube.com/watch?v=${id}` : undefined,
      external_ids,
    });
  }
  return null;
}

/** Human-readable label for how YouTube streams are resolved right now. */
export function youtubeRouteLabel(pv: ProvidersConfig, web: boolean, touch: boolean): string {
  if (web || pv.youtube_mode === "server") return "Server";
  if (!touch && pv.extractor === "yt_dlp") return "yt-dlp";
  return "r-dlp";
}
