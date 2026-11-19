// The app's background, ported from src/visuals (field.rs, glow.rs,
// palette.rs): the glow field sampled once per 8 x 8 px on a low-res canvas
// the browser stretches into soft light, the ASCII layer (10 x 16 cells,
// ramp " .-=+xX0S8#@", 5 x 7 font) and the titlebar dot grid on top.
// Same presets, ramps, thresholds and alphas as the app.
// Settings > Appearance > Animations picks who draws it: "gpu" (default)
// computes the field and everything on top on the graphics card
// (glow-gpu.ts, WebGL 2; with only WebGL 1 just the characters, glow-gl.ts),
// "cpu" draws everything here without WebGL. Performance mode samples
// coarser and draws less often. The graphics card is checked before it is
// used (`gpuProbe`) and watched while it draws; when it fails, the CPU takes
// over and `Glow.onGpuFail` tells the app to stay on the CPU.
import { AsciiGl } from "./glow-gl";
import { FieldGpu, type GpuFrame, type GpuProbe } from "./glow-gpu";

export type Rgb = [number, number, number];
export type Palette = [Rgb, Rgb, Rgb];
export type Preset = "strahl" | "band" | "sonnenaufgang" | "horizont";
export type GlowShape = Preset | "" | "solid";
export const PRESETS: Preset[] = ["strahl", "band", "sonnenaufgang", "horizont"];
/** Who draws the background (`visuals.render_mode`). */
export type RenderMode = "gpu" | "cpu";

const FONT = [
  [0, 0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 4, 0], [0, 0, 0, 14, 0, 0, 0], [0, 0, 31, 0, 31, 0, 0],
  [0, 4, 4, 31, 4, 4, 0], [0, 0, 17, 10, 4, 10, 17], [17, 17, 10, 4, 10, 17, 17], [14, 17, 19, 21, 25, 17, 14],
  [15, 16, 16, 14, 1, 1, 30], [14, 17, 17, 14, 17, 17, 14], [10, 10, 31, 10, 31, 10, 10], [14, 17, 23, 21, 23, 16, 15],
];
const RAMP_LEN = 12, PITCH = 8, CW = 10, CH = 16, TITLEBAR_H = 44;
const PRESET_FADE = 1.5, PALETTE_FADE = 1.5;

export const hex = (h: number): Rgb => [(h >> 16 & 255) / 255, (h >> 8 & 255) / 255, (h & 255) / 255];
const luma = (c: Rgb) => 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
/** Whether adaptive ink on a solid background of `base` (`0xRRGGBB`) should turn dark. */
export const solidIsDarkInk = (base: number): boolean => {
  const v = Math.min(1, Math.max(0, luma(hex(base))));
  const lin = v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
  return lin > 0.3;
};
const lerp = (a: Rgb, b: Rgb, t: number): Rgb => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
const scale = (a: Rgb, k: number): Rgb => [a[0] * k, a[1] * k, a[2] * k];
const lighten = (a: Rgb, k: number): Rgb => lerp(a, [1, 1, 1], k);
const gauss = (x: number) => Math.exp(-x * x);
const smooth = (a: number, b: number, x: number) => { const t = Math.min(1, Math.max(0, (x - a) / (b - a))); return t * t * (3 - 2 * t); };
const sat = (c: Rgb) => { const mx = Math.max(...c), mn = Math.min(...c); return mx <= 0 ? 0 : (mx - mn) / mx; };

type Ramp = [Rgb, Rgb, Rgb, Rgb];
const rampAt = (r: Ramp, v: number): Rgb => { v = Math.min(1, Math.max(0, v)) * 3; const i = Math.min(2, v | 0); return lerp(r[i], r[i + 1], v - i); };
const rampLerp = (a: Ramp, b: Ramp, t: number): Ramp => [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t), lerp(a[3], b[3], t)];

/** field.rs Preset::native: (cool, warm) ramps, dark → core. */
function native(p: Preset): [Ramp, Ramp] {
  switch (p) {
    case "strahl": return [[hex(0x050f5c), hex(0x1d5cff), hex(0x46dcff), hex(0xffffff)], [hex(0x3a0e06), hex(0xff6516), hex(0xffcc3a), hex(0xfff8e0)]];
    case "band": { const r: Ramp = [hex(0x16052f), hex(0x6526d6), hex(0xb88dff), hex(0xffffff)]; return [r, r]; }
    case "sonnenaufgang": { const r: Ramp = [hex(0x360404), hex(0xb11d0b), hex(0xff781d), hex(0xfff1a6)]; return [r, r]; }
    case "horizont": return [[hex(0x10042a), hex(0x6a29b6), hex(0xff6c90), hex(0xfff2f2)], [hex(0x10042a), hex(0x8a2fc0), hex(0xff8a7a), hex(0xffffff)]];
  }
}
const palRamps = ([a, b, d]: Palette): [Ramp, Ramp] => [[scale(a, 0.35), a, b, lighten(b, 0.85)], [scale(a, 0.35), b, d, lighten(d, 0.85)]];

function ramps(p: Preset, theme: Palette | null, pal: Palette | null, blend: number): [Ramp, Ramp] {
  const [c, w] = theme ? palRamps(theme) : native(p);
  if (pal && blend > 0) { const [pc, pw] = palRamps(pal); return [rampLerp(c, pc, blend), rampLerp(w, pw, blend)]; }
  return [c, w];
}

// Smooth 3D value noise in -1..1 (stands in for OpenSimplex).
const hash = (x: number, y: number, z: number) => { let h = (x * 374761393 + y * 668265263 + z * 1274126177) | 0; h = Math.imul(h ^ (h >>> 13), 1274126177); return ((h ^ (h >>> 16)) >>> 0) / 4294967295 * 2 - 1; };
const fade = (t: number) => t * t * (3 - 2 * t);
// Runs for every cell of every frame: plain arithmetic, no helper closures.
function noise(x: number, y: number, z: number) {
  const xi = Math.floor(x), yi = Math.floor(y), zi = Math.floor(z);
  const xf = fade(x - xi), yf = fade(y - yi), zf = fade(z - zi);
  const x1 = xi + 1, y1 = yi + 1, z1 = zi + 1;
  const a = hash(xi, yi, zi), b = hash(x1, yi, zi), c = hash(xi, y1, zi), d = hash(x1, y1, zi);
  const e = hash(xi, yi, z1), f = hash(x1, yi, z1), g = hash(xi, y1, z1), h = hash(x1, y1, z1);
  const ab = a + (b - a) * xf, cd = c + (d - c) * xf, ef = e + (f - e) * xf, gh = g + (h - g) * xf;
  const lo = ab + (cd - ab) * yf, hi = ef + (gh - ef) * yf;
  return lo + (hi - lo) * zf;
}

