// Active track-list keyboard navigation registry (↑ / ↓ / Enter / Delete).
// Pages with playable track lists register their active handler while mounted.

export interface ListNavHandler {
  /** Number of navigable track rows currently rendered. */
  count: () => number;
  /** Plays (or activates) row `index`. */
  play: (index: number) => void;
  /** Optional removal on Delete/Backspace (only on editable lists). */
  remove?: (index: number) => void;
  /** Optional selection toggle when pick mode is active. */
  togglePick?: (index: number, shift: boolean) => void;
  /** True while the list is in multi-select mode. */
  isPicking?: () => boolean;
  /** Scrolls `#main-scroll` so row `index` is comfortably visible. */
  scrollTo?: (index: number) => void;
}

let activeHandler: ListNavHandler | null = null;

export function registerListNav(handler: ListNavHandler): () => void {
  activeHandler = handler;
  return () => {
    if (activeHandler === handler) activeHandler = null;
  };
}

export function currentListNav(): ListNavHandler | null {
  return activeHandler;
}

/**
 * Computes the next `selectedRow` index for an Up/Down step, or `null` if empty.
 */
export function stepRowIndex(current: number | null, delta: 1 | -1, count: number): number | null {
  if (count <= 0) return null;
  if (current == null || current < 0 || current >= count) {
    return delta > 0 ? 0 : count - 1;
  }
  return Math.max(0, Math.min(count - 1, current + delta));
}

/**
 * Scrolls `#main-scroll` so a virtualized row at `index * pitch` inside `listEl`
 * stays comfortably inside the viewport (respecting sticky header headroom).
 */
export function scrollVirtualRowIntoView(listEl: HTMLElement | undefined, index: number, pitch = 56, topMargin = 104, bottomMargin = 32) {
  const sc = typeof document !== "undefined" ? document.getElementById("main-scroll") : null;
  if (!sc || !listEl) return;
  const scRect = sc.getBoundingClientRect();
  const listRect = listEl.getBoundingClientRect();
  const rowTop = listRect.top - scRect.top + sc.scrollTop + index * pitch;
  const rowBottom = rowTop + pitch;
  const viewTop = sc.scrollTop + topMargin;
  const viewBottom = sc.scrollTop + sc.clientHeight - bottomMargin;
  if (rowTop < viewTop) {
    sc.scrollTo({ top: Math.max(0, rowTop - topMargin), behavior: "instant" as ScrollBehavior });
  } else if (rowBottom > viewBottom) {
    sc.scrollTo({ top: Math.max(0, rowBottom - sc.clientHeight + bottomMargin), behavior: "instant" as ScrollBehavior });
  }
}

/** Scrolls a non-virtualized row element matching `[data-nav-row="i"]` into view. */
export function scrollDomRowIntoView(container: HTMLElement | undefined, index: number) {
  const el = container?.querySelector<HTMLElement>(`[data-nav-row="${index}"]`);
  el?.scrollIntoView({ block: "nearest" });
}
