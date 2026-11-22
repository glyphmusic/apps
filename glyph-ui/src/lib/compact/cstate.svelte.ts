import { cubicOut } from "svelte/easing";
import { app } from "../state.svelte";
import { fx } from "../ui.svelte";
import { pwa } from "../pwa.svelte";

// Phone layout state (compact.rs / compact/now_playing.rs): Now Playing is a
// sheet over the page (it never enters the back history), lyrics open full
// screen, the queue and Connect are bottom sheets, the profile menu is a
// drawer from the left, settings a full screen list.
export const phone = $state({
  npOpen: false,
  npPull: 0,
  lyricsOpen: false,
  videoMode: false,
  videoPickerOpen: false,
  drawerOpen: false,
  settingsInPage: false,
  settingsSub: 0,
  libFilter: 0,
  /** How far the page is scrolled (px): the list cover shrinks, the artist photo zooms. */
  scroll: 0,
  /** Resting scrollTop of PhoneListView's pull-to-reveal search/sort threshold. */
  listRevealH: 0,
});

/** The browser moves scroll-linked effects itself (on its own thread, in
 *  step with the finger), so they never stutter when the page is busy.
 *  Older WebViews fall back to updating them from the scroll position. */
export const scrollFx = typeof CSS !== "undefined" && typeof CSS.supports === "function" && CSS.supports("animation-timeline: scroll()");

/** Constants of compact.rs. */
export const MINI_H = 64, TABS_H = 42, HEADER_H = 60, EDGE = 16, SIDE = 24, SWIPE_ACT = 72;

/**
 * A finger drag on `el`: `move(dx, dy)` while dragging, `end(dx, dy, vx, vy)` on
 * release (velocities in px/ms). `axis` claims only drags along it (the other axis scrolls).
 */
export function drag(el: HTMLElement, opts: { axis?: "x" | "y" | "any"; move?: (dx: number, dy: number) => void; end?: (dx: number, dy: number, vx: number, vy: number) => void; onlyAtTop?: () => boolean }) {
  let o = opts;
  let start: { x: number; y: number; id: number; lx: number; ly: number; lt: number; vx: number; vy: number } | null = null;
  let touchStart: { x: number; y: number } | null = null;
  let claimed = false;
  let suppressClick = 0;
  const down = (e: PointerEvent) => {
    if (e.pointerType === "mouse" && e.button !== 0) return;
    suppressClick = 0;
    const now = performance.now();
    start = { x: e.clientX, y: e.clientY, id: e.pointerId, lx: e.clientX, ly: e.clientY, lt: now, vx: 0, vy: 0 };
    claimed = false;
  };
  const move = (e: PointerEvent) => {
    if (!start || e.pointerId !== start.id) return;
    const dx = e.clientX - start.x, dy = e.clientY - start.y;
    const now = performance.now();
    const dt = Math.max(1, now - start.lt);
    const ivx = (e.clientX - start.lx) / dt, ivy = (e.clientY - start.ly) / dt;
    start.vx = start.vx * 0.35 + ivx * 0.65;
    start.vy = start.vy * 0.35 + ivy * 0.65;
    start.lx = e.clientX; start.ly = e.clientY; start.lt = now;
    if (!claimed) {
      if (Math.hypot(dx, dy) < 5) return;
      const horiz = Math.abs(dx) > Math.abs(dy) * 1.2;
      const axis = o.axis ?? "any";
      if ((axis === "x" && !horiz) || (axis === "y" && horiz)) { start = null; return; }
      if (axis === "y" && o.onlyAtTop && (!o.onlyAtTop() || dy < 0)) { start = null; return; }
      claimed = true;
      el.setPointerCapture(e.pointerId);
    }
    e.preventDefault();
    o.move?.(dx, dy);
  };
  const tstart = (e: TouchEvent) => {
    if (e.touches.length !== 1) return;
    touchStart = { x: e.touches[0].clientX, y: e.touches[0].clientY };
  };
  const tmove = (e: TouchEvent) => {
    if (claimed) { e.preventDefault(); return; }
    if (!touchStart || e.touches.length !== 1) return;
    const dx = e.touches[0].clientX - touchStart.x, dy = e.touches[0].clientY - touchStart.y;
    if (Math.hypot(dx, dy) < 2) return;
    const horiz = Math.abs(dx) > Math.abs(dy) * 1.2;
    const axis = o.axis ?? "any";
    if (axis === "x" && horiz) e.preventDefault();
    else if (axis === "y" && !horiz && (!o.onlyAtTop || (o.onlyAtTop() && dy > 0))) e.preventDefault();
    else if (axis === "any") e.preventDefault();
  };
  const tend = () => { touchStart = null; };
  const up = (e: PointerEvent) => {
    if (!start || e.pointerId !== start.id) return;
    const dx = e.clientX - start.x, dy = e.clientY - start.y;
    const stale = performance.now() - start.lt > 85;
    const vx = stale ? 0 : start.vx, vy = stale ? 0 : start.vy;
    const was = claimed;
    start = null; claimed = false;
    if (was) {
      if (e.type === "pointerup") suppressClick = performance.now();
      o.end?.(dx, dy, vx, vy);
      e.preventDefault();
    }
  };
  const click = (e: MouseEvent) => {
    if (claimed || performance.now() - suppressClick < 350) {
      suppressClick = 0;
      e.stopPropagation();
      e.preventDefault();
    }
  };
  el.addEventListener("pointerdown", down);
  el.addEventListener("pointermove", move);
  el.addEventListener("pointerup", up);
  el.addEventListener("pointercancel", up);
  // Smooth scrolling: a blocking touchmove listener makes the browser wait
  // for this page's code before every scroll step (stutter on Android, most
  // of all on song rows, which are everywhere). So the browser is told up
  // front which way it may scroll here (touch-action), and only drags that
  // must decide per move (pull down a sheet only when its list is at the
  // top) keep a blocking listener.
  const ta = o.axis === "x" ? "pan-y" : o.axis === "y" && !o.onlyAtTop ? "pan-x" : o.axis === "any" || !o.axis ? "none" : "";
  const cur = typeof getComputedStyle === "function" ? getComputedStyle(el).touchAction : "";
  if (ta && (cur === "auto" || cur === "")) el.style.touchAction = ta;
  const blocking = !ta || !(el.style.touchAction === ta || cur === ta || cur === "none");
  el.addEventListener("touchstart", tstart, { passive: true });
  if (blocking) el.addEventListener("touchmove", tmove, { passive: false });
  el.addEventListener("touchend", tend, { passive: true });
  el.addEventListener("touchcancel", tend, { passive: true });
  el.addEventListener("click", click, true);
  return {
    update(n: typeof opts) { o = n; },
    destroy() {
      el.removeEventListener("pointerdown", down); el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up); el.removeEventListener("pointercancel", up);
      el.removeEventListener("touchstart", tstart); el.removeEventListener("touchmove", tmove);
      el.removeEventListener("touchend", tend); el.removeEventListener("touchcancel", tend);
      el.removeEventListener("click", click, true);
    },
  };
}

