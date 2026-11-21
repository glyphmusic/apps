<script lang="ts">
  // widgets/track_row.rs fade_edges: a scroll frame whose top and bottom
  // edge fade smoothly while there is more content beyond it.
  import type { Snippet } from "svelte";
  import { app } from "../../state.svelte";
  import { fadeEdges } from "../../ui.svelte";
  import OverlayScrollbar from "../OverlayScrollbar.svelte";
  let { el = $bindable(), children, pad = "0 10px", onscroll }: { el?: HTMLDivElement; children: Snippet; pad?: string; onscroll?: () => void } = $props();
</script>

<div class="fs">
  <div class="scroll in" bind:this={el} style:padding={pad} onscroll={onscroll} use:fadeEdges={{ top: 20, bottom: 20 }}>{@render children()}</div>
  {#if !app.compact}<OverlayScrollbar {el} top={8} bottom={8} />{/if}
</div>

<style>
  .fs { position: relative; flex: 1; min-height: 0; }
  .in { position: absolute; inset: 0; scrollbar-width: none; -ms-overflow-style: none; }
  .in::-webkit-scrollbar { display: none; width: 0; height: 0; }
</style>
