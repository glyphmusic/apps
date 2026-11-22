import { describe, expect, it, vi } from "vitest";
import { drag } from "../compact/cstate.svelte";

class FakeEl {
  parentElement: FakeEl | null = null;
  children: FakeEl[] = [];
  attrs: Record<string, string> = {};
  tag: string;
  scrollTop = 0;
  style: Record<string, string> = { touchAction: "", overflowY: "" };
  private listeners = new Map<string, Set<(e: FakeEvent) => void>>();

  constructor(tag = "div", attrs: Record<string, string> = {}) {
    this.tag = tag.toLowerCase();
    this.attrs = attrs;
  }

  append(child: FakeEl) {
    child.parentElement = this;
    this.children.push(child);
    return child;
  }

  contains(other: FakeEl | null): boolean {
    let cur = other;
    while (cur) {
      if (cur === this) return true;
      cur = cur.parentElement;
    }
    return false;
  }

  matches(selector: string): boolean {
    return selector.split(",").some((part) => {
      const s = part.trim();
      if (s === "[data-drag-own]") return "data-drag-own" in this.attrs;
      if (s === "[data-no-sheet-drag]") return "data-no-sheet-drag" in this.attrs;
      if (s === "[role='slider']") return this.attrs.role === "slider";
      if (s === "[contenteditable='true']") return this.attrs.contenteditable === "true";
      return this.tag === s;
    });
  }

  closest(selector: string): FakeEl | null {
    let cur: FakeEl | null = this;
    while (cur) {
      if (cur.matches(selector)) return cur;
      cur = cur.parentElement;
    }
    return null;
  }

  addEventListener(type: string, fn: (e: FakeEvent) => void) {
    let set = this.listeners.get(type);
    if (!set) this.listeners.set(type, (set = new Set()));
    set.add(fn);
  }

  removeEventListener(type: string, fn: (e: FakeEvent) => void) {
    this.listeners.get(type)?.delete(fn);
  }

  setPointerCapture() {}

  dispatch(e: FakeEvent) {
    let cur: FakeEl | null = this;
    while (cur && !e.stopped) {
      for (const fn of cur.listeners.get(e.type) ?? []) fn(e);
      cur = cur.parentElement;
    }
  }
}

interface FakeEvent {
  type: string;
  target: FakeEl;
  pointerId: number;
  pointerType: string;
  button: number;
  clientX: number;
  clientY: number;
  stopped?: boolean;
  defaultPrevented?: boolean;
  preventDefault(): void;
  stopPropagation(): void;
}

function ptr(type: string, target: FakeEl, x: number, y: number, id = 1): FakeEvent {
  const ev: FakeEvent = {
    type,
    target,
    pointerId: id,
    pointerType: "touch",
    button: 0,
    clientX: x,
    clientY: y,
    preventDefault() { ev.defaultPrevented = true; },
    stopPropagation() { ev.stopped = true; },
  };
  return ev;
}

describe("bottom sheet nested drag isolation", () => {
  it("does not pull the bottom sheet when dragging a [data-drag-own] reorder grip", () => {
    const list = new FakeEl("div");
    const row = list.append(new FakeEl("div"));
    const grip = row.append(new FakeEl("button", { "data-drag-own": "" }));

    const sheetMove = vi.fn();
    const rowSwipe = vi.fn();
    const dList = drag(list as unknown as HTMLElement, { axis: "y", onlyAtTop: () => true, move: sheetMove });
    const dRow = drag(row as unknown as HTMLElement, { axis: "x", move: rowSwipe });

    grip.dispatch(ptr("pointerdown", grip, 100, 100));
    grip.dispatch(ptr("pointermove", grip, 100, 140));
    grip.dispatch(ptr("pointerup", grip, 100, 140));

    expect(sheetMove).not.toHaveBeenCalled();
    expect(rowSwipe).not.toHaveBeenCalled();

    dRow.destroy();
    dList.destroy();
  });

  it("does not pull the bottom sheet when dragging a slider or typing/selecting in a textarea", () => {
    const sheet = new FakeEl("div");
    const slider = sheet.append(new FakeEl("div", { role: "slider" }));
    const textarea = sheet.append(new FakeEl("textarea"));

    const sheetMove = vi.fn();
    const dSheet = drag(sheet as unknown as HTMLElement, { axis: "y", onlyAtTop: () => true, move: sheetMove });

    slider.dispatch(ptr("pointerdown", slider, 50, 50, 2));
    slider.dispatch(ptr("pointermove", slider, 60, 90, 2));
    slider.dispatch(ptr("pointerup", slider, 60, 90, 2));

    textarea.dispatch(ptr("pointerdown", textarea, 50, 50, 3));
    textarea.dispatch(ptr("pointermove", textarea, 50, 110, 3));
    textarea.dispatch(ptr("pointerup", textarea, 50, 110, 3));

    expect(sheetMove).not.toHaveBeenCalled();
    dSheet.destroy();
  });

  it("keeps nested horizontal row swipes from pulling the vertical sheet down", () => {
    const list = new FakeEl("div");
    const row = list.append(new FakeEl("div"));

    const sheetMove = vi.fn();
    const rowSwipe = vi.fn();
    const dList = drag(list as unknown as HTMLElement, { axis: "y", onlyAtTop: () => true, move: sheetMove });
    const dRow = drag(row as unknown as HTMLElement, { axis: "x", move: rowSwipe });

    // Horizontal swipe that drifts slightly downward
    row.dispatch(ptr("pointerdown", row, 100, 100, 4));
    row.dispatch(ptr("pointermove", row, 120, 105, 4));
    row.dispatch(ptr("pointermove", row, 140, 130, 4));
    row.dispatch(ptr("pointerup", row, 140, 130, 4));

    expect(rowSwipe).toHaveBeenCalled();
    expect(sheetMove).not.toHaveBeenCalled();

    // Slightly diagonal swipe (more horizontal than vertical) must not pull sheet down
    row.dispatch(ptr("pointerdown", row, 100, 100, 5));
    row.dispatch(ptr("pointermove", row, 107, 106, 5));
    row.dispatch(ptr("pointermove", row, 120, 118, 5));
    row.dispatch(ptr("pointerup", row, 120, 118, 5));

    expect(sheetMove).not.toHaveBeenCalled();

    dRow.destroy();
    dList.destroy();
  });

  it("does not pull the sheet when a nested scrollable box is scrolled down", () => {
    const body = new FakeEl("div");
    const nested = body.append(new FakeEl("div"));
    nested.style.overflowY = "auto";
    nested.scrollTop = 48;

    const sheetMove = vi.fn();
    const dBody = drag(body as unknown as HTMLElement, { axis: "y", onlyAtTop: () => true, move: sheetMove });

    nested.dispatch(ptr("pointerdown", nested, 100, 100, 6));
    nested.dispatch(ptr("pointermove", nested, 100, 160, 6));
    nested.dispatch(ptr("pointerup", nested, 100, 160, 6));

    expect(sheetMove).not.toHaveBeenCalled();

    // Once scrolled back to top, pulling down drags the sheet normally
    nested.scrollTop = 0;
    nested.dispatch(ptr("pointerdown", nested, 100, 100, 7));
    nested.dispatch(ptr("pointermove", nested, 100, 160, 7));
    nested.dispatch(ptr("pointerup", nested, 100, 160, 7));

    expect(sheetMove).toHaveBeenCalledWith(0, 60);
    dBody.destroy();
  });
});
