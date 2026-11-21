// Pure math helpers for drag-to-reorder track rows in playlists.

export function dragShift(
  index: number,
  drag: { from: number; slot: number } | null,
  pitch = 56,
): number {
  if (!drag || index === drag.from) return 0;
  const { from, slot } = drag;
  if (from < slot && index > from && index <= slot) return -pitch;
  if (slot < from && index >= slot && index < from) return pitch;
  return 0;
}

export function clampDragSlot(pointerY: number, listTop: number, count: number, pitch = 56): number {
  if (count <= 0) return 0;
  return Math.max(0, Math.min(count - 1, Math.floor((pointerY - listTop) / pitch)));
}
