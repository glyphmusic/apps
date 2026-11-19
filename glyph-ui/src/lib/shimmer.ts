// ASCII shimmer of the loading placeholders (src/ui/widgets/skeleton.rs
// paint_shimmer): a grid of mono characters from the ramp " .:-=+*#", each
// cell with a little fixed grain, and a soft diagonal band that sweeps
// across and lifts the characters it passes to denser ones, so the
// characters themselves change as it moves.
// One clock for every placeholder (20 fps). It only runs while a moving
// placeholder is on screen and the window is visible; per frame only the
// cells whose character changed are drawn again. Performance mode and
// reduced motion: one still frame (just the grain), no clock at all.

const RAMP = " .:-=+*#";
const FPS = 20;
/** Cells per second the band travels (the app: 60). */
const SPEED = 60;
const MONO = '"JetBrains Mono", Consolas, "Cascadia Mono", Menlo, monospace';

interface Atlas { c: HTMLCanvasElement; gw: number; gh: number }

interface Item {
  el: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D | null;
  seed: number;
  still: boolean;
  visible: boolean;
  /** Box in css px and the device pixel ratio it was laid out for. */
  w: number; h: number; dpr: number;
  /** Character cell in device px. */
  cw: number; lh: number; oy: number;
  cols: number; rows: number;
  /** Ramp index drawn per cell (255: nothing drawn yet). */
  shown: Uint8Array;
  grain: Float32Array;
  atlas: Atlas | null;
}

const items = new Set<Item>();
const byEl = new WeakMap<Element, Item>();
const atlases = new Map<string, Atlas>();
const t0 = performance.now();
let timer = 0;
let fontsHooked = false;

const ro = typeof ResizeObserver !== "undefined"
  ? new ResizeObserver((entries) => {
      for (const e of entries) {
        const it = byEl.get(e.target);
        if (!it) continue;
        const box = e.contentBoxSize?.[0];
        layout(it, box ? box.inlineSize : e.contentRect.width, box ? box.blockSize : e.contentRect.height);
      }
      wake();
    })
  : null;

const io = typeof IntersectionObserver !== "undefined"
  ? new IntersectionObserver((entries) => {
      for (const e of entries) {
        const it = byEl.get(e.target);
        if (!it) continue;
        it.visible = e.isIntersecting;
        // Off screen and back: draw it whole again.
        if (it.visible) it.shown.fill(255);
      }
      wake();
    })
  : null;

/** One picture with every ramp character side by side, for one cell size. */
function atlasFor(fsDev: number, cw: number, lh: number): Atlas {
  const gw = Math.max(1, Math.ceil(cw)), gh = Math.max(1, Math.ceil(lh));
  const key = `${fsDev.toFixed(1)}|${gw}|${gh}`;
  let a = atlases.get(key);
  if (a) return a;
  if (atlases.size > 48) atlases.clear();
  const c = document.createElement("canvas");
  c.width = gw * RAMP.length; c.height = gh;
  const x = c.getContext("2d");
  if (x) {
    x.font = `600 ${fsDev}px ${MONO}`;
    x.textBaseline = "middle";
    x.textAlign = "center";
    x.fillStyle = "rgba(255, 255, 255, 0.32)";
    for (let i = 1; i < RAMP.length; i++) x.fillText(RAMP[i], i * gw + gw / 2, gh / 2);
  }
  a = { c, gw, gh };
  atlases.set(key, a);
  return a;
}

/** Cell size and grid for a box of `w` x `h` css px (skeleton.rs skeleton()). */
function layout(it: Item, w: number, h: number) {
  const dpr = Math.min(2, Math.max(1, devicePixelRatio || 1));
  if (w === it.w && h === it.h && dpr === it.dpr && it.atlas) return;
  it.w = w; it.h = h; it.dpr = dpr;
  const W = Math.max(1, Math.round(w * dpr)), H = Math.max(1, Math.round(h * dpr));
  if (it.el.width !== W) it.el.width = W;
  if (it.el.height !== H) it.el.height = H;
  if (w < 1 || h < 1) { it.cols = 0; it.rows = 0; return; }
  // Thin bars get one row of characters as tall as the bar; bigger boxes
  // characters of at least 9 px, more of them the bigger the box.
  const fs = h < 18 ? Math.max(6, h * 0.95) : Math.max(9, Math.sqrt((w * h) / (0.6 * 1.05 * 1500)));
  const cw = fs * 0.6, lh = fs * 1.05;
  const cols = Math.max(1, Math.ceil(w / cw));
  const rows = Math.max(1, Math.floor(h / lh));
  it.cw = cw * dpr; it.lh = lh * dpr;
  it.cols = cols; it.rows = rows;
  it.oy = Math.max(0, (H - rows * it.lh) / 2);
  it.atlas = atlasFor(fs * dpr, it.cw, it.lh);
  it.shown = new Uint8Array(cols * rows).fill(255);
  it.grain = new Float32Array(cols * rows);
  for (let y = 0; y < rows; y++) for (let x = 0; x < cols; x++) {
    const hh = (Math.imul(x, 2654435761) ^ Math.imul(y + it.seed, 40503)) >>> 0;
    it.grain[y * cols + x] = (hh % 7) / 7 * 0.22;
  }
  it.ctx?.clearRect(0, 0, W, H);
}

