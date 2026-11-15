<script lang="ts">
  // controls.rs surface: a flat surface for grouped content (radius 10, white 3.5 %, p 18).
  import type { Snippet } from "svelte";
  let { children, pad = 18, gap = 0 }: { children: Snippet; pad?: number; gap?: number } = $props();
</script>

<div class="surface" style:padding="{pad}px" style:gap="{gap}px">{@render children()}</div>

<style>
  .surface { border-radius: 10px; background: rgba(255, 255, 255, .035); display: flex; flex-direction: column; }
</style>
