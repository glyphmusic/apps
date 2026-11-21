import { describe, expect, it } from "vitest";
import { pickRangeKeys, selectedTracksOf, toggleAllKeys, togglePickedKey } from "../state/pick";
import { trackKey, type Track } from "../api/types";

const mk = (id: string, title: string, artist = "Artist"): Track => ({
  id,
  source: "deezer",
  title,
  artist,
});

describe("multi-select pick helpers", () => {
  const tracks = [mk("1", "Alpha"), mk("2", "Beta"), mk("3", "Gamma"), mk("4", "Delta"), mk("5", "Epsilon")];

  it("toggles individual keys cleanly", () => {
    const k1 = trackKey(tracks[0]);
    const added = togglePickedKey([], k1);
    expect(added).toEqual([k1]);
    expect(togglePickedKey(added, k1)).toEqual([]);
  });

  it("selects forward and backward Shift+Click ranges without duplicates", () => {
    const k1 = trackKey(tracks[1]);
    const forward = pickRangeKeys(tracks, [k1], 1, 3, true);
    expect(forward).toEqual([trackKey(tracks[1]), trackKey(tracks[2]), trackKey(tracks[3])]);

    const backward = pickRangeKeys(tracks, forward, 3, 0, true);
    expect(backward).toHaveLength(4);
    expect(new Set(backward)).toEqual(new Set([0, 1, 2, 3].map((i) => trackKey(tracks[i]))));
  });

  it("supports clearing a range when select is false", () => {
    const all = tracks.map(trackKey);
    const after = pickRangeKeys(tracks, all, 1, 3, false);
    expect(after).toEqual([trackKey(tracks[0]), trackKey(tracks[4])]);
  });

  it("toggles select-all and select-none", () => {
    const all = toggleAllKeys(tracks, [trackKey(tracks[0])]);
    expect(all).toHaveLength(5);
    expect(toggleAllKeys(tracks, all)).toEqual([]);
  });

  it("returns picked tracks preserving list order", () => {
    const picked = selectedTracksOf(tracks, [trackKey(tracks[3]), trackKey(tracks[1])]);
    expect(picked.map((t) => t.id)).toEqual(["2", "4"]);
  });
});