interface Motion { t: number; dx: number; dy: number; size: number; gain: number; energy: number }

// Reusable scratch outputs for pixel inner loops (avoids millions of tiny tuple arrays per second).
let _smpB = 0, _smpS = 0;
let _cr = 0, _cg = 0, _cb = 0;

/** field.rs sample without tuple allocation: writes `_smpB`, `_smpS`. */
function sampleInto(p: Preset, x: number, y: number, aspect: number, m: Motion) {
  x -= m.dx; y -= m.dy;
  switch (p) {
    case "strahl": {
      const ax = -0.05 * aspect, ay = 1.08, bx = aspect * 1.02, by = -0.08;
      const dx = bx - ax, dy = by - ay, len = Math.hypot(dx, dy), ux = dx / len, uy = dy / len;
      const px = x - ax, py = y - ay, s = (px * ux + py * uy) / len, d = Math.abs(px * uy - py * ux);
      const sc = Math.min(1, Math.max(0, s));
      const w = (0.02 + 0.17 * Math.pow(sc, 1.15)) * m.size;
      const along = smooth(-0.05, 0.25, s) * (0.55 + 0.45 * sc);
      _smpB = (gauss(d / w) + gauss(d / (w * 3)) * 0.26) * along;
      _smpS = smooth(0.35, 1, s);
      return;
    }
    case "band": {
      const u = (x / aspect) * 2 - 1, profile = Math.pow(Math.max(0, 1 - u * u), 0.85);
      const w = (0.025 + 0.2 * profile) * m.size, d = y - 0.52;
      _smpB = gauss(d / w) * (0.35 + 0.65 * profile) + gauss(d / (w * 3.5 + 0.04)) * 0.32 * (0.3 + 0.7 * profile);
      _smpS = 0.5;
      return;
    }
    case "sonnenaufgang": {
      const cx = aspect * 0.5, cy = 1.52, r = 0.78 * m.size * Math.min(1.7, Math.max(1, aspect)) / 1.35;
      const e = Math.hypot(x - cx, y - cy) - r;
      const rim = gauss(e / 0.06), halo = gauss(Math.max(0, e) / 0.38) * 0.6, inner = e < 0 ? 0.75 * Math.exp(e / 0.55) : 0;
      _smpB = Math.max(rim, halo, inner);
      _smpS = 1 - Math.min(1, Math.max(0, y));
      return;
    }
    case "horizont": {
      const sky = gauss(y / (0.62 * m.size)) * 0.98;
      const cx = aspect * 0.5, cy = 1.95, r = 1.32 / Math.max(0.5, m.size) * Math.min(1.7, Math.max(1, aspect)) / 1.35 + 0.05;
      const dist = Math.hypot(x - cx, y - cy), mask = smooth(r - 0.02, r + 0.22, dist);
      _smpB = sky * mask + gauss((dist - r) / 0.045) * 0.5;
      _smpS = 1 - mask;
      return;
    }
  }
}

function sampleMixRgb(cool: Ramp, warm: Ramp, b: number, s: number) {
  const vc = Math.min(1, Math.max(0, Math.min(b, 1))) * 3;
  const ic = Math.min(2, vc | 0), fc = vc - ic;
  const c0 = cool[ic], c1 = cool[ic + 1];
  const cr = c0[0] + (c1[0] - c0[0]) * fc;
  const cg = c0[1] + (c1[1] - c0[1]) * fc;
  const cb = c0[2] + (c1[2] - c0[2]) * fc;
  const w0 = warm[ic], w1 = warm[ic + 1];
  const wr = w0[0] + (w1[0] - w0[0]) * fc;
  const wg = w0[1] + (w1[1] - w0[1]) * fc;
  const wb = w0[2] + (w1[2] - w0[2]) * fc;
  _cr = cr + (wr - cr) * s;
  _cg = cg + (wg - cg) * s;
  _cb = cb + (wb - cb) * s;
}

/** One frame's field settings, kept in GPU mode so `lumAt` can work out a few cells here. */
interface CellFrame {
  preset: Preset; prevPreset: Preset; presetT: number;
  cool: Ramp; warm: Ramp; prev: [Ramp, Ramp] | null;
  m: Motion; t: number; sh: number; level: number; density: number; threshold: number;
  aspect: number; iw: number; ih: number;
}

let _fb = 0;
/** Cell (px, py) exactly as the draw loop computes it: writes `_fb` and `_cr`, `_cg`, `_cb`. */
function cellInto(f: CellFrame, px: number, py: number) {
  const x = (px + 0.5) / f.iw * f.aspect, y = (py + 0.5) / f.ih;
  sampleInto(f.preset, x, y, f.aspect, f.m);
  let b = _smpB;
  sampleMixRgb(f.cool, f.warm, b, _smpS);
  let c0 = _cr, c1 = _cg, c2 = _cb;
  if (f.prev) {
    sampleInto(f.prevPreset, x, y, f.aspect, f.m);
    const pb = _smpB;
    sampleMixRgb(f.prev[0], f.prev[1], pb, _smpS);
    b = pb + (b - pb) * f.presetT;
    c0 = _cr + (c0 - _cr) * f.presetT;
    c1 = _cg + (c1 - _cg) * f.presetT;
    c2 = _cb + (c2 - _cb) * f.presetT;
  }
  const n = noise(x * 1.7, y * 1.7, f.t * 0.045) * f.sh;
  _fb = Math.max(0, (b + n * Math.min(b, 1) * 1.4) * f.m.gain);
  _cr = c0; _cg = c1; _cb = c2;
}

/** Displayed brightness of one cell: the light plus the character drawn on it. */
function cellLum(b: number, c0: number, c1: number, c2: number, level: number, density: number, threshold: number) {
  const k = smooth(0.02, 1.1, b) * level * 0.5;
  const ascii = b * density >= threshold
    ? 0.33 * (0.16 + 0.16 * Math.min(b * density, 1)) * level * (0.2126 * (c0 + (1 - c0) * 0.3) + 0.7152 * (c1 + (1 - c1) * 0.3) + 0.0722 * (c2 + (1 - c2) * 0.3))
    : 0;
  return (0.2126 * c0 + 0.7152 * c1 + 0.0722 * c2) * k + ascii;
}

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));
const lum = (r: number, g: number, b: number) => 0.2126 * r + 0.7152 * g + 0.0722 * b;

