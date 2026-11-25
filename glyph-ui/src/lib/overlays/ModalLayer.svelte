<script lang="ts">
  import { eatTap } from "../ui.svelte";
  // widgets.rs modal_layer + modal_backdrop: a full-window layer. The
  // titlebar band is dimmed too (clicks go through to it); the backdrop
  // starts below the titlebar, takes the clicks and keeps its panel
  // centred on the window (padding-bottom = titlebar height).
  import type { Snippet } from "svelte";
  let { dim = 0.55, blur = 0, vis = 1, onbackdrop, children, z = 70 }: { dim?: number; blur?: number; vis?: number; onbackdrop?: (e: PointerEvent) => void; children: Snippet; z?: number } = $props();
  const filter = $derived(blur > 0 ? `blur(${(blur * vis).toFixed(1)}px) saturate(${(1 + 0.1 * vis).toFixed(3)})` : undefined);
</script>

<div class="layer" style:z-index={z} style:backdrop-filter={filter} style:-webkit-backdrop-filter={filter}>
  <div class="band" style:background="rgba(0,0,0,{dim * vis})"></div>
  <div class="backdrop" style:background="rgba(0,0,0,{dim * vis})" onpointerdown={(e) => { if (e.target === e.currentTarget && onbackdrop) { eatTap(); onbackdrop(e); } }} role="presentation">
    {@render children()}
  </div>
</div>

<style>
  .layer { position: fixed; inset: 0; pointer-events: none; }
  .band { position: absolute; top: 0; left: 0; right: 0; height: var(--titlebar-h); pointer-events: none; }
  .backdrop { position: absolute; top: var(--titlebar-h); left: 0; right: 0; bottom: 0; padding-bottom: var(--titlebar-h); display: flex; align-items: center; justify-content: center; pointer-events: auto; }
</style>
