// Scroll direction memory. After scrolling up or down, a new scroll that
// starts within HOLD ms (a short pause) keeps going up or down, even when it
// starts on a row that scrolls sideways (home card rows, artist rows): the
// row does not catch it. After scrolling a row sideways, that row keeps the
// sideways direction for the same short time. A longer pause, and the row
// takes the wheel or swipe again.
// Mouse wheel: `sideScroll` turns up/down wheel into sideways scrolling only
// when the page was not just scrolled (and not at the row's end). Touch:
// classes on the root and the row switch the rows' touch-action (app.css),
// set while scrolling, so they are in place before the next touch starts.

const HOLD = 800;
type Axis = "x" | "y";

let axis: Axis | null = null;
let at = 0;
let xEl: Element | null = null;
let timer = 0;
const last = new WeakMap<Element, { l: number; t: number }>();

function release() {
  timer = 0;
  axis = null;
  document.documentElement.classList.remove("scroll-y");
  xEl?.classList.remove("scroll-x");
  xEl = null;
}

function note(a: Axis, el: Element) {
  axis = a;
  at = performance.now();
  document.documentElement.classList.toggle("scroll-y", a === "y");
  const nx = a === "x" ? el : null;
  if (xEl !== nx) {
    xEl?.classList.remove("scroll-x");
    nx?.classList.add("scroll-x");
    xEl = nx;
  }
  clearTimeout(timer);
  timer = window.setTimeout(release, HOLD);
}

function onScroll(e: Event) {
  const el = e.target instanceof Element ? e.target : document.scrollingElement;
  if (!el) return;
  const p = last.get(el) ?? { l: 0, t: 0 };
  const l = el.scrollLeft, t = el.scrollTop;
  last.set(el, { l, t });
  const dx = Math.abs(l - p.l), dy = Math.abs(t - p.t);
  if (dx === dy) return;
  note(dx > dy ? "x" : "y", el);
}

if (typeof document !== "undefined") {
  document.addEventListener("scroll", onScroll, { capture: true, passive: true });
}

/** The direction scrolled last, if that was less than HOLD ms ago:
 *  "y" for up/down anywhere, "x" only for sideways scrolling of `el`. */
export function recentAxis(el: Element): Axis | null {
  if (!axis || performance.now() - at >= HOLD) return null;
  if (axis === "x") return xEl === el ? "x" : null;
  return "y";
}

/**
 * `use:sideScroll` on a row that scrolls sideways: the up/down mouse wheel
 * scrolls it sideways, unless the page was just scrolled up or down (then
 * the page goes on) or the row is at its end that way (then the page
 * scrolls). Also marks the row for the touch rules in app.css.
 */
export function sideScroll(el: HTMLElement) {
  el.dataset.hscroll = "";
  const wheel = (e: WheelEvent) => {
    if (el.scrollWidth <= el.clientWidth || Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
    if (recentAxis(el) === "y") return;
    const max = el.scrollWidth - el.clientWidth;
    if ((e.deltaY < 0 && el.scrollLeft <= 0) || (e.deltaY > 0 && el.scrollLeft >= max - 1)) return;
    e.preventDefault();
    el.scrollBy({ left: e.deltaY, behavior: e.deltaMode === 0 && Math.abs(e.deltaY) < 50 ? "auto" : "smooth" });
  };
  el.addEventListener("wheel", wheel, { passive: false });
  return {
    destroy() {
      el.removeEventListener("wheel", wheel);
      if (xEl === el) xEl = null;
    },
  };
}
