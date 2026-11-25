// The app's own icons (glyph-apps/assets/icons/ui, see docs/ICONS.md).
// Rendered via CSS `mask-image` with opaque `#fff` SVG geometry so overlapping
// strokes composite to a uniform silhouette before `currentColor` (including
// translucent tokens like `--dim` or `--faint`) is applied.
const files = import.meta.glob("$assets/icons/ui/*.svg", { query: "?raw", import: "default", eager: true }) as Record<string, string>;

/** `src` without `<!-- … -->` comments; an unclosed one cuts off the rest.
 *  Repeats until none is left, so joined pieces cannot form a new one. */
function stripComments(src: string): string {
  let s = src;
  while (s.includes("<!--")) s = stripOnce(s);
  return s;
}

function stripOnce(src: string): string {
  let out = "";
  let i = 0;
  for (;;) {
    const start = src.indexOf("<!--", i);
    if (start < 0) return out + src.slice(i);
    out += src.slice(i, start);
    const end = src.indexOf("-->", start + 4);
    if (end < 0) return out;
    i = end + 3;
  }
}

const icons: Record<string, string> = {};
const masks: Record<string, string> = {};
for (const [path, src] of Object.entries(files)) {
  const name = path.split("/").pop()!.replace(/\.svg$/, "");
  const clean = stripComments(src).trim();
  // Size only the outer <svg>; rects inside keep their width / height.
  icons[name] = clean.replace(/<svg\b[^>]*>/, (tag) =>
    tag.replace(/\s(width|height|class)="[^"]*"/g, "").replace("<svg", '<svg aria-hidden="true" width="100%" height="100%"'),
  );
  const solid = clean.replace(/currentColor/g, "#fff").replace(/\s+/g, " ").replace(/>\s+</g, "><");
  masks[name] = `url("data:image/svg+xml,${encodeURIComponent(solid)}")`;
}

/** SVG markup of icon `name` (empty when unknown). */
export function iconSvg(name: string): string {
  return icons[name] ?? "";
}

/** CSS `mask-image` URL for icon `name` (empty when unknown). */
export function iconMask(name: string): string {
  return masks[name] ?? "";
}

/** Toggle icons that have a filled "on" variant (widgets/buttons.rs has_fill). */
export function hasFill(name: string): boolean {
  return ["heart", "list-music", "picture-in-picture-2", "layout-split", "thumbs-up", "pin"].includes(name);
}
