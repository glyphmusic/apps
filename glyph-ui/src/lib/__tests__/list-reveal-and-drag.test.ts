import { describe, expect, it } from "vitest";
import { REVEAL_BAR_H, REVEAL_NO_ART_H, REVEAL_TOP_BAR, revealHeight, revealOpacity, revealSnapTarget } from "../compact/list-reveal";
import { clampDragSlot, dragShift } from "../pages/lists/list-drag";

describe("mobile list pull-to-reveal threshold", () => {
  it("computes resting heights for cover and no-art lists", () => {
    expect(revealHeight(false, false)).toBe(0);
    expect(revealHeight(true, false)).toBe(REVEAL_TOP_BAR + REVEAL_BAR_H);
    expect(revealHeight(true, true)).toBe(REVEAL_NO_ART_H);
  });

  it("snaps open past midpoint and closed below midpoint", () => {
    const h = 98;
    expect(revealSnapTarget({ scrollTop: 30, revealH: h, keepOpen: false, startedDeep: false })).toBe(0);
    expect(revealSnapTarget({ scrollTop: 70, revealH: h, keepOpen: false, startedDeep: false })).toBe(h);
    expect(revealSnapTarget({ scrollTop: 0, revealH: h, keepOpen: false, startedDeep: false })).toBeNull();
    expect(revealSnapTarget({ scrollTop: h, revealH: h, keepOpen: false, startedDeep: false })).toBeNull();
  });

  it("stops at default threshold when flinging up from deep in a list", () => {
    expect(revealSnapTarget({ scrollTop: 0, revealH: 98, keepOpen: false, startedDeep: true })).toBe(98);
    expect(revealSnapTarget({ scrollTop: 20, revealH: 98, keepOpen: false, startedDeep: true })).toBe(98);
  });

  it("keeps bar open while search is focused or active", () => {
    expect(revealSnapTarget({ scrollTop: 65, revealH: 98, keepOpen: true, startedDeep: true })).toBe(0);
    expect(revealSnapTarget({ scrollTop: 0, revealH: 98, keepOpen: true, startedDeep: false })).toBeNull();
  });

  it("fades opacity cleanly from 1 at scrollTop=0 to 0 before threshold", () => {
    expect(revealOpacity(0, 98)).toBe(1);
    expect(revealOpacity(98, 98)).toBe(0);
    expect(revealOpacity(50, 50)).toBe(0);
    expect(revealOpacity(20, 98)).toBeGreaterThan(0.5);
  });
});

describe("playlist drag-reorder math", () => {
  it("shifts rows between dragged source and target slot", () => {
    expect(dragShift(1, { from: 1, slot: 3 }, 56)).toBe(0);
    expect(dragShift(2, { from: 1, slot: 3 }, 56)).toBe(-56);
    expect(dragShift(3, { from: 1, slot: 3 }, 56)).toBe(-56);
    expect(dragShift(4, { from: 1, slot: 3 }, 56)).toBe(0);

    expect(dragShift(1, { from: 3, slot: 1 }, 56)).toBe(56);
    expect(dragShift(2, { from: 3, slot: 1 }, 56)).toBe(56);
    expect(dragShift(0, { from: 3, slot: 1 }, 56)).toBe(0);
  });

  it("clamps drag slots inside valid list bounds", () => {
    expect(clampDragSlot(10, 100, 5, 56)).toBe(0);
    expect(clampDragSlot(100 + 56 * 2 + 10, 100, 5, 56)).toBe(2);
    expect(clampDragSlot(9999, 100, 5, 56)).toBe(4);
    expect(clampDragSlot(50, 0, 0, 56)).toBe(0);
  });
});
