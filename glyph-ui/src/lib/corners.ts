// A look's corner roundness (LookSpec.radius: 0 square, 1 Glyph's, 2 very
// round) for the whole interface. Glyph's components set their corners in
// pixels in their own styles, so one variable can't reach them all: this
// scales every pixel corner radius in Glyph's style sheets instead (and in
// sheets loaded later, like a page's styles). Round shapes stay round:
// percentages and pill radii (100px and more) are left alone, and so is the
// looks' own style sheet. Scale 1 puts the original values back.
//
// A look can also set single parts (LookSpec.radii): each rule is sorted
// into a part by what its selector styles (its last piece), see partOf.

const PROPS = ["border-top-left-radius", "border-top-right-radius", "border-bottom-right-radius", "border-bottom-left-radius"];
/** Custom properties that hold a corner (`--r`, `--mi-r`, `--card-radius` …). */
const RADIUS_VAR = /^--(?:[a-z0-9-]*-)?r$|^--(?!radius-k$)[a-z0-9-]*radius[a-z0-9-]*$/;
/** Pill-shaped corners (999px and alike) are not scaled. */
const PILL = 100;

/** Parts a look can round one by one (glyph_core::plugins::RADIUS_PARTS). */
export type CornerPart = "covers" | "buttons" | "fields" | "cards" | "menus" | "dialogs" | "player";

/** What a selector's last piece names -> its part (first match wins). */
const PART_HINTS: [CornerPart, RegExp][] = [
  ["player", /mini-?bar|mini-?player|player-?bar|now-?playing|\bnp\b|data-part="(?:player-bar|mini-player|mini-bar|now-playing)"/],
  ["dialogs", /sheet|modal|dialog|toast|popup|data-part="(?:dialog|toasts)"/],
  ["menus", /menu|ctx|tooltip|\btip\b|popover|dropdown|data-part="(?:menu|tooltip)"/],
  ["fields", /input|textarea|select|field|search/],
  ["covers", /cover|\bart\b|artwork|thumb|\bimg\b|avatar|pfp|photo|picture/],
  ["buttons", /button|\bbtn|chip|pill|\btog|toggle|\bplay\b|bar-play|action|\btab\b/],
];

/** The part a rule belongs to: what its selector's last piece styles. */
export function partOf(selector: string): CornerPart {
  // Svelte's scope classes (.svelte-1abc2d) are random letters: left out.
  for (const sel of selector.toLowerCase().replace(/\.svelte-[a-z0-9]+/g, "").split(",")) {
    const last = sel.trim().split(/\s*[ >+~]\s*/).pop() ?? "";
    for (const [part, re] of PART_HINTS) if (re.test(last)) return part;
  }
  return "cards";
}

/** Original corner values of each rule touched, to scale from (or restore). */
const originals = new WeakMap<CSSStyleDeclaration, [string, string, string][]>();
/** Each rule's part, sorted once. */
const partsOf = new WeakMap<CSSStyleDeclaration, CornerPart>();
let scale = 1;
let parts: Partial<Record<CornerPart, number>> = {};
let watching = false;

/** The scale of `part` now: its own, else the whole look's. */
function scaleOf(part: CornerPart): number {
  return parts[part] ?? scale;
}

/** A corner of `px` pixels set from code (inline styles), scaled with the
 *  look's corners like the style sheets (the root's `--radius-k`). */
export function cornerPx(px: number): string {
  return `calc(${px}px * var(--radius-k, 1))`;
}

/** `value` with every pixel length times `k` (pills stay pills). */
export function scaleRadius(value: string, k: number): string {
  return value.replace(/(\d*\.?\d+)px/g, (m, n: string) => {
    const x = parseFloat(n);
    return x >= PILL ? m : `${Math.round(x * k * 10) / 10}px`;
  });
}

function fix(style: CSSStyleDeclaration, selector: string) {
  let orig = originals.get(style);
  if (!orig) {
    // Corner variables too (like a menu's `--mi-r: 10px`): rules that use
    // them read as empty corners here, so the variable is scaled instead.
    const vars = Array.from(style).filter((p) => p.startsWith("--") && RADIUS_VAR.test(p));
    orig = [...PROPS, ...vars].map((p) => [p, style.getPropertyValue(p).trim(), style.getPropertyPriority(p)] as [string, string, string]).filter(([, v]) => v && v.includes("px"));
    if (!orig.length) return;
    originals.set(style, orig);
    partsOf.set(style, partOf(selector));
  }
  const k = scaleOf(partsOf.get(style) ?? "cards");
  for (const [p, v, prio] of orig) style.setProperty(p, k === 1 ? v : scaleRadius(v, k), prio);
}

function walk(rules: CSSRuleList) {
  for (const r of Array.from(rules)) {
    if ("style" in r && r.style instanceof CSSStyleDeclaration) fix(r.style, (r as CSSStyleRule).selectorText ?? "");
    const inner = (r as CSSGroupingRule).cssRules;
    if (inner) walk(inner);
  }
}

function fixSheet(sheet: CSSStyleSheet | null) {
  if (!sheet || (sheet.ownerNode as Element | null)?.id === "glyph-plugin-looks") return;
  try { walk(sheet.cssRules); } catch { /* another origin's sheet: not ours */ }
}

function fixAll() {
  for (const s of Array.from(document.styleSheets)) fixSheet(s);
}

/** Watches for style sheets added later (lazily loaded pages). */
function watch() {
  if (watching || typeof MutationObserver === "undefined") return;
  watching = true;
  new MutationObserver((muts) => {
    if (allOne()) return;
    for (const m of muts) for (const n of Array.from(m.addedNodes)) {
      if (n instanceof HTMLStyleElement) fixSheet(n.sheet);
      else if (n instanceof HTMLLinkElement && n.rel === "stylesheet") {
        if (n.sheet) fixSheet(n.sheet);
        n.addEventListener("load", () => fixSheet(n.sheet), { once: true });
      }
    }
  }).observe(document.head, { childList: true });
}

const clampK = (k: number) => Math.min(2, Math.max(0, Number.isFinite(k) ? k : 1));
const allOne = () => scale === 1 && Object.values(parts).every((v) => v === 1);

/** The look's corner scale now (1 = Glyph's), of one part or the whole. */
export function cornerK(part?: CornerPart): number { return part ? scaleOf(part) : scale; }

/**
 * Scales the corners of the whole interface by `k` (0 to 2; 1 = Glyph's),
 * and single parts by their own (`each`).
 */
export function setCornerScale(k: number, each: Partial<Record<CornerPart, number>> = {}) {
  if (typeof document === "undefined") return;
  k = clampK(k);
  const next: Partial<Record<CornerPart, number>> = {};
  for (const [p, v] of Object.entries(each)) next[p as CornerPart] = clampK(v as number);
  if (k === scale && JSON.stringify(next) === JSON.stringify(parts)) return;
  scale = k;
  parts = next;
  fixAll();
  if (!allOne()) watch();
}
