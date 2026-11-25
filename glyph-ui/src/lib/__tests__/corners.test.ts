import { describe, expect, it } from "vitest";
import { partOf, scaleRadius } from "../corners";

describe("partOf", () => {
  it("sorts a rule by what its selector styles", () => {
    expect(partOf(".cover.svelte-x")).toBe("covers");
    expect(partOf(".menu.svelte-x button")).toBe("buttons");
    expect(partOf(".sheet.svelte-x")).toBe("dialogs");
    expect(partOf(".ctx-menu")).toBe("menus");
    expect(partOf(".search-field input")).toBe("fields");
    expect(partOf(".mini-bar")).toBe("player");
    expect(partOf(".box.svelte-x")).toBe("cards");
  });
});

describe("scaleRadius", () => {
  it("scales pixel corners", () => {
    expect(scaleRadius("10px", 1.6)).toBe("16px");
    expect(scaleRadius("6px 6px 0px 0px", 0.5)).toBe("3px 3px 0px 0px");
    expect(scaleRadius("7.5px", 2)).toBe("15px");
  });
  it("keeps round shapes round", () => {
    expect(scaleRadius("50%", 2)).toBe("50%");
    expect(scaleRadius("999px", 0)).toBe("999px");
    expect(scaleRadius("var(--radius)", 2)).toBe("var(--radius)");
  });
});

describe("cornerPx", () => {
  it("follows the look's corner scale", async () => {
    const { cornerPx } = await import("../corners");
    expect(cornerPx(6)).toBe("calc(6px * var(--radius-k, 1))");
  });
});