/** A fixed frame (strahl, its own colours) for checking the GPU and timing the CPU. */
function testFrames(W: number, H: number): { cf: CellFrame; gf: GpuFrame } {
  const iw = Math.max(2, Math.ceil(W / PITCH)), ih = Math.max(2, Math.ceil(H / PITCH)), aspect = W / H;
  const [cool, warm] = native("strahl");
  const m: Motion = { t: 4.2, dx: 0, dy: 0, size: 1, gain: 0.94, energy: 0.7 };
  const sh = 0.1 + 0.08 * m.energy, level = 0.9, density = 0.83, threshold = 0.42;
  return {
    cf: { preset: "strahl", prevPreset: "strahl", presetT: 1, cool, warm, prev: null, m, t: m.t, sh, level, density, threshold, aspect, iw, ih },
    gf: {
      W, H, iw, ih, pitch: PITCH, aspect, preset: 0, prevPreset: 0, presetT: 1,
      cool, warm, prevCool: cool, prevWarm: warm, dx: 0, dy: 0, size: 1, gain: m.gain, t: m.t, sh, level,
      density, threshold, dotLevel: 0.3, blend: 0, ascii: true, dots: true,
    },
  };
}

/**
 * Checks the frame the GPU just drew against the CPU maths for the same
 * frame (`cf`): the field cell by cell, then one row of the window picture
 * (the light may not be darker than the field says; characters only add).
 * Returns what is wrong, or null.
 */
function verifyGpu(f: FieldGpu, cf: CellFrame, H: number, pitch: number): string | null {
  const err = f.check();
  if (err) return err;
  const px = f.readField(0);
  const total = cf.iw * cf.ih;
  if (!px || px.length < total * 4) return "could not read the field back";
  const step = Math.max(1, Math.floor(total / 900));
  let n = 0, bad = 0, lit = 0, gpuLit = 0;
  for (let i = 0; i < total; i += step) {
    cellInto(cf, i % cf.iw, (i / cf.iw) | 0);
    const b = Math.min(1, _fb * 0.5), o = i * 4;
    const e = Math.max(Math.abs(px[o] / 255 - clamp01(_cr)), Math.abs(px[o + 1] / 255 - clamp01(_cg)), Math.abs(px[o + 2] / 255 - clamp01(_cb)), Math.abs(px[o + 3] / 255 - b));
    if (!(e <= 0.12)) bad++;
    if (b > 0.1) lit++;
    if (px[o + 3] > 25) gpuLit++;
    n++;
  }
  if (lit > n * 0.02 && gpuLit === 0) return "the field came out black";
  if (bad > n * 0.25) return `the field came out wrong (${bad} of ${n} cells)`;
  const y = Math.floor(H / 2), row = f.readScreenRow(y);
  if (!row) return null;
  const cy = Math.min(cf.ih - 1, Math.floor(y / pitch));
  let want = 0;
  for (let x = 0; x < cf.iw; x++) {
    cellInto(cf, x, cy);
    const k = smooth(0.02, 1.1, _fb) * cf.level * 0.5;
    want += lum(clamp01(_cr * k), clamp01(_cg * k), clamp01(_cb * k));
  }
  want /= cf.iw;
  let got = 0;
  for (let i = 0; i < row.length; i += 4) got += lum(row[i], row[i + 1], row[i + 2]) / 255;
  got /= row.length / 4;
  if (want > 0.02 && !(got >= want * 0.35)) return "the window picture came out black";
  return null;
}

const PROBE_W = 256, PROBE_H = 160;
let probeCache: GpuProbe | null = null;
let pickCache: VisualsPick | null = null;

/** Whether (and how well) this device draws the background on the graphics card. Checked once per run. */
export function gpuProbe(): GpuProbe {
  if (probeCache) return probeCache;
  if (typeof document === "undefined") return { ok: false, broken: false, software: false, renderer: "", reason: "no window" };
  probeCache = FieldGpu.probe(FONT, PROBE_W, PROBE_H, (f) => {
    const { cf, gf } = testFrames(PROBE_W, PROBE_H);
    f.draw(gf);
    return verifyGpu(f, cf, PROBE_H, PITCH);
  });
  return probeCache;
}

/** Checks the graphics card again (Settings > Appearance > Animations: "Try again"). */
export function gpuRecheck(): GpuProbe {
  probeCache = null;
  pickCache = null;
  return gpuProbe();
}

/** The real background failed on the graphics card: remember it for this run. */
function markGpuBroken(reason: string) {
  probeCache = { ok: false, broken: true, software: probeCache?.software ?? false, renderer: probeCache?.renderer ?? "", reason };
  pickCache = null;
}

/** Whether this device can draw the background on the graphics card (WebGL 2, and it draws right). */
export function gpuBackgroundSupported(): boolean {
  return gpuProbe().ok;
}

/** What suits this device: who draws the background and whether performance mode helps. */
export interface VisualsPick { render: RenderMode; perf: boolean }

/** Milliseconds the processor needs for the field of a 1280 x 800 window (best of three). */
function cpuFieldMs(): number {
  const { cf } = testFrames(1280, 800);
  let best = Infinity;
  for (let r = 0; r < 3; r++) {
    const a = performance.now();
    for (let y = 0; y < cf.ih; y++) for (let x = 0; x < cf.iw; x++) cellInto(cf, x, y);
    best = Math.min(best, performance.now() - a);
  }
  return best;
}

/**
 * The best settings for this device, worked out once (first start, and the
 * "Recommended" mark in the onboarding): the graphics card draws when it
 * passes the check and is a real one (not a software renderer); performance
 * mode when the processor is slow, there are few cores or little memory,
 * or the processor has to draw everything and is not fast.
 */
export function recommendVisuals(): VisualsPick {
  if (pickCache) return pickCache;
  const g = gpuProbe();
  const gpu = g.ok && !g.software;
  const cores = (typeof navigator !== "undefined" && navigator.hardwareConcurrency) || 4;
  const mem = typeof navigator !== "undefined" ? (navigator as Navigator & { deviceMemory?: number }).deviceMemory : undefined;
  const ms = cpuFieldMs();
  const slow = ms > 6 || cores <= 2 || (mem !== undefined && mem <= 4);
  const perf = slow || (!gpu && (ms > 3 || cores <= 4));
  pickCache = { render: gpu ? "gpu" : "cpu", perf };
  console.info(`[glow] device check: ${g.renderer || "no renderer"}${g.software ? " (software)" : ""}, gpu ${g.ok ? "ok" : g.broken ? "broken" : "missing"}, ${cores} cores, ${mem ?? "?"} GB, field ${ms.toFixed(1)} ms -> ${pickCache.render}${perf ? " + performance mode" : ""}`);
  return pickCache;
}

/** field.rs sample: brightness and warm/cool mix at (x, y). */
function sample(p: Preset, x: number, y: number, aspect: number, m: Motion): [number, number] {
  sampleInto(p, x, y, aspect, m);
  return [_smpB, _smpS];
}

