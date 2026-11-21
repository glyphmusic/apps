import { describe, expect, it } from "vitest";
import { eventKey, isBareShortcut, keyLabel } from "../keys";

describe("keyboard shortcut helpers", () => {
  it("normalizes keyboard events into config notation", () => {
    expect(eventKey({ key: " ", ctrlKey: false, metaKey: false, altKey: false, shiftKey: false } as KeyboardEvent)).toBe("space");
    expect(eventKey({ key: "ArrowDown", ctrlKey: false, metaKey: false, altKey: false, shiftKey: false } as KeyboardEvent)).toBe("down");
    expect(eventKey({ key: "ArrowUp", ctrlKey: false, metaKey: false, altKey: false, shiftKey: true } as KeyboardEvent)).toBe("shift-up");
    expect(eventKey({ key: "K", ctrlKey: true, metaKey: false, altKey: false, shiftKey: false } as KeyboardEvent)).toBe("secondary-k");
  });

  it("distinguishes bare single-key shortcuts from modifier/function shortcuts", () => {
    expect(isBareShortcut("space")).toBe(true);
    expect(isBareShortcut("m")).toBe(true);
    expect(isBareShortcut("secondary-k")).toBe(false);
    expect(isBareShortcut("alt-1")).toBe(false);
    expect(isBareShortcut("f5")).toBe(false);
  });

  it("formats readable key labels", () => {
    expect(keyLabel("secondary-k")).toMatch(/(Ctrl K|⌘K)/);
    expect(keyLabel("left")).toBe("←");
    expect(keyLabel("")).toBe("");
  });
});
