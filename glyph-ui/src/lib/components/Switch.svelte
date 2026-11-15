<script lang="ts">
  // widgets/buttons.rs toggle / toggle_unavailable: 30 x 18 switch (white
  // when on with a black knob), label sm; unavailable = dimmed with a tip.
  import type { Snippet } from "svelte";
  import { tip } from "../ui.svelte";
  let { on, onchange, label, children, disabled = false, why }: { on: boolean; onchange?: (v: boolean) => void; label?: string; children?: Snippet; disabled?: boolean; why?: string } = $props();
</script>

<button class="sw" class:on class:off={disabled} role="switch" aria-checked={on} onclick={() => !disabled && onchange?.(!on)} use:tip={disabled ? why : undefined}>
  <span class="track"><span class="knob"></span></span>
  {#if label}<span>{label}</span>{/if}
  {#if children}{@render children()}{/if}
</button>

<style>
  .sw { display: flex; align-items: center; gap: 10px; font-size: var(--sm); }
  .sw.off { opacity: .4; color: var(--faint); cursor: default; }
  .track { flex: none; width: 30px; height: 18px; border-radius: 9px; padding: 2px; display: flex; background: var(--active); transition: background-color .15s ease-out; }
  .knob { width: 14px; height: 14px; border-radius: 50%; background: var(--dim); transition: transform .18s var(--ease), background-color .15s; }
  .on .track { background: var(--text); }
  .on .knob { background: #000; transform: translateX(12px); }
</style>
