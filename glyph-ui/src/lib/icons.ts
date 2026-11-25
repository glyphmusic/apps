// The app's own icons (glyph-apps/assets/icons/ui, see docs/ICONS.md).
// Rendered via CSS `mask-image` with opaque `#fff` SVG geometry so overlapping
// strokes composite to a uniform silhouette before `currentColor` (including
// translucent tokens like `--dim` or `--faint`) is applied.
const files = import.meta.glob("$assets/icons/ui/*.svg", { query: "?raw", import: "default", eager: true }) as Record<string, string>;

const icons: Record<string, string> = {};
const masks: Record<string, string> = {};
for (const [path, src] of Object.entries(files)) {
  const name = path.split("/").pop()!.replace(/\.svg$/, "");
  // Repeat until nothing is left, so nested or broken comments cannot slip through.
  let clean = src;
  for (let prev = ""; prev !== clean; ) {
    prev = clean;
    clean = clean.replace(/<!--[\s\S]*?-->/g, "").replace(/<!--/g, "");
  }
  clean = clean.trim();
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
