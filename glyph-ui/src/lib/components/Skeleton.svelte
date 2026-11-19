<script lang="ts">
  // Loading placeholder (src/ui/widgets/skeleton.rs). ASCII effects on: a
  // field of mono characters with a band sweeping across that changes them
  // (shimmer.ts); performance mode and reduced motion keep the characters
  // still. ASCII effects off: a plain, still box.
  import { fx } from "../ui.svelte";
  import { seedOf, shimmer } from "../shimmer";
  let { w, h, radius = 6, id = "", round = false, ascii }: { w: number | string; h: number | string; radius?: number; id?: string; round?: boolean; ascii?: boolean } = $props();
  const isAscii = $derived(ascii !== undefined ? ascii : fx.ascii);
  const still = $derived(fx.still || fx.reduce);
  const seed = $derived(seedOf(id));
  const r = $derived(round ? 9999 : radius);
</script>

<div
  class="sk"
  class:plain={!isAscii}
  style:width={typeof w === "number" ? `${w}px` : w}
  style:height={typeof h === "number" ? `${h}px` : h}
  style:border-radius="{r}px"
>
  {#if isAscii}<canvas use:shimmer={{ seed, still }} aria-hidden="true"></canvas>{/if}
</div>

<style>
  .sk { position: relative; flex: none; overflow: hidden; background: rgba(255, 255, 255, .04); }
  .sk.plain { background: rgba(255, 255, 255, .06); }
  canvas { position: absolute; inset: 0; width: 100%; height: 100%; display: block; pointer-events: none; }
</style>
