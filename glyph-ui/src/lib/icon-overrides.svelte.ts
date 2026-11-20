// Icons a plugin's look replaces (skin.ts): icon name -> CSS mask address.
// Icon.svelte reads these before Glyph's own set. Drawn as a mask in the text
// colour, like Glyph's icons; an SVG used as a mask never runs anything.
export const iconOv = $state<{ masks: Record<string, string> }>({ masks: {} });

/** Sets the replaced icons (name -> SVG text); empty brings Glyph's back. */
export function setIconOverrides(svgs: Record<string, string>) {
  const masks: Record<string, string> = {};
  for (const [name, svg] of Object.entries(svgs)) {
    if (!/^[a-z0-9-]{1,40}$/.test(name) || !svg.includes("<svg")) continue;
    // Comments out until none is left (a removed one can join a new "<!--").
    let plain = svg;
    for (let before = ""; before !== plain; ) {
      before = plain;
      plain = plain.replace(/<!--[\s\S]*?-->/g, "").replace(/<!--/g, "");
    }
    const solid = plain.replace(/currentColor/g, "#fff").replace(/\s+/g, " ").trim();
    masks[name] = `url("data:image/svg+xml,${encodeURIComponent(solid)}")`;
  }
  iconOv.masks = masks;
}
