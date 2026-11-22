import { describe, expect, it, vi } from "vitest";
import { FEEDBACK_ISSUES_URL, buildFeedbackIssueUrl, createShakeDetector } from "../feedback";
import { SHORTCUTS } from "../keys";

describe("feedback issue URL builder", () => {
  it("returns the base GitHub issues URL when message is empty", () => {
    expect(buildFeedbackIssueUrl("")).toBe(FEEDBACK_ISSUES_URL);
    expect(buildFeedbackIssueUrl("   \n\t  ")).toBe(FEEDBACK_ISSUES_URL);
  });

  it("pre-fills title and body with user feedback and metadata", () => {
    const url = new URL(
      buildFeedbackIssueUrl("Queue sheet drags while reordering\nHappens on Android", {
        version: "0.13.1",
        platform: "android",
      }),
    );
    expect(url.origin + url.pathname).toBe(FEEDBACK_ISSUES_URL);
    expect(url.searchParams.get("title")).toBe("Queue sheet drags while reordering");
    expect(url.searchParams.get("body")).toBe(
      "Queue sheet drags while reordering\nHappens on Android\n\n---\nGlyph v0.13.1 · android",
    );
  });

  it("truncates long single-line feedback for the issue title while keeping full body", () => {
    const longMsg = "A".repeat(120);
    const url = new URL(buildFeedbackIssueUrl(longMsg));
    expect(url.searchParams.get("title")?.length).toBe(72);
    expect(url.searchParams.get("title")?.endsWith("…")).toBe(true);
    expect(url.searchParams.get("body")).toBe(longMsg);
  });

  it("registers the desktop feedback keyboard shortcut", () => {
    const sc = SHORTCUTS.find((s) => s.id === "feedback");
    expect(sc).toBeDefined();
    expect(sc?.key).toBe("secondary-shift-f");
    expect(sc?.scope).toBe("global");
  });
});

describe("shake detector", () => {
  it("ignores resting gravity and single bumps", () => {
    const onShake = vi.fn();
    const d = createShakeDetector(onShake);

    // Resting 1g
    expect(d.sample(0, 0, 9.81, 1000)).toBe(false);
    expect(d.sample(0, 9.81, 0, 1200)).toBe(false);
    // Single bump (~3g)
    expect(d.sample(28, 0, 0, 1400)).toBe(false);
    expect(d.sample(0, 0, 9.81, 1600)).toBe(false);
    expect(onShake).not.toHaveBeenCalled();
  });

  it("fires once after 3 sharp shake peaks within the window and respects cooldown", () => {
    const onShake = vi.fn();
    const d = createShakeDetector(onShake);

    expect(d.sample(26, 0, 0, 1000)).toBe(false);
    // Too close to previous peak (< 110ms) -> ignored
    expect(d.sample(-26, 0, 0, 1050)).toBe(false);
    // Second peak
    expect(d.sample(-26, 0, 0, 1180)).toBe(false);
    // Third peak -> triggers!
    expect(d.sample(27, 0, 0, 1340)).toBe(true);
    expect(onShake).toHaveBeenCalledTimes(1);

    // Additional peaks inside cooldown should not retrigger
    expect(d.sample(28, 0, 0, 1600)).toBe(false);
    expect(d.sample(-28, 0, 0, 1800)).toBe(false);
    expect(d.sample(28, 0, 0, 2000)).toBe(false);
    expect(onShake).toHaveBeenCalledTimes(1);

    // After cooldown (> 2500ms after 1340 = 3840ms) a fresh shake triggers again
    expect(d.sample(28, 0, 0, 4000)).toBe(false);
    expect(d.sample(-28, 0, 0, 4150)).toBe(false);
    expect(d.sample(28, 0, 0, 4300)).toBe(true);
    expect(onShake).toHaveBeenCalledTimes(2);
  });

  it("expires old peaks outside the sliding window", () => {
    const onShake = vi.fn();
    const d = createShakeDetector(onShake);

    expect(d.sample(26, 0, 0, 1000)).toBe(false);
    expect(d.sample(-26, 0, 0, 1200)).toBe(false);
    // 1.5s later: first two peaks expired (> 900ms)
    expect(d.sample(26, 0, 0, 2700)).toBe(false);
    expect(onShake).not.toHaveBeenCalled();
  });

  it("works with gravity-excluded linear acceleration samples", () => {
    const onShake = vi.fn();
    const d = createShakeDetector(onShake);

    expect(d.sample(15, 0, 0, 1000, false)).toBe(false);
    expect(d.sample(-15, 0, 0, 1150, false)).toBe(false);
    expect(d.sample(15, 0, 0, 1300, false)).toBe(true);
    expect(onShake).toHaveBeenCalledTimes(1);
  });
});
