<script lang="ts">
  // Loading placeholder (src/ui/widgets/skeleton.rs). ASCII effects on: a
  // field of mono characters with a band sweeping across that changes them
  // (shimmer.ts); performance mode and reduced motion keep the characters
  // still. ASCII effects off: a plain box with a soft light band sweeping
  // across (still in performance mode and with reduced motion).
  import { fx } from "../ui.svelte";
  import { seedOf, shimmer } from "../shimmer";
  import { cornerPx } from "../corners";
  let { w, h, radius = 6, id = "", round = false, ascii }: { w: number | string; h: number | string; radius?: number; id?: string; round?: boolean; ascii?: boolean } = $props();
  const isAscii = $derived(ascii !== undefined ? ascii : fx.ascii);
  const still = $derived(fx.still || fx.reduce);
  const seed = $derived(seedOf(id));
  const r = $derived(round ? 9999 : radius);
</script>

<div
  class="sk"
  class:plain={!isAscii}
  class:sweep={!isAscii && !still}
  style:width={typeof w === "number" ? `${w}px` : w}
  style:height={typeof h === "number" ? `${h}px` : h}
  style:border-radius={round ? `${r}px` : cornerPx(r)}
>
  {#if isAscii}<canvas use:shimmer={{ seed, still }} aria-hidden="true"></canvas>{/if}
</div>

<style>
  .sk { position: relative; flex: none; overflow: hidden; background: rgba(255, 255, 255, .04); }
  .sk.plain { background: rgba(255, 255, 255, .06); }
  .sk.sweep::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(100deg, transparent 20%, rgba(255, 255, 255, .07) 50%, transparent 80%);
    transform: translateX(-100%);
    animation: sk-sweep 1.6s ease-in-out infinite;
    pointer-events: none;
  }
  @keyframes sk-sweep { to { transform: translateX(100%); } }
  canvas { position: absolute; inset: 0; width: 100%; height: 100%; display: block; pointer-events: none; }
</style>
