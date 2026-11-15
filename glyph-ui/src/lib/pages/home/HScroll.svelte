<script lang="ts">
  // views.rs cover_row_with / new_releases_row / recent_row frame: a row of
  // cards (gap 4, ml -8) that runs to the panel edge instead of stopping at
  // the page padding (mr -32, pr 32), scrolls sideways, wheel = sideways.
  import type { Snippet } from "svelte";
  import { fadeEdges } from "../../ui.svelte";
  let { children }: { children: Snippet } = $props();
  let el: HTMLDivElement | undefined = $state();
  function wheel(e: WheelEvent) {
    if (!el || el.scrollWidth <= el.clientWidth || Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
    e.preventDefault();
    el.scrollBy({ left: e.deltaY, behavior: e.deltaMode === 0 && Math.abs(e.deltaY) < 50 ? "auto" : "smooth" });
  }
</script>

<div class="hs" bind:this={el} onwheel={wheel} use:fadeEdges={{ axis: "x", size: 28, minOverflow: 16 }}>{@render children()}</div>

<style>
  .hs { display: flex; gap: 4px; margin: 0 calc(var(--page-x) * -1) 0 -8px; padding-right: var(--page-x); overflow-x: auto; overflow-y: hidden; scrollbar-width: none; overscroll-behavior-x: contain; }
  .hs::-webkit-scrollbar { display: none; }
</style>
