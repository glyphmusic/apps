<script lang="ts">
  // controls.rs input_box: w 300 (phones: as wide as the row allows), h 34,
  // px 10, radius 8, white 6 % fill, no border, 13 px mono.
  import Icon from "../components/Icon.svelte";
  let {
    value = $bindable(""), placeholder = "", width = 300, height = 34, masked = false, onenter, oninput, onblur, disabled = false, label, mono = true, icon,
  }: {
    value?: string; placeholder?: string; width?: number | string; height?: number; masked?: boolean; onenter?: (v: string) => void;
    oninput?: (v: string) => void; onblur?: (v: string) => void; disabled?: boolean; label?: string; mono?: boolean; icon?: string;
  } = $props();
  const w = $derived(typeof width === "number" ? `${width}px` : width);
</script>

<div class="box" style:width={w} style:height="{height}px" class:dis={disabled}>
  {#if icon}<span class="ic-wrap"><Icon name={icon} size={14} color="rgba(255,255,255,.35)" /></span>{/if}
  <input bind:value type={masked ? "password" : "text"} size={1} {placeholder} {disabled} aria-label={label ?? placeholder} class:mono autocomplete="off" spellcheck="false"
    oninput={() => oninput?.(value)} onblur={() => onblur?.(value)} onkeydown={(e) => e.key === "Enter" && onenter?.(value)} />
</div>

<style>
  .box { flex: none; max-width: 100%; padding: 0 10px; display: flex; align-items: center; gap: 8px; border-radius: 8px; background: rgba(255, 255, 255, .06); transition: background-color .15s ease-out; }
  .ic-wrap { flex: none; display: inline-flex; }
  .box:focus-within { background: rgba(255, 255, 255, .1); }
  .box.dis { opacity: .5; }
  input { width: 100%; min-width: 0; border: 0; outline: 0; background: none; font-size: 13px; color: #fff; }
  input.mono { font-family: var(--mono); font-variant-ligatures: none; }
  input::placeholder { color: rgba(255, 255, 255, .3); }
</style>
