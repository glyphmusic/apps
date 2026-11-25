// Order and hidden parts of the home page, the sidebar and the phone tabs,
// from plugin looks (skin.ts, glyph_core::plugins::LayoutSpec). Home.svelte,
// Sidebar.svelte and TabBar.svelte read `arrange`. Settings always stays in
// the sidebar, so a look can always be turned off.

export interface LayoutSpec { order?: string[]; hide?: string[] }
export type LayoutArea = "home" | "sidebar" | "tabs";

export const layout = $state<{ areas: Partial<Record<LayoutArea, LayoutSpec>>; sidebarRight: boolean }>({ areas: {}, sidebarRight: false });

/**
 * Sets the layouts in use (first look per area wins) and where the sidebar
 * sits (right: the side panel opens on the left); empty brings Glyph's back.
 */
export function setLayouts(areas: Partial<Record<LayoutArea, LayoutSpec>>, sidebarRight = false) {
  layout.areas = areas;
  layout.sidebarRight = sidebarRight;
}

/**
 * `parts` (Glyph's order) as the look wants them: the parts it lists in
 * `order` first, the rest after in Glyph's order, without the hidden ones.
 * Names this version does not know are skipped. When a look would hide every
 * tab, Glyph's tabs stay.
 */
export function arrange<T extends string>(parts: readonly T[], spec: LayoutSpec | undefined, keepOne = false): T[] {
  if (!spec) return [...parts];
  const known = new Set<string>(parts);
  const hide = new Set((spec.hide ?? []).filter((n) => known.has(n)));
  const first = (spec.order ?? []).filter((n, i, a): n is T => known.has(n) && a.indexOf(n) === i);
  const out = [...first, ...parts.filter((p) => !first.includes(p))].filter((p) => !hide.has(p));
  return keepOne && !out.length ? [...parts] : out;
}