/** Velocity-aware dismiss check: fast return flick cancels close even when past threshold. */
export function shouldClose(dist: number, vel: number, threshold = SWIPE_ACT): boolean {
  if (vel < -0.3) return false;
  if (vel > 0.45 && dist > 18) return true;
  return dist > threshold;
}

export function sheetIn(_node: HTMLElement, opts: { axis?: "x" | "y"; dir?: 1 | -1; duration?: number } = {}) {
  const axis = opts.axis === "x" ? "X" : "Y";
  const dir = opts.dir ?? 1;
  return {
    duration: fx.reduce ? 1 : (opts.duration ?? 280),
    easing: cubicOut,
    css: (t: number) => `transform: translate${axis}(${((1 - t) * dir * 100).toFixed(2)}%);`,
  };
}

export function sheetOut(node: HTMLElement, opts: { axis?: "x" | "y"; dir?: 1 | -1; duration?: number; radius?: boolean } = {}) {
  const axis = opts.axis === "x" ? "X" : "Y";
  const dir = opts.dir ?? 1;
  const cs = getComputedStyle(node);
  const m = new DOMMatrix(cs.transform);
  const start = axis === "X" ? m.m41 : m.m42;
  const end = dir * ((axis === "X" ? node.offsetWidth : node.offsetHeight) || (axis === "X" ? innerWidth : innerHeight));
  const startR = opts.radius ? parseFloat(cs.borderTopLeftRadius) || 0 : 0;
  return {
    duration: fx.reduce ? 1 : (opts.duration ?? 220),
    easing: cubicOut,
    css: (_t: number, u: number) => {
      const pos = start + (end - start) * u;
      const rad = opts.radius ? ` border-radius: ${(startR + (22 - startR) * u).toFixed(1)}px ${(startR + (22 - startR) * u).toFixed(1)}px 0 0;` : "";
      return `transform: translate${axis}(${pos.toFixed(1)}px);${rad}`;
    },
  };
}

