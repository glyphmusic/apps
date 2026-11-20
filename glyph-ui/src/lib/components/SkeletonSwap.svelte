<script lang="ts">
  // Smooth grid-stacked crossfade between loading skeletons and loaded results.
  // Keeping both layers in grid-area 1/1 prevents blank flashes and layout
  // collapses during transitions; reduced-motion / performance mode skips motion.
  import type { Snippet } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { fx } from "../ui.svelte";

  let {
    loading,
    key = "",
    skeleton,
    children,
  }: {
    loading: boolean;
    key?: string | number;
    skeleton: Snippet;
    children: Snippet;
  } = $props();

  const instant = $derived(fx.reduce || fx.still);
  const skIn = $derived(instant ? 0 : 140);
  const skOut = $derived(instant ? 0 : 180);
  const resIn = $derived(instant ? 0 : 240);
  const resDelay = $derived(instant ? 0 : 35);
  const resY = $derived(instant ? 0 : 4);
</script>

<div class="swap">
  {#if loading}
    <div class="layer sk" in:fade={{ duration: skIn, easing: cubicOut }} out:fade={{ duration: skOut, easing: cubicOut }}>
      {@render skeleton()}
    </div>
  {:else}
    {#key key}
      <div class="layer" in:fly={{ y: resY, duration: resIn, delay: resDelay, opacity: 0, easing: cubicOut }} out:fade={{ duration: skIn, easing: cubicOut }}>
        {@render children()}
      </div>
    {/key}
  {/if}
</div>

<style>
  .swap { display: grid; min-width: 0; }
  .layer { grid-area: 1 / 1; min-width: 0; }
  .layer.sk { pointer-events: none; }
</style>
