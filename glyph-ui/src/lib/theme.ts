// Colour themes (src/theme.rs THEMES): accent, glow colours while no song
// colours take over, and a faint tint in the black base.
export interface ThemeColors { accent: number; glow: [number, number, number] | null; base: number }
export interface Theme extends ThemeColors { id: string; name: string }

export const THEMES: Theme[] = [
  { id: "glyph", name: "theme.glyph", accent: 0xff5fd2, glow: null, base: 0x030303 },
  { id: "ember", name: "theme.ember", accent: 0xff8a3d, glow: [0x3a1206, 0xff6516, 0xffc04a], base: 0x050302 },
  { id: "ocean", name: "theme.ocean", accent: 0x4da3ff, glow: [0x06124a, 0x1d5cff, 0x46dcff], base: 0x020308 },
  { id: "forest", name: "theme.forest", accent: 0x3ddc84, glow: [0x04210f, 0x1f9d55, 0x9cf0b8], base: 0x020503 },
  { id: "violet", name: "theme.violet", accent: 0xb07cff, glow: [0x16052f, 0x6526d6, 0xb88dff], base: 0x040208 },
  { id: "crimson", name: "theme.crimson", accent: 0xff4d5e, glow: [0x360404, 0xb11d0b, 0xff6a4d], base: 0x060202 },
  { id: "mono", name: "theme.mono", accent: 0xe8e8e8, glow: [0x101010, 0x5a5a5a, 0xd0d0d0], base: 0x030303 },
];

/** Themes from installed plugins (`plugin:<id>`). */
let pluginThemes: Theme[] = [];
export function setPluginThemes(t: Theme[]) { pluginThemes = t; }
export function allThemes(): Theme[] { return [...THEMES, ...pluginThemes]; }

export function themeColors(id: string): ThemeColors {
  return pluginThemes.find((t) => t.id === id) ?? THEMES.find((t) => t.id === id) ?? THEMES[0];
}

export const css = (h: number) => `#${h.toString(16).padStart(6, "0")}`;
/** The solid background's colour (`0xRRGGBB`): the one picked in Settings >
 *  Appearance, else the theme's base colour. */
export function solidBase(v: { theme?: string; solid_color?: string } | undefined): number {
  const m = /^#?([0-9a-f]{6})$/i.exec(v?.solid_color ?? "");
  return m ? parseInt(m[1], 16) : themeColors(v?.theme || "glyph").base;
}

const DEFAULT_ACCENT = 0xff5fd2;
let baseIconPromise: Promise<ImageData | null> | null = null;
let faviconCanvas: HTMLCanvasElement | null = null;
let lastFaviconAccent: number | null = null;
let faviconSeq = 0;

function loadBaseFavicon(): Promise<ImageData | null> {
  if (baseIconPromise) return baseIconPromise;
  baseIconPromise = new Promise((resolve) => {
    const img = new Image();
    img.onload = () => {
      const c = document.createElement("canvas");
      c.width = img.naturalWidth || 64;
      c.height = img.naturalHeight || 64;
      const ctx = c.getContext("2d");
      if (!ctx) return resolve(null);
      ctx.drawImage(img, 0, 0);
      faviconCanvas = c;
      resolve(ctx.getImageData(0, 0, c.width, c.height));
    };
    img.onerror = () => resolve(null);
    img.src = `${import.meta.env.BASE_URL}icon-64.png`;
  });
  return baseIconPromise;
}

function updateFavicon(accent: number) {
  if (typeof document === "undefined" || lastFaviconAccent === accent) return;
  lastFaviconAccent = accent;
  const link = document.querySelector<HTMLLinkElement>('link[rel="icon"]');
  if (!link) return;
  if (accent === DEFAULT_ACCENT) {
    faviconSeq++;
    link.href = `${import.meta.env.BASE_URL}icon-64.png`;
    return;
  }
  const seq = ++faviconSeq;
  loadBaseFavicon().then((base) => {
    if (!base || !faviconCanvas || seq !== faviconSeq) return;
    const ctx = faviconCanvas.getContext("2d");
    if (!ctx) return;
    const out = new Uint8ClampedArray(base.data);
    const tr = (accent >> 16) & 255, tg = (accent >> 8) & 255, tb = accent & 255;
    const mix = 0.72;
    const sr = (mix * tr + (1 - mix) * 255) / 255;
    const sg = (mix * tg + (1 - mix) * 255) / 255;
    const sb = (mix * tb + (1 - mix) * 255) / 255;
    for (let i = 0; i < out.length; i += 4) {
      const a = out[i + 3];
      if (!a) continue;
      const r = out[i], g = out[i + 1], b = out[i + 2];
      if (r <= g || b <= g) continue;
      const intensity = Math.min(r, (r - g) / 0.3125, (b - g) / 0.256);
      if (intensity <= 0.5) continue;
      const white = Math.max(0, r - intensity);
      out[i] = Math.min(255, Math.max(0, Math.round(white + intensity * sr)));
      out[i + 1] = Math.min(255, Math.max(0, Math.round(white + intensity * sg)));
      out[i + 2] = Math.min(255, Math.max(0, Math.round(white + intensity * sb)));
    }
    ctx.putImageData(new ImageData(out, base.width, base.height), 0, 0);
    link.href = faviconCanvas.toDataURL("image/png");
  });
}

/** Sets the CSS tokens of theme `id`, the favicon, theme-color meta, and the text size factor. */
export function applyTheme(id: string, textScale = 1) {
  const c = themeColors(id), s = document.documentElement.style;
  const baseHex = css(c.base);
  s.setProperty("--accent", css(c.accent));
  s.setProperty("--bg", baseHex);
  s.setProperty("--k", String(Math.min(1.3, Math.max(0.9, textScale || 1))));
  const r = (c.accent >> 16) & 255, g = (c.accent >> 8) & 255, b = c.accent & 255;
  s.setProperty("--accent-rgb", `${r}, ${g}, ${b}`);
  const meta = document.querySelector<HTMLMetaElement>('meta[name="theme-color"]');
  if (meta) meta.content = baseHex;
  updateFavicon(c.accent);
}

/** Dark tile in the accent's hue (Liked Songs, Downloads covers): theme.rs accent_tile. */
export function accentTile(l: number, s: number, accent: number = 0xff5fd2): string {
  const r = ((accent >> 16) & 255) / 255, g = ((accent >> 8) & 255) / 255, b = (accent & 255) / 255;
  const mx = Math.max(r, g, b), mn = Math.min(r, g, b), d = mx - mn;
  let h = 0;
  if (d) h = mx === r ? ((g - b) / d) % 6 : mx === g ? (b - r) / d + 2 : (r - g) / d + 4;
  const sat = mx === 0 ? 0 : d / (1 - Math.abs(mx + mn - 1));
  return `hsl(${Math.round(h * 60 + 360) % 360} ${sat < 0.05 ? 0 : s * 100}% ${l * 100}%)`;
}