/** Draws the characters that changed since the last frame. */
function paint(it: Item, t: number) {
  const { ctx, atlas, cols, rows } = it;
  if (!ctx || !atlas || !cols || !rows) return;
  // The band runs from left of the box to past its right edge (diagonal: the
  // lower rows lag behind), then starts again.
  const span = cols + rows * 1.6 + 40;
  const x0 = it.still ? -1e4 : ((t * SPEED) % span) - 20;
  const top = RAMP.length - 1;
  for (let y = 0; y < rows; y++) {
    const py = Math.round(it.oy + y * it.lh);
    for (let x = 0; x < cols; x++) {
      const i = y * cols + x;
      const d = x + y * 1.6 - x0;
      const band = Math.exp(-(d * d) / 18);
      const v = Math.min(1, 0.18 + it.grain[i] + band * 0.8);
      const k = Math.min(top, Math.round(v * top));
      if (it.shown[i] === k) continue;
      it.shown[i] = k;
      const px = Math.round(x * it.cw);
      ctx.clearRect(px, py, atlas.gw, atlas.gh);
      if (k > 0) ctx.drawImage(atlas.c, k * atlas.gw, 0, atlas.gw, atlas.gh, px, py, atlas.gw, atlas.gh);
    }
  }
}

function moving() {
  for (const it of items) if (!it.still && it.visible) return true;
  return false;
}

function frame() {
  timer = 0;
  if (typeof document !== "undefined" && document.hidden) return;
  const t = (performance.now() - t0) / 1000;
  for (const it of items) if (it.visible) paint(it, t);
  if (moving()) timer = window.setTimeout(frame, 1000 / FPS);
}

/** Draws now and keeps the clock going while something moves. */
function wake() {
  if (timer || typeof window === "undefined") return;
  timer = window.setTimeout(frame, 0);
}

if (typeof document !== "undefined") {
  document.addEventListener("visibilitychange", () => { if (!document.hidden) wake(); });
}

/** Fonts that arrive late: draw every placeholder again with the right one. */
function hookFonts() {
  if (fontsHooked || typeof document === "undefined" || !document.fonts) return;
  fontsHooked = true;
  void document.fonts.ready.then(() => {
    atlases.clear();
    for (const it of items) { it.atlas = null; layout(it, it.w, it.h); }
    wake();
  });
}

export function seedOf(id: string): number {
  let h = 2166136261;
  for (let i = 0; i < id.length; i++) h = Math.imul(h ^ (id.charCodeAt(i) & 255), 16777619) >>> 0;
  return h;
}

/** `use:shimmer={{ seed, still }}` on a canvas that fills its placeholder. */
export function shimmer(el: HTMLCanvasElement, opts: { seed: number; still: boolean }) {
  hookFonts();
  const it: Item = {
    el, ctx: el.getContext("2d"), seed: opts.seed, still: opts.still,
    // Until the observer says otherwise: it was just put on the page.
    visible: !io,
    w: -1, h: -1, dpr: 0, cw: 0, lh: 0, oy: 0, cols: 0, rows: 0,
    shown: new Uint8Array(0), grain: new Float32Array(0), atlas: null,
  };
  items.add(it);
  byEl.set(el, it);
  const r = el.getBoundingClientRect();
  layout(it, r.width, r.height);
  ro?.observe(el);
  io?.observe(el);
  wake();
  return {
    update(n: { seed: number; still: boolean }) {
      if (n.seed !== it.seed) { it.seed = n.seed; it.atlas = null; layout(it, it.w, it.h); }
      if (n.still !== it.still) { it.still = n.still; it.shown.fill(255); }
      wake();
    },
    destroy() {
      items.delete(it);
      ro?.unobserve(el);
      io?.unobserve(el);
    },
  };
}
