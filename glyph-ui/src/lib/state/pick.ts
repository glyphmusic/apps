// Pure multi-select helpers for track lists (toggle, Shift+Click range, select all).
import { trackKey, type Track } from "../api/types";

export function togglePickedKey(keys: readonly string[], key: string): string[] {
  return keys.includes(key) ? keys.filter((x) => x !== key) : [...keys, key];
}

/**
 * Adds (or clears if `select === false`) the inclusive range `[anchor, target]`
 * from `tracks` into `existingKeys`, preserving order and deduplicating keys.
 */
export function pickRangeKeys(
  tracks: readonly Track[],
  existingKeys: readonly string[],
  anchorIndex: number,
  targetIndex: number,
  select = true,
): string[] {
  if (!tracks.length) return [...existingKeys];
  const lo = Math.max(0, Math.min(anchorIndex, targetIndex, tracks.length - 1));
  const hi = Math.min(tracks.length - 1, Math.max(anchorIndex, targetIndex, 0));
  const rangeKeys = new Set<string>();
  for (let i = lo; i <= hi; i++) rangeKeys.add(trackKey(tracks[i]));

  if (!select) {
    return existingKeys.filter((k) => !rangeKeys.has(k));
  }
  const seen = new Set(existingKeys);
  const out = [...existingKeys];
  for (const k of rangeKeys) {
    if (!seen.has(k)) {
      seen.add(k);
      out.push(k);
    }
  }
  return out;
}

export function toggleAllKeys(tracks: readonly Track[], existingKeys: readonly string[]): string[] {
  const set = new Set(existingKeys);
  const all = tracks.length > 0 && tracks.every((x) => set.has(trackKey(x)));
  return all ? [] : [...new Set(tracks.map(trackKey))];
}

export function selectedTracksOf(tracks: readonly Track[], keys: readonly string[]): Track[] {
  if (!keys.length) return [];
  const set = new Set(keys);
  return tracks.filter((x) => set.has(trackKey(x)));
}