/** palette.rs Palette::from_image: median cut over a 64 px thumbnail. */
export async function paletteFromImage(url: string): Promise<Palette | null> {
  let img: HTMLImageElement | null = null;
  let c: HTMLCanvasElement | null = null;
  try {
    img = new Image();
    img.crossOrigin = "anonymous";
    img.src = url;
    await img.decode();
    c = document.createElement("canvas");
    c.width = c.height = 64;
    const x = c.getContext("2d", { willReadFrequently: true })!;
    x.drawImage(img, 0, 0, 64, 64);
    const d = x.getImageData(0, 0, 64, 64).data;
    const all: number[][] = [];
    for (let i = 0; i < d.length; i += 4) all.push([d[i], d[i + 1], d[i + 2]]);
    let px = all.filter((p) => { const r: Rgb = [p[0] / 255, p[1] / 255, p[2] / 255]; return luma(r) > 0.06 && (sat(r) > 0.18 || luma(r) > 0.85); });
    if (px.length < 16) px = all;
    if (!px.length) return null;
    const widest = (b: number[][]) => { const lo = [255, 255, 255], hi = [0, 0, 0]; for (const p of b) for (let k = 0; k < 3; k++) { lo[k] = Math.min(lo[k], p[k]); hi[k] = Math.max(hi[k], p[k]); } let ch = 0; for (let k = 1; k < 3; k++) if (hi[k] - lo[k] > hi[ch] - lo[ch]) ch = k; return [ch, hi[ch] - lo[ch]]; };
    const boxes = [px];
    while (boxes.length < 6) {
      let idx = 0, best = [-1, -1];
      boxes.forEach((b, i) => { const w = widest(b); if (w[1] > best[1]) { best = w; idx = i; } });
      if (best[1] < 8 || boxes[idx].length < 2) break;
      const b = boxes.splice(idx, 1)[0].sort((a, z) => a[best[0]] - z[best[0]]);
      boxes.push(b.slice(0, b.length >> 1), b.slice(b.length >> 1));
    }
    const cols = boxes.filter((b) => b.length).map((b) => {
      const s = b.reduce((a, p) => [a[0] + p[0], a[1] + p[1], a[2] + p[2]], [0, 0, 0]);
      const c: Rgb = [s[0] / b.length / 255, s[1] / b.length / 255, s[2] / b.length / 255];
      return { c, w: b.length * (0.35 + sat(c)) };
    }).sort((a, b) => b.w - a.w);
    const picked: Rgb[] = [];
    for (const { c } of cols) { if (picked.every((p) => Math.hypot(p[0] - c[0], p[1] - c[1], p[2] - c[2]) > 0.18)) picked.push(c); if (picked.length === 3) break; }
    if (!picked.length) return null;
    while (picked.length < 3) picked.push(lighten(picked[picked.length - 1], 0.35));
    picked.sort((a, b) => luma(a) - luma(b));
    const boost = (c: Rgb) => scale(c, Math.min(2.5, 0.95 / Math.max(0.05, ...c)));
    return [scale(boost(picked[0]), 0.75), boost(picked[1]), boost(picked[2])];
  } catch {
    return null;
  } finally {
    if (img) {
      img.onload = null;
      img.onerror = null;
      img.src = "";
    }
    if (c) {
      c.width = 0;
      c.height = 0;
    }
  }
}

/** palette.rs Palette::from_seed: a palette for songs without a cover. */
export function paletteFromSeed(seed: string): Palette {
  let h = 2166136261;
  for (let i = 0; i < seed.length; i++) { h ^= seed.charCodeAt(i) & 255; h = Math.imul(h, 16777619) >>> 0; }
  const hue = (h % 360) / 360;
  const c = (dh: number, s: number, l: number): Rgb => hsl(((hue + dh) % 1), s, l);
  return [c(0, 0.85, 0.3), c(0.07, 0.9, 0.55), c(0.14, 0.95, 0.72)];
}

function hsl(h: number, s: number, l: number): Rgb {
  const k = (n: number) => (n + h * 12) % 12, a = s * Math.min(l, 1 - l);
  const f = (n: number) => l - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)));
  return [f(0), f(8), f(4)];
}

export function presetFromSeed(seed: string): Preset {
  let h = 7;
  for (let i = 0; i < seed.length; i++) h = (Math.imul(h, 31) + (seed.charCodeAt(i) & 255)) >>> 0;
  return PRESETS[h % 4];
}

export interface GlowOptions {
  intensity: number;
  reactivity: number;
  throttleUnfocused: boolean;
  /** No characters / dots: plain glow only. */
  reduce: boolean;
  /** Top dot-matrix strip under the desktop titlebar. */
  titlebarDots?: boolean;
  /** Plain solid base background: no glow field, no ASCII characters, no dots. */
  solid?: boolean;
  /** Theme base colour (`0xRRGGBB`) used when `solid` is true. */
  base?: number;
  /** Performance mode: coarser field, lower frame rate, characters redrawn less often. */
  perf?: boolean;
}

/**
 * Renders a static miniature preview of a glow preset (or a 2x2 split of all
 * four presets when `preset === ""`, or plain base fill for `"solid"`) onto
 * `canvas` using the exact field math.
 */
export function renderGlowPreview(
  canvas: HTMLCanvasElement,
  preset: GlowShape,
  theme: Palette | null,
  intensity: number,
  baseHex = 0x030303,
) {
  const W = 128, H = 72;
  if (canvas.width !== W) canvas.width = W;
  if (canvas.height !== H) canvas.height = H;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  if (preset === "solid") {
    const [br, bg, bb] = hex(baseHex);
    ctx.fillStyle = `rgb(${Math.round(br * 255)}, ${Math.round(bg * 255)}, ${Math.round(bb * 255)})`;
    ctx.fillRect(0, 0, W, H);
    return;
  }
  const img = ctx.createImageData(W, H);
  const data = img.data;
  const baseRgb = hex(baseHex);
  const level = 0.5 + 0.5 * Math.min(1, Math.max(0, intensity));
  const m: Motion = { t: 4.2, dx: 0, dy: 0, size: 1, gain: 0.94, energy: 0.7 };
  const aspect = W / H;

  const drawSample = (p: Preset, px: number, py: number, ux: number, uy: number) => {
    const [cool, warm] = ramps(p, theme, null, 0);
    const x = ux * aspect, y = uy;
    let [b, s] = sample(p, x, y, aspect, m);
    const c = lerp(rampAt(cool, Math.min(b, 1)), rampAt(warm, Math.min(b, 1)), s);
    const n = noise(x * 1.7, y * 1.7, 0.19) * 0.14;
    b = Math.max(0, (b + n * Math.min(b, 1) * 1.4) * m.gain);
    const k = smooth(0.02, 1.1, b) * level * 0.62;
    const r = Math.min(1, baseRgb[0] + c[0] * k);
    const g = Math.min(1, baseRgb[1] + c[1] * k);
    const bl = Math.min(1, baseRgb[2] + c[2] * k);
    const oo = (py * W + px) * 4;
    data[oo] = r * 255;
    data[oo + 1] = g * 255;
    data[oo + 2] = bl * 255;
    data[oo + 3] = 255;
  };

  for (let py = 0; py < H; py++) {
    const uy = (py + 0.5) / H;
    for (let px = 0; px < W; px++) {
      const ux = (px + 0.5) / W;
      if (preset) {
        drawSample(preset, px, py, ux, uy);
      } else {
        const qx = ux < 0.5 ? ux * 2 : (ux - 0.5) * 2;
        const qy = uy < 0.5 ? uy * 2 : (uy - 0.5) * 2;
        const p: Preset = ux < 0.5 ? (uy < 0.5 ? "strahl" : "sonnenaufgang") : (uy < 0.5 ? "band" : "horizont");
        drawSample(p, px, py, qx, qy);
      }
    }
  }
  ctx.putImageData(img, 0, 0);
}

