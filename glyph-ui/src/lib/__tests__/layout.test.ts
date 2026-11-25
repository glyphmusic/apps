import { describe, expect, it } from "vitest";
import { arrange } from "../layout.svelte";

const HOME = ["greeting", "cards", "recent", "trending"] as const;

describe("arrange", () => {
  it("keeps Glyph's order without a layout", () => {
    expect(arrange(HOME, undefined)).toEqual([...HOME]);
  });
  it("puts listed parts first and hides parts", () => {
    expect(arrange(HOME, { order: ["trending", "recent"], hide: ["greeting"] })).toEqual(["trending", "recent", "cards"]);
  });
  it("skips names it does not know and doubles", () => {
    expect(arrange(HOME, { order: ["future", "cards", "cards"], hide: ["other"] })).toEqual(["cards", "greeting", "recent", "trending"]);
  });
  it("keeps one when asked and everything is hidden", () => {
    const tabs = ["home", "search"] as const;
    expect(arrange(tabs, { hide: ["home", "search"] }, true)).toEqual(["home", "search"]);
    expect(arrange(tabs, { hide: ["home", "search"] })).toEqual([]);
  });
});
