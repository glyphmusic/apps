<script lang="ts">
  // settings.rs switch: 36 x 20, p 2, white when on with a black 16 px knob,
  // white 14 % with a white 75 % knob when off. `disabled` = unavailable (40 %, with a tip).
  import { tip } from "../ui.svelte";
  let { on, onchange, disabled = false, why, label }: { on: boolean; onchange?: (v: boolean) => void; disabled?: boolean; why?: string; label?: string } = $props();
</script>

<button class="tog" class:on class:off={disabled} role="switch" aria-checked={on} aria-label={label} onclick={() => !disabled && onchange?.(!on)} use:tip={disabled ? why : undefined}>
  <span class="knob"></span>
</button>

<style>
  .tog { flex: none; width: 36px; height: 20px; padding: 2px; border-radius: 10px; display: flex; background: rgba(255, 255, 255, .14); cursor: pointer; transition: background-color .15s ease-out; }
  .knob { width: 16px; height: 16px; border-radius: 50%; background: rgba(255, 255, 255, .75); transition: transform .18s var(--ease), background-color .15s; }
  .on { background: #fff; }
  .on .knob { background: #000; transform: translateX(16px); }
  .off { opacity: .4; cursor: default; }
</style>
