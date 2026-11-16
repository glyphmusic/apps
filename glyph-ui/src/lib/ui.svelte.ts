// Small UI runtimes shared by the widgets (src/ui/widgets.rs UI_RT):
// tooltips, ASCII reveal, adaptive ink, press / pop feedback.
import type { Glow } from "./glow";
import { tipFor } from "./i18n.svelte";

// ---- tooltips (widgets/tooltip.rs) --------------------------------------
// Next to their button (below, or above near the bottom edge), never at the
// mouse. One at a time; the first waits 500 ms, the next opens at once while
// the mouse moves on within 0.8 s. A click hides it until the mouse leaves.
export const tipState = $state<{ label: string; rect: DOMRect | null; wrap: boolean; side: "below" | "above" | "right"; shown: boolean }>({ label: "", rect: null, wrap: false, side: "below", shown: false });
let tipTimer = 0;
let tipLast = 0;
let tipEl: HTMLElement | null = null;

export function tipsWarm() { return tipState.shown || performance.now() - tipLast < 800; }

function showTip(el: HTMLElement, label: string, opts: { instant?: boolean; wrap?: boolean; side?: "right" }) {
  clearTimeout(tipTimer);
  tipEl = el;
  const open = () => {
    if (tipEl !== el || !el.isConnected) return;
    const r = el.getBoundingClientRect();
    tipState.label = label; tipState.rect = r; tipState.wrap = !!opts.wrap;
    tipState.side = opts.side ?? (r.bottom > innerHeight - 90 ? "above" : "below");
    tipState.shown = true;
  };
  if (opts.instant || tipsWarm()) open(); else tipTimer = window.setTimeout(open, 500);
}
function hideTip(el: HTMLElement) {
  if (tipEl !== el) return;
  clearTimeout(tipTimer);
  if (tipState.shown) tipLast = performance.now();
  tipState.shown = false; tipEl = null;
}
export function tipClosed() { tipLast = performance.now(); }

/** `use:tip={"Label"}` or `use:tip={{ label, wrap, instant, side }}`; `use:tip={iconTip("heart")}`. */
export function tip(el: HTMLElement, arg: string | undefined | { label?: string; wrap?: boolean; instant?: boolean; side?: "right" }) {
  let a = typeof arg === "string" || arg == null ? { label: arg } : arg;
  const enter = () => a.label && showTip(el, a.label, a);
  const leave = () => hideTip(el);
  const down = () => { if (tipEl === el) { clearTimeout(tipTimer); tipState.shown = false; } };
  el.addEventListener("pointerenter", enter);
  el.addEventListener("pointerleave", leave);
  el.addEventListener("pointerdown", down);
  return {
    update(n: typeof arg) {
      a = typeof n === "string" || n == null ? { label: n } : n;
      if (tipEl === el && tipState.shown && a.label) tipState.label = a.label;
    },
    destroy() { leave(); el.removeEventListener("pointerenter", enter); el.removeEventListener("pointerleave", leave); el.removeEventListener("pointerdown", down); },
  };
}
export const iconTip = (name: string) => tipFor(name);

