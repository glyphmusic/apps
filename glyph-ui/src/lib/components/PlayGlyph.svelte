<script lang="ts">
  // widgets/indicators.rs play_glyph: pause / play in black, or a ~100° arc
  // turning once every 0.8 s while a song loads.
  import Icon from "./Icon.svelte";
  let { loading = false, playing = false, size = 15, button = 34, color = "#000" }: { loading?: boolean; playing?: boolean; size?: number; button?: number; color?: string } = $props();
  const ring = $derived(button * 0.56);
  const stroke = $derived(Math.max(2, button / 16));
</script>

{#if loading}
  <svg class="spin" width={ring} height={ring} viewBox="0 0 {ring} {ring}">
    <circle cx={ring / 2} cy={ring / 2} r={ring / 2 - stroke / 2} fill="none" stroke={color} stroke-width={stroke} stroke-linecap="round"
      stroke-dasharray="{Math.PI * (ring - stroke) * 0.28} 999" />
  </svg>
{:else}
  <Icon name={playing ? "pause" : "play"} {size} {color} />
{/if}

<style>
  .spin { animation: spin .8s linear infinite; }
</style>
