/** Who wins when two plugins change the same thing (no app state here, so it can be tested). */

export type Named = { id: string; name: string };

/** Plugins in the order Glyph applies them (like the backend): the order
 *  the user picked first, the rest after it by name. */
export function sortLikeGlyph<T extends Named>(v: T[], order: string[]): T[] {
  const pos = (p: Named) => { const i = order.indexOf(p.id); return i < 0 ? 1e9 : i; };
  const n = (p: Named) => p.name.toLowerCase();
  return [...v].sort((a, b) => (n(a) < n(b) ? -1 : n(a) > n(b) ? 1 : 0)).sort((a, b) => pos(a) - pos(b));
}

/** The order after the user picked `id` to win over `over` (like the backend). */
export function preferOrder(ids: string[], id: string, over: string): string[] {
  const v = ids.filter((x) => x !== id);
  const at = Math.max(0, v.indexOf(over));
  v.splice(at, 0, id);
  return v;
}