// ---- ASCII reveal (widgets/indicators.rs) -------------------------------
export const fx = $state({ ascii: true, reduce: false, t: 0 });
export const pageChrome = $state({ stickyTop: false });
function initialSortDirs(): Record<string, "asc" | "desc"> {
  try {
    const raw = typeof localStorage !== "undefined" ? localStorage.getItem("glyph:list-sort-dir") : null;
    const parsed = raw ? JSON.parse(raw) : null;
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}
export const listSortDirs = $state<Record<string, "asc" | "desc">>(initialSortDirs());
const reveals = new Map<string, number>();
const DECODE_RAMP = "#%&*+=-:.@$0X";

let revealRaf = 0;
let revealUntil = 0;
function ensureRevealClock() {
  if (revealRaf || typeof requestAnimationFrame === "undefined" || (typeof document !== "undefined" && document.hidden)) return;
  const loop = () => {
    const now = performance.now();
    if (now < revealUntil && !document.hidden) {
      fx.t = now;
      revealRaf = requestAnimationFrame(loop);
    } else {
      fx.t = now;
      revealRaf = 0;
    }
  };
  revealRaf = requestAnimationFrame(loop);
}

/** Keeps the ASCII reveal timestamp cache bounded across route changes. */
export function trimReveals(limit = 128): void {
  if (limit <= 0) {
    reveals.clear();
    return;
  }
  while (reveals.size > limit) {
    const oldest = reveals.keys().next();
    if (oldest.done) break;
    reveals.delete(oldest.value);
  }
}

/** Progress 0..1 of the reveal under `key` (starts the first time asked). */
export function revealK(key: string, dur = 0.9): number {
  if (!fx.ascii || fx.reduce) return 1;
  void fx.t;
  let at = reveals.get(key);
  const now = performance.now();
  if (at == null) {
    at = now;
    reveals.set(key, at);
    if (reveals.size > 512) trimReveals(256);
  }
  const end = at + dur * 1000 + 80;
  if (now < end && end > revealUntil) {
    revealUntil = end;
    ensureRevealClock();
  }
  return Math.min(1, (now - at) / 1000 / dur);
}
export function revealRestart(key: string) {
  const now = performance.now();
  reveals.set(key, now);
  revealUntil = Math.max(revealUntil, now + 1200);
  ensureRevealClock();
}
export function resetReveals(prefix: string) { for (const k of [...reveals.keys()]) if (k.startsWith(prefix)) reveals.delete(k); }

/** `text` part-way decoded: the first `k` share is real, the rest flickers. */
export function decode(text: string, k: number): string {
  if (k >= 1) return text;
  const chars = [...text], done = Math.floor(Math.max(0, k) * chars.length), tick = Math.floor(performance.now() / 1000 * 12);
  return chars.map((c, i) => (i < done || /\s/.test(c) ? c : DECODE_RAMP[(((Math.imul(i, 2654435761) ^ Math.imul(tick, 40503)) >>> 7) >>> 0) % DECODE_RAMP.length])).join("");
}

// Start the reveal clock only if an active reveal window is still open.
export function startRevealClock() {
  if (performance.now() < revealUntil) ensureRevealClock();
}

// ---- adaptive ink (theme.rs adaptive) -----------------------------------
// Text right on the glow turns dark where the glow under it is bright.
let glow: Glow | null = null;
const inked = new Set<HTMLElement>();
let inkTimer = 0;
let staticDarkInk = false;
const glowSuspensions = new Set<string>();
let glowSuspendTimer = 0;
let glowTransient = false;

function syncGlowSuspension() {
  glow?.setSuspended(glowSuspensions.size > 0 || glowTransient);
}

/** Suspends background canvas drawing while a named UI interaction (e.g. sidebar resize) is active. */
export function setUiSuspended(key: string, suspended: boolean) {
  if (suspended) glowSuspensions.add(key);
  else glowSuspensions.delete(key);
  syncGlowSuspension();
}

/** Temporarily pauses background canvas drawing while UI panels or views finish changing. */
export function suspendBackground(ms = 240) {
  glowTransient = true;
  syncGlowSuspension();
  clearTimeout(glowSuspendTimer);
  glowSuspendTimer = window.setTimeout(() => {
    glowSuspendTimer = 0;
    glowTransient = false;
    syncGlowSuspension();
  }, ms);
}

export function setGlow(g: Glow | null, solidDarkInk = false) {
  glow = g;
  staticDarkInk = g ? false : solidDarkInk;
  clearInterval(inkTimer);
  inkTimer = 0;
  if (g) {
    syncGlowSuspension();
    inkTimer = window.setInterval(updateInks, 450);
  } else {
    for (const el of inked) el.classList.toggle("ink-dark", staticDarkInk);
  }
}
function updateInks() {
  if (!glow || document.hidden || glow.isPaused() || inked.size === 0) return;
  for (const el of inked) {
    const lum = glow.lumAt(el.getBoundingClientRect());
    if (lum == null) continue;
    const v = Math.min(1, Math.max(0, lum)), lin = v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
    el.classList.toggle("ink-dark", lin > 0.3);
  }
}
/** `use:ink` on text that sits on the background. */
export function ink(el: HTMLElement) {
  inked.add(el);
  el.style.transition = "color .3s ease-out";
  if (!glow) el.classList.toggle("ink-dark", staticDarkInk);
  return { destroy() { inked.delete(el); } };
}

// ---- press / pop feedback (widgets::press, toggle pop) -------------------
/** `use:press`: the element shrinks a little and springs back on click. */
export function press(el: HTMLElement, cls = "pressing") {
  const run = () => { el.classList.remove(cls); void el.offsetWidth; el.classList.add(cls); };
  const end = () => el.classList.remove(cls);
  el.addEventListener("pointerdown", run);
  el.addEventListener("animationend", end);
  return { destroy() { el.removeEventListener("pointerdown", run); el.removeEventListener("animationend", end); } };
}

/** Closes on a click outside `el` or on Escape. */
export function outside(el: HTMLElement, close: () => void) {
  const down = (e: PointerEvent) => { if (!el.contains(e.target as Node)) close(); };
  const key = (e: KeyboardEvent) => { if (e.key === "Escape") close(); };
  setTimeout(() => { addEventListener("pointerdown", down, true); addEventListener("keydown", key); });
  return { destroy() { removeEventListener("pointerdown", down, true); removeEventListener("keydown", key); } };
}

// ---- scroll edge fades (widgets/track_row.rs fade_edges) -----------------
export interface FadeEdgesOpts {
  /** Scroll axis ("y" default, or "x" for horizontal rows). */
  axis?: "x" | "y";
  /** Fade size in px for both edges when start/end aren't set separately. */
  size?: number;
  /** Start edge (top / left) fade size in px. */
  start?: number;
  /** End edge (bottom / right) fade size in px. */
  end?: number;
  /** Convenience aliases for `start` / `end` on vertical or horizontal frames. */
  top?: number;
  bottom?: number;
  left?: number;
  right?: number;
  /** Minimum overflow (px) before edges start fading. */
  minOverflow?: number;
  /** Disable temporarily (e.g. collapsed bio or desktop wrapper inside compact). */
  active?: boolean;
}

/**
 * `use:fadeEdges`: applies a scroll-aware CSS `mask-image` fade to the start
 * and end edges of a scroll container whenever there is more content beyond
 * that edge, avoiding hard cutoffs and dark overlay seams against the glow.
 */
export function fadeEdges(el: HTMLElement, initial: FadeEdgesOpts = {}) {
  let opts = initial;
  let lastMask = "";
  let raf = 0;

  const update = () => {
    raf = 0;
    const axis = opts.axis ?? "y";
    const active = opts.active ?? true;
    if (!active) {
      if (lastMask) {
        lastMask = "";
        el.style.maskImage = "";
        el.style.webkitMaskImage = "";
      }
      return;
    }
    const total = axis === "y" ? el.clientHeight : el.clientWidth;
    const scrollTotal = axis === "y" ? el.scrollHeight : el.scrollWidth;
    const pos = axis === "y" ? el.scrollTop : el.scrollLeft;
    const max = scrollTotal - total;
    const minOverflow = opts.minOverflow ?? 32;

    let startK = 0;
    let endK = 0;
    if (total > 0 && max >= minOverflow) {
      const ramp = Math.min(40, Math.max(8, max * 0.5));
      startK = Math.min(1, Math.max(0, (pos - 2) / ramp));
      endK = Math.min(1, Math.max(0, (max - pos - 2) / ramp));
    }

    let mask = "";
    if (startK > 0.001 || endK > 0.001) {
      let s = opts.start ?? (axis === "y" ? opts.top : opts.left) ?? opts.size ?? 18;
      let e = opts.end ?? (axis === "y" ? opts.bottom : opts.right) ?? opts.size ?? 18;
      const cap = total * 0.85;
      if (s + e > cap && s + e > 0) {
        const scale = cap / (s + e);
        s *= scale;
        e *= scale;
      }
      const stops: string[] = [];
      if (startK > 0.001 && s > 1) {
        const pad = Math.min(2, s * 0.1);
        const span = s - pad;
        stops.push(
          `rgba(0,0,0,${(1 - startK).toFixed(3)}) 0px`,
          `rgba(0,0,0,${(1 - startK).toFixed(3)}) ${pad.toFixed(1)}px`,
          `rgba(0,0,0,${(1 - startK * 0.844).toFixed(3)}) ${(pad + span * 0.25).toFixed(1)}px`,
          `rgba(0,0,0,${(1 - startK * 0.5).toFixed(3)}) ${(pad + span * 0.5).toFixed(1)}px`,
          `rgba(0,0,0,${(1 - startK * 0.156).toFixed(3)}) ${(pad + span * 0.75).toFixed(1)}px`,
          `#000 ${s.toFixed(1)}px`,
        );
      } else {
        stops.push("#000 0px");
      }
      if (endK > 0.001 && e > 1) {
        const pad = Math.min(2, e * 0.1);
        const span = e - pad;
        stops.push(
          `#000 calc(100% - ${e.toFixed(1)}px)`,
          `rgba(0,0,0,${(1 - endK * 0.156).toFixed(3)}) calc(100% - ${(pad + span * 0.75).toFixed(1)}px)`,
          `rgba(0,0,0,${(1 - endK * 0.5).toFixed(3)}) calc(100% - ${(pad + span * 0.5).toFixed(1)}px)`,
          `rgba(0,0,0,${(1 - endK * 0.844).toFixed(3)}) calc(100% - ${(pad + span * 0.25).toFixed(1)}px)`,
          `rgba(0,0,0,${(1 - endK).toFixed(3)}) calc(100% - ${pad.toFixed(1)}px)`,
          `rgba(0,0,0,${(1 - endK).toFixed(3)}) 100%`,
        );
      } else {
        stops.push("#000 100%");
      }
      mask = `linear-gradient(${axis === "y" ? "to bottom" : "to right"}, ${stops.join(", ")})`;
    }

    if (mask !== lastMask) {
      lastMask = mask;
      el.style.maskImage = mask;
      el.style.webkitMaskImage = mask;
    }
  };

  const schedule = () => {
    if (!raf) raf = requestAnimationFrame(update);
  };

  const ro = new ResizeObserver(schedule);
  const observeChildren = () => {
    ro.disconnect();
    ro.observe(el);
    for (const child of el.children) ro.observe(child);
  };
  observeChildren();

  const mo = new MutationObserver(() => {
    observeChildren();
    schedule();
  });
  mo.observe(el, { childList: true });

  el.addEventListener("scroll", update, { passive: true });
  update();
  schedule();

  return {
    update(next: FadeEdgesOpts = {}) {
      opts = next;
      update();
    },
    destroy() {
      cancelAnimationFrame(raf);
      el.removeEventListener("scroll", update);
      ro.disconnect();
      mo.disconnect();
      if (lastMask) {
        el.style.maskImage = "";
        el.style.webkitMaskImage = "";
      }
    },
  };
}
