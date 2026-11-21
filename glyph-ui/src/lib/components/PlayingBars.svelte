<script lang="ts">
  // widgets/cover.rs playing_bars: four bars next to the playing song that
  // move with the music (a gentle motion without a spectrum); low and still
  // while paused.
  import { app } from "../state.svelte";
  let { size = 13, color = "var(--accent)", active }: { size?: number; color?: string; active?: boolean } = $props();
  const run = $derived(active ?? app.playing);
</script>

<span class="pb" class:run style:width="{size}px" style:height="{size}px" style:gap="{Math.max(1.5, size * 0.1)}px">
  {#each [0, 1, 2, 3] as i}
    <i style:width="{Math.max(2, size * 0.16)}px" style:background={color} style:animation-delay="{-i * 0.37}s" style:animation-duration="{0.9 + i * 0.23}s"></i>
  {/each}
</span>

<style>
  .pb { flex: none; display: flex; align-items: flex-end; justify-content: center; }
  i { height: 25%; border-radius: 1px 1px 0 0; }
  .run i { animation: eq ease-in-out infinite alternate; }
  @keyframes eq { 0% { height: 20%; } 50% { height: 85%; } 100% { height: 40%; } }
</style>