/** Opens context menus on long press in compact layout (touch + mouse hold) with press/bounce feedback. */
export function installCompactLongPress(holdMs = 360): () => void {
  let holdTimer = 0;
  let fireTimer = 0;
  let popTimer = 0;
  let sx = 0, sy = 0, pid = -1, suppressedPid = -1;
  let target: HTMLElement | null = null;
  let host: HTMLElement | null = null;
  let firedAt = -1000;
  let suppressUntil = 0;

  const findHost = (el: HTMLElement) =>
    el.closest(".row, .card[role='button'], .c, .lr, .it, .ar, .tile, .rc, .ac, .lcard, .mini, .q-row, .now-row, .arow, .grow, button.link") as HTMLElement | null;

  const clearHold = () => {
    clearTimeout(holdTimer); holdTimer = 0;
    clearTimeout(fireTimer); fireTimer = 0;
    host?.classList.remove("lp-hold");
    pid = -1; target = null; host = null;
  };

  const onDown = (e: PointerEvent) => {
    if (e.pointerType === "mouse" && e.button !== 0) return;
    const t = e.target as HTMLElement | null;
    if (!t || t.closest("input, textarea, select, [data-no-longpress], .bs-panel, .bs-dim, .ctx, .sheet, .mini .acts, .row .b, .it .x, .grip")) return;
    clearHold();
    suppressedPid = -1;
    const h = findHost(t);
    if (!h) return;
    sx = e.clientX; sy = e.clientY; pid = e.pointerId; target = t;
    host = h;
    holdTimer = window.setTimeout(() => { h.classList.add("lp-hold"); }, 85);
    fireTimer = window.setTimeout(() => {
      const el = target, popEl = host, x = sx, y = sy, activePid = pid;
      clearHold();
      const hit = el?.isConnected ? el : popEl?.isConnected ? popEl : null;
      if (!hit) return;
      firedAt = performance.now();
      suppressedPid = activePid;
      suppressUntil = firedAt + 5000;
      if (popEl?.isConnected) {
        popEl.classList.remove("lp-pop");
        void popEl.offsetWidth;
        popEl.classList.add("lp-pop");
        clearTimeout(popTimer);
        popTimer = window.setTimeout(() => popEl.classList.remove("lp-pop"), 360);
      }
      hit.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, composed: true, view: window, clientX: x, clientY: y }));
    }, holdMs);
  };
  const onMove = (e: PointerEvent) => {
    if (fireTimer && e.pointerId === pid && Math.hypot(e.clientX - sx, e.clientY - sy) > 12) clearHold();
  };
  const onUp = (e: PointerEvent) => {
    if (e.pointerId === pid) clearHold();
    if (e.pointerId === suppressedPid) {
      suppressUntil = performance.now() + 400;
      suppressedPid = -1;
    }
  };
  const onScroll = (e: Event) => {
    if (target && (e.target === document || (e.target instanceof Node && e.target.contains(target)))) clearHold();
  };
  const onClick = (e: MouseEvent) => {
    if (performance.now() < suppressUntil) {
      if (suppressedPid < 0) suppressUntil = 0;
      e.preventDefault();
      e.stopImmediatePropagation();
    }
  };
  const onCtx = (e: MouseEvent) => {
    if (e.isTrusted && performance.now() - firedAt < 600) { e.preventDefault(); e.stopImmediatePropagation(); }
  };

  addEventListener("pointerdown", onDown, true);
  addEventListener("pointermove", onMove, true);
  addEventListener("pointerup", onUp, true);
  addEventListener("pointercancel", onUp, true);
  addEventListener("scroll", onScroll, true);
  addEventListener("click", onClick, true);
  addEventListener("contextmenu", onCtx, true);
  return () => {
    clearHold();
    clearTimeout(popTimer);
    removeEventListener("pointerdown", onDown, true);
    removeEventListener("pointermove", onMove, true);
    removeEventListener("pointerup", onUp, true);
    removeEventListener("pointercancel", onUp, true);
    removeEventListener("scroll", onScroll, true);
    removeEventListener("click", onClick, true);
    removeEventListener("contextmenu", onCtx, true);
  };
}

/** Handles Android hardware/gesture back before backgrounding the activity. */
export function installAndroidBack() {
  (window as unknown as { __glyphBack?: () => boolean }).__glyphBack = () => {
    if (pwa.sheetOpen) { pwa.sheetOpen = false; return true; }
    if (app.confirm) { app.confirm = null; return true; }
    if (app.shareSheet) { app.shareSheet = null; return true; }
    if (app.pickSongs) { app.pickSongs = null; return true; }
    if (app.nameDialog) { app.nameDialog = null; return true; }
    if (app.addSongs) { app.addSongs = null; return true; }
    if (app.sharedPreview) { app.sharedPreview = null; return true; }
    if (app.onboarding) { app.onboarding = false; return true; }
    if (app.ctx || app.controlMenu) { app.closeMenus(); return true; }
    if (app.jamOpen) { app.jamOpen = false; return true; }
    if (phone.drawerOpen) { phone.drawerOpen = false; return true; }
    if (phone.lyricsOpen) { phone.lyricsOpen = false; return true; }
    if (app.settingsOpen) {
      if (phone.settingsInPage) phone.settingsInPage = false;
      else app.settingsOpen = false;
      return true;
    }
    if (app.showQueue) { app.showQueue = false; return true; }
    if (app.showDevices) { app.showDevices = false; return true; }
    if (phone.npOpen) { phone.npOpen = false; return true; }
    if (app.history.length > 0) { app.back(); return true; }
    if (app.view !== "home") { app.navigate("home"); return true; }
    return false;
  };
}
