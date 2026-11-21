// Pure threshold/snap math for the mobile playlist pull-to-reveal search & sort bar.

/** Height of the translucent mobile backbar (`Compact.svelte .backbar.over`). */
export const REVEAL_TOP_BAR = 52;
/** Height of the search + sort bar row sitting beneath the backbar when revealed. */
export const REVEAL_BAR_H = 56;
/** Resting scroll offset for lists without cover art (keeps h1 at y = 56 px). */
export const REVEAL_NO_ART_H = 60;

/**
 * Resting scrollTop ("Schwellenpunkt") so a mobile list opens in its exact
 * default position while pulling/scrolling up reveals the search + sort bar.
 */
export function revealHeight(hasBar: boolean, noArt: boolean): number {
  if (!hasBar) return 0;
  return noArt ? REVEAL_NO_ART_H : REVEAL_TOP_BAR + REVEAL_BAR_H;
}

/**
 * Decides where to snap once touch/scroll settles inside the threshold band:
 * - `0`: reveal bar fully open
 * - `revealH`: default resting position (bar hidden)
 * - `null`: already settled at/beyond a stable position
 */
export function revealSnapTarget(opts: {
  scrollTop: number;
  revealH: number;
  keepOpen: boolean;
  startedDeep: boolean;
}): number | null {
  const { scrollTop, revealH, keepOpen, startedDeep } = opts;
  if (revealH <= 0) return null;
  if (keepOpen) {
    return scrollTop > 1 && scrollTop < revealH ? 0 : null;
  }
  if (startedDeep && scrollTop < revealH - 1) {
    return revealH;
  }
  if (scrollTop <= 1 || scrollTop >= revealH - 1) return null;
  return scrollTop <= revealH * 0.5 ? 0 : revealH;
}

/** Opacity (0..1) of the reveal bar between `scrollTop = 0` (open) and `revealH` (hidden). */
export function revealOpacity(scrollTop: number, revealH: number): number {
  if (revealH <= 0) return 0;
  const fadeRange = Math.max(1, revealH * 0.85);
  return Math.min(1, Math.max(0, 1 - scrollTop / fadeRange));
}

/**
 * Computes the scrollTop to apply when a mobile list opens or when its
 * pull-to-reveal bar height changes (e.g. after async tracks finish loading
 * or when toggling multi-select pick mode):
 * - A new list always starts at its resting threshold (`nextRevealH`) so the
 *   search + sort bar stays hidden by default.
 * - On the same list, when the bar mounts (`0 -> nextRevealH`) or changes
 *   height, shift `currentScrollTop` by the delta so content stays anchored
 *   and a list at the top (`0`) settles at `nextRevealH` (hidden) instead of
 *   exposing the bar.
 * - Returns `null` when neither the list nor `revealH` changed.
 */
export function revealSettleScroll(opts: {
  isNewList: boolean;
  prevRevealH: number;
  nextRevealH: number;
  currentScrollTop: number;
}): number | null {
  const { isNewList, prevRevealH, nextRevealH, currentScrollTop } = opts;
  if (isNewList) return nextRevealH;
  if (prevRevealH === nextRevealH) return null;
  return Math.max(0, currentScrollTop + (nextRevealH - prevRevealH));
}
