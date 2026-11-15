<script lang="ts">
  // Lightweight GPU-composited placeholder shimmer (replaces per-instance
  // DPR `<canvas>` + ResizeObserver + 20 fps text drawing).
  import { fx } from "../ui.svelte";
  let { w, h, radius = 6, id: _id = "", round = false }: { w: number | string; h: number | string; radius?: number; id?: string; round?: boolean } = $props();
  const r = $derived(round ? 9999 : radius);
</script>

<div
  class="sk"
  class:plain={!fx.ascii}
  class:anim={fx.ascii && !fx.reduce}
  style:width={typeof w === "number" ? `${w}px` : w}
  style:height={typeof h === "number" ? `${h}px` : h}
  style:border-radius="{r}px"
></div>

<style>
  .sk { position: relative; flex: none; overflow: hidden; background: rgba(255, 255, 255, .04); }
  .sk.plain { background: rgba(255, 255, 255, .06); }
  .sk:not(.plain)::before {
    content: "";
    position: absolute;
    inset: 0;
    background-image: radial-gradient(rgba(255, 255, 255, .14) 1px, transparent 1px);
    background-size: 6px 8px;
    opacity: .55;
  }
  .sk.anim::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(90deg, transparent 0%, rgba(255, 255, 255, .14) 50%, transparent 100%);
    transform: translateX(-100%);
    animation: sk-sweep 1.6s linear infinite;
  }
  @keyframes sk-sweep { to { transform: translateX(100%); } }
</style>