/** The background field. `spectrum()` gives bass / mids / highs 0..1. */
/** When a page last scrolled (any scroll area); see Glow.loop. */
let scrollAt = 0;
const SCROLL_QUIET = 160;
if (typeof document !== "undefined") {
  document.addEventListener("scroll", () => { scrollAt = performance.now(); }, { capture: true, passive: true });
}

export class Glow {
  private gctx: CanvasRenderingContext2D | null;
  private actx: CanvasRenderingContext2D | null = null;
  private agl: AsciiGl | null = null;
  /** GPU mode: the whole background on the graphics card. */
  private fgpu: FieldGpu | null = null;
  private cellFrame: CellFrame | null = null;
  private asciiFresh = false;
  private pitch = PITCH;
  private W = 0; private H = 0; private iw = 0; private ih = 0;
  private img: ImageData | null = null; private aimg: ImageData | null = null;
  private bright = new Float32Array(0);
  private cols: Float32Array = new Float32Array(0);
  /** Brightness under text is worked out from the field only when asked
   *  (`lumAt`, about twice a second), not for every cell of every frame. */
  private lumSolid = -1;
  private fLevel = 0; private fDensity = 0; private fThreshold = 0;
  private t = 0; private motionK = 1; private playingK = 0; private last = performance.now(); private frame = 0;
  private bands: [number, number, number] = [0, 0, 0];
  private pulse = 0;
  private preset: Preset = "strahl"; private prevPreset: Preset = "strahl"; private presetSince = -1e9;
  private theme: Palette | null = null; private prevTheme: Palette | null = null; private themeSince = -1e9;
  private palFrom: Palette | null = null; private palTo: Palette | null = null; private palSince = -1e9;
  private timer = 0;
  private resizeTimer = 0;
  private paused = false;
  private resizing = false;
  private uiPaused = false;
  private solidKey = "";
  private lumScratch: number[] = [];
  private onResize = () => {
    // Freeze background rendering while the window is actively resizing so we
    // don't reallocate buffers or rasterize ASCII frames on every drag event.
    this.resizing = true;
    clearTimeout(this.timer);
    this.timer = 0;
    clearTimeout(this.resizeTimer);
    this.resizeTimer = window.setTimeout(() => {
      this.resizeTimer = 0;
      this.resizing = false;
      if (document.hidden || this.opts.solid) return;
      this.resize();
      this.last = performance.now();
      if (!this.paused && !this.uiPaused) {
        this.loop();
      } else {
        this.draw();
      }
    }, 220);
  };
  private onVis = () => {
    clearTimeout(this.timer);
    this.timer = 0;
    if (document.hidden) {
      this.releaseAsciiBuffer();
      return;
    }
    this.watchLost();
    if (!this.paused && !this.resizing && !this.uiPaused && !this.opts.solid) {
      if (this.W !== Math.max(1, innerWidth) || this.H !== Math.max(1, innerHeight)) {
        this.resize();
      }
      this.last = performance.now();
      this.loop();
    }
  };
  /** GPU frames drawn since the GPU was (re)started; the second one is checked. */
  private gpuFrames = 0;
  private gpuChecked = false;
  /** When the GPU context was lost lately (window visible), to spot a card that keeps failing. */
  private losses: number[] = [];
  private lostTimer = 0;
  /** Why the graphics card was given up (the CPU draws since), or null. */
  failReason: string | null = null;
  /** The graphics card failed and the CPU took over: the app should stay on the CPU. */
  onGpuFail: ((reason: string) => void) | null = null;
  playing = false;
  opts: GlowOptions = { intensity: 0.8, reactivity: 1, throttleUnfocused: true, reduce: false, titlebarDots: true };
  spectrum: () => [number, number, number] = () => [0, 0, 0];
  /** Called after every frame (adaptive ink reads `lumAt`). */
  onFrame: (() => void) | null = null;

  /**
   * `mode` "gpu": the graphics card draws everything (WebGL 2), or at least
   * the characters (WebGL 1); "cpu": no WebGL at all. A canvas keeps the kind
   * of drawing it started with, so a new mode needs new canvases.
   */
  constructor(private glowEl: HTMLCanvasElement, private asciiEl: HTMLCanvasElement, mode: RenderMode = "gpu") {
    this.gctx = glowEl.getContext("2d");
    if (mode === "gpu") {
      // A graphics card that failed the check is not used at all (also not
      // for the characters alone): `failReason` tells the app.
      const p = gpuProbe();
      if (p.broken) this.failReason = p.reason;
      else {
        if (p.ok) this.fgpu = FieldGpu.create(asciiEl, FONT, this.onGpuChange);
        if (!this.fgpu) this.agl = AsciiGl.create(asciiEl, FONT);
      }
    }
    if (!this.fgpu && !this.agl) this.actx = asciiEl.getContext("2d");
    if (!this.gctx) return;
    this.resize();
    addEventListener("resize", this.onResize);
    document.addEventListener("visibilitychange", this.onVis);
    this.loop();
  }

  setPreset(p: Preset) {
    if (p === this.preset) return;
    this.prevPreset = this.preset; this.preset = p; this.presetSince = performance.now() / 1000;
  }
  setTheme(p: Palette | null) {
    if (JSON.stringify(p) === JSON.stringify(this.theme)) return;
    this.prevTheme = this.theme; this.theme = p; this.themeSince = performance.now() / 1000;
  }
  /** Fades to the playing song's colours (palette.rs PaletteFade). */
  setPalette(p: Palette | null) {
    if (JSON.stringify(p) === JSON.stringify(this.palTo)) return;
    const [cur, blend] = this.palCurrent();
    this.palFrom = blend > 0.5 ? cur : null;
    this.palTo = p; this.palSince = performance.now() / 1000;
  }
  /** A beat-like kick (song change). */
  kick() { this.pulse = 1; }

