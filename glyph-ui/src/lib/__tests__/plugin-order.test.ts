import { describe, expect, it } from "vitest";
import { preferOrder, sortLikeGlyph } from "../settings/plugin-order";

const p = (id: string, name = id) => ({ id, name });

describe("plugin order (who wins a conflict)", () => {
  it("goes by name until the user picks", () => {
    expect(sortLikeGlyph([p("sakura", "Sakura"), p("aurora", "Aurora")], []).map((x) => x.id)).toEqual(["aurora", "sakura"]);
  });
  it("puts picked plugins first, the rest after them by name", () => {
    const v = [p("aurora"), p("berry"), p("citrus"), p("sakura")];
    expect(sortLikeGlyph(v, ["sakura", "aurora"]).map((x) => x.id)).toEqual(["sakura", "aurora", "berry", "citrus"]);
  });
  it("moves the picked plugin right before the other one", () => {
    expect(preferOrder(["aurora", "citrus", "sakura"], "sakura", "aurora")).toEqual(["sakura", "aurora", "citrus"]);
    expect(preferOrder(["sakura", "aurora", "citrus"], "aurora", "sakura")).toEqual(["aurora", "sakura", "citrus"]);
  });
});
