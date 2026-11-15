<script lang="ts">
  // widgets/buttons.rs chip (filter chips: white when on) and
  // indicators.rs kind_chip ("Song", "Artist" in mixed results).
  import type { Snippet } from "svelte";
  let { on = false, kind = "filter", onclick, children }: { on?: boolean; kind?: "filter" | "kind"; onclick?: (e: MouseEvent) => void; children: Snippet } = $props();
</script>

{#if kind === "kind"}
  <span class="kc">{@render children()}</span>
{:else}
  <button class="chip" class:on {onclick}>{@render children()}</button>
{/if}

<style>
  .chip { flex: none; height: 28px; padding: 0 12px; display: flex; align-items: center; justify-content: center; border-radius: 8px; font-size: 12px; line-height: 14px; background: #141414; color: rgba(255, 255, 255, .85); transition: background-color .12s, transform .18s cubic-bezier(.34, 1.56, .64, 1); }
  .chip:hover { background: #1f1f1f; }
  .chip:active { transform: scale(.93); }
  .chip.on { background: #fff; color: #000; }
  .kc { flex: none; height: 18px; padding: 0 6px; display: flex; align-items: center; border-radius: 4px; background: rgba(255, 255, 255, .08); font-size: 11px; color: rgba(255, 255, 255, .6); }
</style>
