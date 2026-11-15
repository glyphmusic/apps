<script lang="ts">
  // controls.rs rows: rows separated by hairlines (white 6 %).
  import type { Snippet } from "svelte";
  let { children }: { children: Snippet } = $props();
</script>

<div class="rows">{@render children()}</div>

<style>
  .rows { display: flex; flex-direction: column; }
  .rows > :global(.srow:not(:last-child)) { border-bottom: 1px solid rgba(255, 255, 255, .06); }
</style>
