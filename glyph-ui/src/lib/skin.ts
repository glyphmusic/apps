// Looks of enabled plugins (glyph_core::plugins::LookSpec): font, corner
// roundness, replaced icons and a style sheet for the whole interface
// (every window, menu, dialog, toast and the mini player). The backend has
// checked the style sheet (only the look's own files in url(), sent here
// inlined as data: addresses; no @import; balanced braces) and checks again
// here before it goes in.
//
// App.svelte pauses every look while Settings › Plugins or a plugin's
// install warning is open: there a look can always be turned off, and the
// plugin warnings always look like Glyph.
//
// Hooks for plugin makers that stay stable: `[data-skin]` (the main window),
// `[data-part="titlebar" | "sidebar" | "page" | "side-panel" | "player-bar" |
// "settings" | "menu" | "dialog" | "toasts" | "tooltip" | "mini-player" |
// "tab-bar" | "mini-bar" | "now-playing"]`.
import { setIconOverrides } from "./icon-overrides.svelte";
import { setCornerScale, type CornerPart } from "./corners";
import { setLayouts, type LayoutArea, type LayoutSpec } from "./layout.svelte";
import type { SplashSpec } from "./overlays/splash-art";

export interface PluginLook {
  plugin: string; font: string; radius: number | null; radii?: Record<string, number>; css: string;
  /** The plugin's own text font as a data: address (font: ""). */
  font_data?: string | null;
  /** Monospaced font: Glyph's font name, or the plugin's own as mono_data. */
  mono?: string; mono_data?: string | null;
  /** Order and hidden parts of the home page, sidebar and phone tabs. */
  home?: LayoutSpec | null; sidebar?: LayoutSpec | null; tabs?: LayoutSpec | null;
  /** "left" (Glyph's) or "right", "" = not set. */
  sidebar_position?: string;
  /** Own startup animation (splash-art.ts). */
  splash?: SplashSpec | null;
  /** Replaced icons: name -> SVG text. */
  icons?: Record<string, string>;
  /** A theme plugin's look: only while its theme is picked. */
  theme?: boolean;
}

/** The looks in use: extensions' always, a theme's only while it is the theme. */
export function activeLooks(looks: PluginLook[], theme: string): PluginLook[] {
  return looks.filter((l) => !l.theme || theme === `plugin:${l.plugin}`);
}

/** System font stacks; a style sheet can bring its own with @font-face. */
const FONTS: Record<string, string> = {
  inter: 'Inter, system-ui, -apple-system, "Segoe UI", sans-serif',
  system: 'system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, Cantarell, sans-serif',
  rounded: 'ui-rounded, "SF Pro Rounded", "Nunito", "Varela Round", "Quicksand", system-ui, sans-serif',
  serif: 'ui-serif, "New York", Georgia, Cambria, "Times New Roman", serif',
  mono: '"JetBrains Mono", Consolas, "Cascadia Mono", Menlo, monospace',
};
const BASE_RADIUS = 10;

let styleEl: HTMLStyleElement | null = null;

/** A plugin's own font file, in use everywhere: its @font-face and the stack. */
function ownFont(plugin: string, kind: "text" | "mono", data: string): { face: string; stack: string } | null {
  if (!data.startsWith("data:font/") || /["\\<\s()]/.test(data)) return null;
  const family = `glyph-look-${kind}-${plugin.replace(/[^a-z0-9-]/g, "")}`;
  const fallback = kind === "mono" ? FONTS.mono : FONTS.inter;
  return {
    face: `@font-face { font-family: "${family}"; src: url("${data}"); font-weight: 100 900; font-style: normal; font-display: block; }`,
    stack: `"${family}", ${fallback}`,
  };
}

/** The first look's font of `kind`: (stack, @font-face or ""). */
function pickFont(looks: PluginLook[], kind: "text" | "mono"): { face: string; stack: string } | null {
  for (const l of looks) {
    const name = kind === "text" ? l.font : l.mono;
    const data = kind === "text" ? l.font_data : l.mono_data;
    if (data) {
      const own = ownFont(l.plugin, kind, data);
      if (own) return own;
    }
    if (name && FONTS[name]) return { face: "", stack: FONTS[name] };
  }
  return null;
}

/** Applies the looks (plugins sorted by name; the first font / radius / icon wins). */
export function applyLooks(looks: PluginLook[]) {
  if (typeof document === "undefined") return;
  const root = document.documentElement.style;
  const font = pickFont(looks, "text");
  if (font) root.setProperty("--font", font.stack);
  else root.removeProperty("--font");
  const mono = pickFont(looks, "mono");
  if (mono) root.setProperty("--mono", mono.stack);
  else root.removeProperty("--mono");
  const faces = [font?.face, mono?.face].filter((f): f is string => !!f);
  const radius = looks.find((l) => l.radius != null && Number.isFinite(l.radius))?.radius;
  if (radius != null) {
    const k = Math.min(2, Math.max(0, radius));
    root.setProperty("--radius", `${Math.round(BASE_RADIUS * k * 10) / 10}px`);
    root.setProperty("--radius-k", String(k));
  } else {
    root.removeProperty("--radius");
    root.removeProperty("--radius-k");
  }
  // Single parts (radii): for each part the first look that names it wins.
  const parts: Partial<Record<CornerPart, number>> = {};
  for (const l of [...looks].reverse()) {
    for (const [p, v] of Object.entries(l.radii ?? {})) if (Number.isFinite(v)) parts[p as CornerPart] = v;
  }
  setCornerScale(radius ?? 1, parts);
  const icons: Record<string, string> = {};
  for (const l of [...looks].reverse()) Object.assign(icons, l.icons ?? {});
  setIconOverrides(icons);
  const areas: Partial<Record<LayoutArea, LayoutSpec>> = {};
  for (const a of ["home", "sidebar", "tabs"] as const) {
    const spec = looks.find((l) => l[a])?.[a];
    if (spec) areas[a] = spec;
  }
  const side = looks.find((l) => l.sidebar_position === "left" || l.sidebar_position === "right")?.sidebar_position;
  setLayouts(areas, side === "right");
  const sheets = looks.filter((l) => l.css.trim() && safe(l.css));
  if (!sheets.length && !faces.length) {
    styleEl?.remove();
    styleEl = null;
    return;
  }
  if (!styleEl) {
    styleEl = document.createElement("style");
    styleEl.id = "glyph-plugin-looks";
  }
  // Last in <head>: after Glyph's own styles, so equal rules go to the look.
  document.head.appendChild(styleEl);
  // Fonts first: a style sheet can still set --font itself.
  styleEl.textContent = [...faces, ...sheets.map((l) => `/* plugin: ${l.plugin} */\n${l.css}`)].join("\n\n");
}

/** Last check before the sheet goes in: url() only with inlined data, nothing else that loads, balanced braces. */
function safe(css: string): boolean {
  const low = css.toLowerCase();
  if (["@import", "image-set(", "image(", "src(", "element(", "expression(", "javascript:", "\\", "<"].some((b) => low.includes(b))) return false;
  for (let i = low.indexOf("url("); i >= 0; i = low.indexOf("url(", i + 4)) {
    if (!css.startsWith('url("data:', i)) return false;
  }
  let depth = 0;
  for (const ch of css.replace(/\/\*[\s\S]*?\*\//g, "").replace(/"[^"\n]*"|'[^'\n]*'/g, "")) {
    if (ch === "{") depth++;
    else if (ch === "}" && --depth < 0) return false;
  }
  return depth === 0;
}
