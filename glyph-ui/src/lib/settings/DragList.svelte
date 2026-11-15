<script lang="ts" generics="T">
  // DESIGN.md "Pattern: drag to reorder": the real row is picked up and
  // follows the cursor (flat lighter surface, no shadow), its place stays
  // empty, the rows between the old and the new place glide one row aside
  // (150 ms ease-out) to open a gap, dropping moves it, letting go on the
  // same slot puts it back. Rows have a fixed height `pitch`.
  import type { Snippet } from "svelte";
  let { items, pitch, onmove, row, key }: { items: T[]; pitch: number; onmove: (from: number, to: number) => void; row: Snippet<[T, number]>; key: (x: T) => string } = $props();
  let from = $state<number | null>(null);
  let slot = $state(0);
  let dy = $state(0);
  let startY = 0;

  function down(e: PointerEvent, i: number) {
    if (e.button !== 0 || (e.target as HTMLElement).closest("[data-nodrag]")) return;
    from = i; slot = i; dy = 0; startY = e.clientY;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function move(e: PointerEvent) {
    if (from == null) return;
    dy = e.clientY - startY;
    slot = Math.min(items.length - 1, Math.max(0, Math.round(from + dy / pitch)));
  }
  function up() {
    if (from == null) return;
    const f = from, s = slot;
    from = null; dy = 0;
    if (f !== s) onmove(f, s);
  }
  function shift(i: number): number {
    if (from == null) return 0;
    if (i === from) return dy;
    if (from < i && i <= slot) return -pitch;
    if (slot <= i && i < from) return pitch;
    return 0;
  }
</script>

<div class="dl">
  {#each items as it, i (key(it))}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="r" class:lift={from === i} style:height="{pitch}px" style:transform="translateY({shift(i)}px)"
      onpointerdown={(e) => down(e, i)} onpointermove={move} onpointerup={up} onpointercancel={() => { from = null; dy = 0; }}>
      {@render row(it, i)}
    </div>
  {/each}
</div>

<style>
  .dl { display: flex; flex-direction: column; }
  .r { position: relative; flex: none; border-radius: 8px; cursor: grab; touch-action: none; transition: transform .15s var(--ease), background-color .12s; }
  .r:hover { background: rgba(255, 255, 255, .03); }
  .r.lift { z-index: 3; transition: none; cursor: grabbing; background: rgba(43, 43, 43, .98); outline: 1px solid rgba(255, 255, 255, .12); }
</style>
