// While a sheet slides in or out, small background work waits (the loading
// shimmer, ...): each redraw there would make the phone draw the moving
// sheet again, and the slide stutters.

let quietUntil = 0;

/** Keeps background redraws still for `ms` (a sheet moves meanwhile). */
export function holdStill(ms: number) {
  quietUntil = Math.max(quietUntil, performance.now() + ms);
}

/** True while a sheet moves: skip redraws that can wait. */
export function holdingStill(): boolean {
  return performance.now() < quietUntil;
}