  private palCurrent(): [Palette | null, number] {
    const t = smooth(0, 1, Math.min(1, (performance.now() / 1000 - this.palSince) / PALETTE_FADE));
    const a = this.palFrom, b = this.palTo;
    if (a && b) return [[lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)], 1];
    if (b) return [b, t];
    if (a) return [a, 1 - t];
    return [null, 0];
  }

  /** The GPU draws right now (not lost). */
  private get gpuOn() { return !!this.fgpu && !this.fgpu.lost; }

  /** The GPU context was lost (the CPU field takes over) or came back. */
  private onGpuChange = () => {
    if (!this.gctx || !this.fgpu) return;
    clearTimeout(this.lostTimer);
    this.lostTimer = 0;
    if (this.fgpu.lost) {
      // Phones drop it in the background: only count losses in view.
      if (!document.hidden) {
        const now = performance.now();
        this.losses = this.losses.filter((at) => now - at < 120_000);
        this.losses.push(now);
        if (this.losses.length >= 3) { this.gpuFail("the graphics card kept dropping the picture"); return; }
      }
      this.watchLost();
    } else {
      // Back: check its first frames again.
      this.gpuFrames = 0;
      this.gpuChecked = false;
    }
    this.resize();
    if (!document.hidden && !this.resizing && !this.opts.solid) this.draw();
  };

  /** A lost GPU that does not come back within 5 s (window visible) is given up. */
  private watchLost() {
    if (this.lostTimer || !this.fgpu?.lost || document.hidden) return;
    this.lostTimer = window.setTimeout(() => {
      this.lostTimer = 0;
      if (this.fgpu?.lost && !document.hidden) this.gpuFail("the graphics card did not come back");
    }, 5000);
  }

  /** The graphics card failed: the CPU draws from now on (the characters
   *  come back once the app gives this background new canvases). */
  private gpuFail(reason: string) {
    if (!this.fgpu) return;
    this.failReason = reason;
    console.warn("[glow] GPU background failed, the CPU draws instead:", reason);
    markGpuBroken(reason);
    clearTimeout(this.lostTimer);
    this.lostTimer = 0;
    try { this.fgpu.destroy(); } catch { /* already gone */ }
    this.fgpu = null;
    this.cellFrame = null;
    this.resize();
    this.onGpuFail?.(reason);
  }

  private releaseAsciiBuffer() {
    this.aimg = null;
    this.asciiFresh = false;
    if (this.asciiEl.width !== 0 || this.asciiEl.height !== 0) {
      this.asciiEl.width = 0;
      this.asciiEl.height = 0;
    }
  }

  private resize() {
    if (!this.gctx) return;
    this.W = Math.max(1, innerWidth); this.H = Math.max(1, innerHeight);
    this.pitch = this.opts.perf ? 12 : PITCH;
    this.iw = Math.max(2, Math.ceil(this.W / this.pitch)); this.ih = Math.max(2, Math.ceil(this.H / this.pitch));
    if (this.gpuOn) {
      // The GPU draws light and characters into the top canvas; the small
      // CPU canvas and its buffers stay empty.
      this.glowEl.width = 1; this.glowEl.height = 1;
      this.img = null;
      this.bright = new Float32Array(0);
      this.cols = new Float32Array(0);
      if (this.opts.solid) this.releaseAsciiBuffer();
      else { this.asciiEl.width = this.W; this.asciiEl.height = this.H; }
      this.solidKey = "";
      return;
    }
    this.glowEl.width = this.iw; this.glowEl.height = this.ih;
    this.img = this.gctx.createImageData(this.iw, this.ih);
    if (this.opts.solid || this.opts.reduce || (!this.actx && !this.agl)) {
      this.releaseAsciiBuffer();
    } else if (this.agl) {
      this.asciiEl.width = this.W; this.asciiEl.height = this.H;
      this.asciiFresh = false;
    } else {
      this.asciiEl.width = this.W; this.asciiEl.height = this.H;
      this.aimg = this.actx!.createImageData(this.W, this.H);
    }
    this.bright = new Float32Array(this.iw * this.ih);
    this.cols = new Float32Array(this.iw * this.ih * 3);
    this.solidKey = "";
  }

  private loop() {
    clearTimeout(this.timer);
    this.timer = 0;
    if (!this.gctx) return;
    const tick = () => {
      if (this.paused || this.resizing || this.uiPaused || this.opts.solid || document.hidden) {
        if (document.hidden) {
          this.releaseAsciiBuffer();
        }
        return;
      }
      // Phones: the glow holds still while a page scrolls, so the scroll gets
      // all the time it needs (the glow moves too slowly for anyone to notice).
      if (performance.now() - scrollAt < SCROLL_QUIET && document.documentElement.classList.contains("compact")) {
        this.timer = window.setTimeout(tick, SCROLL_QUIET);
        return;
      }
      const slow = this.opts.throttleUnfocused && !document.hasFocus();
      const fps = this.opts.perf ? (slow ? 3 : this.playing ? 12 : 6) : slow ? 6 : this.playing ? 24 : 12;
      this.draw();
      if (!this.paused && !this.resizing && !this.uiPaused && !this.opts.solid && !document.hidden) {
        this.timer = window.setTimeout(tick, 1000 / fps);
      }
    };
    tick();
  }

  /** Pause the loop while an opaque overlay (like the startup splash) covers the canvas. */
  setPaused(p: boolean) {
    if (this.paused === p) return;
    this.paused = p;
    clearTimeout(this.timer);
    this.timer = 0;
    if (p) {
      // Ensure one frame with the current theme/palette is ready underneath.
      if (!this.resizing && !this.uiPaused) this.draw();
    } else if (!this.resizing && !this.uiPaused && !this.opts.solid && !document.hidden) {
      this.last = performance.now();
      this.loop();
    }
  }

  /** Suspend drawing cleanly during transient UI layout changes without forcing a synchronous redraw. */
  setSuspended(p: boolean) {
    if (this.uiPaused === p) return;
    this.uiPaused = p;
    clearTimeout(this.timer);
    this.timer = 0;
    if (!p && !this.paused && !this.resizing && !this.opts.solid && !document.hidden) {
      if (this.W !== Math.max(1, innerWidth) || this.H !== Math.max(1, innerHeight)) {
        this.resize();
      }
      this.last = performance.now();
      this.loop();
    }
  }

  isPaused() { return this.paused || this.resizing || this.uiPaused; }

  /** Call after changing `opts`: performance mode changes the field's grid. */
  refreshLayout() {
    const want = this.opts.perf ? 12 : PITCH;
    if (want === this.pitch || !this.gctx) return;
    this.resize();
    if (!document.hidden && !this.resizing) this.draw();
  }

  destroy() {
    clearTimeout(this.timer);
    this.timer = 0;
    clearTimeout(this.resizeTimer);
    this.resizeTimer = 0;
    clearTimeout(this.lostTimer);
    this.lostTimer = 0;
    this.onGpuFail = null;
    this.releaseAsciiBuffer();
    this.fgpu?.destroy();
    this.fgpu = null;
    removeEventListener("resize", this.onResize);
    document.removeEventListener("visibilitychange", this.onVis);
  }

  /** Displayed luminance (90th percentile) under a rect in window px. */
  lumAt(r: DOMRect): number | null {
    const P = this.pitch;
    const x0 = Math.max(0, Math.floor(r.left / P)), y0 = Math.max(0, Math.floor(r.top / P));
    const x1 = Math.min(this.iw, Math.ceil(r.right / P)), y1 = Math.min(this.ih, Math.ceil(r.bottom / P));
    const v = this.lumScratch;
    v.length = 0;
    const solidL = this.lumSolid, level = this.fLevel, density = this.fDensity, threshold = this.fThreshold;
    const bright = this.bright, cols = this.cols;
    // GPU mode: the field lives on the graphics card; these few cells are worked out here.
    const fr = this.gpuOn ? this.cellFrame : null;
    if (this.gpuOn && !fr && solidL < 0) return null;
    for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) {
      if (solidL >= 0) { v.push(solidL); continue; }
      if (fr) {
        cellInto(fr, x, y);
        v.push(cellLum(_fb, _cr, _cg, _cb, fr.level, fr.density, fr.threshold));
        continue;
      }
      const i = y * this.iw + x, ci = i * 3;
      v.push(cellLum(bright[i], cols[ci], cols[ci + 1], cols[ci + 2], level, density, threshold));
    }
    if (!v.length) return null;
    v.sort((a, b) => a - b);
    return v[Math.min(v.length - 1, Math.floor(v.length * 0.9))];
  }

  draw() {
    if (!this.gctx || (!this.img && !this.gpuOn)) return;
    const now = performance.now();
    const dt = Math.min(0.25, (now - this.last) / 1000);
    this.last = now;
    const o = this.opts;
    if (o.solid) {
      clearTimeout(this.timer);
      this.timer = 0;
      const base = o.base ?? 0x030303;
      const key = `${this.W}x${this.H}:${base}`;
      if (this.solidKey !== key) {
        const rgb = hex(base);
        this.gctx.fillStyle = `rgb(${Math.round(rgb[0] * 255)}, ${Math.round(rgb[1] * 255)}, ${Math.round(rgb[2] * 255)})`;
        this.gctx.fillRect(0, 0, this.iw, this.ih);
        this.releaseAsciiBuffer();
        this.lumSolid = luma(rgb);
        this.solidKey = key;
        this.onFrame?.();
      }
      return;
    }
    this.solidKey = "";
    const kTarget = o.intensity > 0 ? 1 : 0;
    this.motionK += (kTarget - this.motionK) * (1 - Math.exp(-dt * 3));
    this.t += dt * this.motionK;
    this.playingK += ((this.playing ? 1 : 0) - this.playingK) * (1 - Math.exp(-dt * 1.2));
    const raw = this.spectrum();
    for (let i = 0; i < 3; i++) this.bands[i] += (raw[i] - this.bands[i]) * (raw[i] > this.bands[i] ? 0.55 : 0.12);
    this.pulse = Math.max(0, this.pulse - dt * 1.8);
    const mk = this.motionK, re = o.reactivity;
    const [bass, mids, highs] = this.bands.map((b) => b * mk * re);
    const pulse = this.pulse * mk * re, playing = this.playingK * re;

    // field.rs Motion::new
    const t = this.t, breath = Math.sin(2 * Math.PI * t / 28), drift = t / 34, amp = 0.035 + 0.035 * playing;
    const m: Motion = {
      t, dx: noise(drift, 3.7, 0) * amp * 2, dy: noise(11.1, drift, 0) * amp,
      size: 1 + breath * (0.04 + 0.05 * playing) + bass * 0.32,
      gain: 0.86 + 0.06 * breath * (1 - playing) + mids * 0.38 + playing * 0.08 + pulse * 0.25,
      energy: 0.4 + 0.6 * playing,
    };
    const sec = now / 1000;
    const presetT = Math.min(1, (sec - this.presetSince) / PRESET_FADE);
    const themeT = Math.min(1, (sec - this.themeSince) / PRESET_FADE);
    const [pal, blend] = this.palCurrent();
    const themed = (p: Preset): [Ramp, Ramp] => {
      const cur = ramps(p, this.theme, pal, blend);
      if (themeT >= 1) return cur;
      const prev = ramps(p, this.prevTheme, pal, blend);
      return [rampLerp(prev[0], cur[0], themeT), rampLerp(prev[1], cur[1], themeT)];
    };
    const [cool, warm] = themed(this.preset);
    const prev = presetT < 1 ? themed(this.prevPreset) : null;
    const level = 0.5 + 0.5 * o.intensity;
    const density = 0.75 + 0.45 * highs + 0.08 * playing;
    const threshold = 0.42 - 0.06 * highs;
    const dotLevel = 0.18 + 0.14 * o.intensity + 0.28 * bass + 0.1 * pulse;
    const sh = 0.1 + 0.08 * m.energy;

    // GPU: hand the numbers over; the graphics card does every cell.
    if (this.gpuOn) {
      const { iw, ih, W, H } = this, aspect = W / H;
      if (this.asciiEl.width !== W || this.asciiEl.height !== H) { this.asciiEl.width = W; this.asciiEl.height = H; }
      const pr: [Ramp, Ramp] = prev ?? [cool, warm];
      this.lumSolid = -1;
      const cf: CellFrame = { preset: this.preset, prevPreset: this.prevPreset, presetT: prev ? presetT : 1, cool, warm, prev, m, t, sh, level, density, threshold, aspect, iw, ih };
      this.cellFrame = cf;
      // Some graphics cards pass the small check and still fail on the real
      // window (errors, a black picture): the second frame is checked against
      // the CPU maths too, and any error hands over to the CPU.
      let why: string | null = null;
      try {
        this.fgpu!.draw({
          W, H, iw, ih, pitch: this.pitch, aspect,
          preset: PRESETS.indexOf(this.preset), prevPreset: PRESETS.indexOf(this.prevPreset), presetT: prev ? presetT : 1,
          cool, warm, prevCool: pr[0], prevWarm: pr[1],
          dx: m.dx, dy: m.dy, size: m.size, gain: m.gain, t, sh, level,
          density, threshold, dotLevel, blend,
          ascii: !o.reduce, dots: !o.reduce && o.titlebarDots !== false,
        });
        if (!this.gpuChecked && ++this.gpuFrames >= 2) {
          this.gpuChecked = true;
          why = verifyGpu(this.fgpu!, cf, H, this.pitch);
        }
      } catch (e) {
        why = e instanceof Error ? e.message : String(e);
      }
      if (why && !this.fgpu!.lost) {
        this.gpuFail(why);
        this.draw();
        return;
      }
      this.onFrame?.();
      return;
    }

    if (!this.img) return;
    const { iw, ih, W, H } = this, aspect = W / H, data = this.img.data;
    const cols = this.cols, bright = this.bright;
    this.lumSolid = -1; this.fLevel = level; this.fDensity = density; this.fThreshold = threshold;
    for (let py = 0; py < ih; py++) {
      const y = (py + 0.5) / ih;
      for (let px = 0; px < iw; px++) {
        const x = (px + 0.5) / iw * aspect;
        sampleInto(this.preset, x, y, aspect, m);
        let b = _smpB;
        sampleMixRgb(cool, warm, b, _smpS);
        let c0 = _cr, c1 = _cg, c2 = _cb;
        if (prev) {
          sampleInto(this.prevPreset, x, y, aspect, m);
          const pb = _smpB;
          sampleMixRgb(prev[0], prev[1], pb, _smpS);
          b = pb + (b - pb) * presetT;
          c0 = _cr + (c0 - _cr) * presetT;
          c1 = _cg + (c1 - _cg) * presetT;
          c2 = _cb + (c2 - _cb) * presetT;
        }
        const n = noise(x * 1.7, y * 1.7, t * 0.045) * sh;
        b = Math.max(0, (b + n * Math.min(b, 1) * 1.4) * m.gain);
        const k = smooth(0.02, 1.1, b) * level * 0.5, i = py * iw + px, oo = i * 4, ci = i * 3;
        data[oo] = c0 * k * 255; data[oo + 1] = c1 * k * 255; data[oo + 2] = c2 * k * 255; data[oo + 3] = 255;
        bright[i] = b; cols[ci] = c0; cols[ci + 1] = c1; cols[ci + 2] = c2;
      }
    }
    this.gctx.putImageData(this.img, 0, 0);

    // GPU path: the shader draws characters and dots from the field.
    if (this.agl && !this.agl.lost) {
      const every = o.perf ? 3 : 2;
      if (o.reduce) {
        if (this.asciiFresh) { this.agl.clear(); this.asciiFresh = false; }
      } else if (!this.asciiFresh || this.frame++ % every === 0) {
        if (this.asciiEl.width !== W || this.asciiEl.height !== H) { this.asciiEl.width = W; this.asciiEl.height = H; }
        this.agl.draw({ W, H, iw, ih, pitch: this.pitch, cols, bright, density, threshold, level, dotLevel, blend, dots: o.titlebarDots !== false });
        this.asciiFresh = true;
      }
      this.onFrame?.();
      return;
    }
    if (this.agl) { this.onFrame?.(); return; }

    // ASCII layer and titlebar dots at half cadence (6-12 fps) without per-pixel tuple allocations.
    if ((!this.aimg || this.frame++ % 2 === 0) && !o.reduce && this.actx) {
      if (!this.aimg || this.aimg.width !== W || this.aimg.height !== H) {
        this.asciiEl.width = W; this.asciiEl.height = H;
        this.aimg = this.actx.createImageData(W, H);
      }
      const a = this.aimg.data;
      a.fill(0);
      const P = this.pitch;
      const at = (lx: number, ly: number) => Math.min(ih - 1, (ly / P) | 0) * iw + Math.min(iw - 1, (lx / P) | 0);
      const put = (x: number, y: number, r: number, g: number, b: number, al: number) => {
        if (x < 0 || y < 0 || x >= W || y >= H) return;
        const q = (y * W + x) * 4;
        a[q] = r; a[q + 1] = g; a[q + 2] = b; a[q + 3] = al;
      };
      const showDots = o.titlebarDots !== false;
      const ncols = (W / CW) | 0, nrows = ((H / CH) | 0) + 1;
      for (let row = 0; row < nrows; row++) {
        const ly = row * CH;
        if ((showDots && ly < TITLEBAR_H) || ly + CH > H) continue;
        for (let col = 0; col < ncols; col++) {
          const lx = col * CW, i = at(lx + CW / 2, ly + CH / 2), dv = bright[i] * density;
          if (dv < threshold) continue;
          const idx = Math.min(RAMP_LEN - 1, Math.max(1, Math.round(Math.min(dv, 1) * (RAMP_LEN - 1))));
          const ci = i * 3;
          const r255 = (cols[ci] + (1 - cols[ci]) * 0.3) * 255;
          const g255 = (cols[ci + 1] + (1 - cols[ci + 1]) * 0.3) * 255;
          const b255 = (cols[ci + 2] + (1 - cols[ci + 2]) * 0.3) * 255;
          const al255 = (0.16 + 0.16 * Math.min(dv, 1)) * level * 255;
          const ox = lx + 2, oy = ly + 4, glyph = FONT[idx];
          for (let ry = 0; ry < 7; ry++) { const bits = glyph[ry]; if (bits) for (let rx = 0; rx < 5; rx++) if (bits & (16 >> rx)) put(ox + rx, oy + ry, r255, g255, b255, al255); }
        }
      }
      // Titlebar dot grid: 2 px dots, 4 px pitch, glow colour along the top.
      if (showDots) {
        const floor = hex(0x0c1a6b), dotsY = (TITLEBAR_H / 4) | 0;
        const fl0 = floor[0], fl1 = floor[1], fl2 = floor[2];
        for (let i = 0; i < W / 4; i++) {
          const lx = i * 4 + 1, s = at(lx, 6), b = bright[s], mt = Math.max(smooth(0, 0.6, b), blend * 0.55);
          const ci = s * 3;
          const tr = (fl0 + (cols[ci] - fl0) * mt) * 255;
          const tg = (fl1 + (cols[ci + 1] - fl1) * mt) * 255;
          const tb = (fl2 + (cols[ci + 2] - fl2) * mt) * 255;
          for (let k = 0; k < dotsY; k++) {
            const al = Math.min(1, dotLevel * (1 - (k / dotsY) * 0.35)) * 255, y = k * 4 + 1;
            put(lx, y, tr, tg, tb, al); put(lx + 1, y, tr, tg, tb, al); put(lx, y + 1, tr, tg, tb, al); put(lx + 1, y + 1, tr, tg, tb, al);
          }
        }
      }
      this.actx.putImageData(this.aimg, 0, 0);
    } else if (o.reduce && this.frame === 1 && this.actx) {
      this.actx.clearRect(0, 0, W, H);
    }
    this.onFrame?.();
  }
}
