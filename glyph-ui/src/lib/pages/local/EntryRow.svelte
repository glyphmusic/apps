<script lang="ts">
  // local.rs entry_row: folder / artist / album row, h 56, px 10, gap 14,
  // radius 8, hover white 5 %: picture or icon, name (md) and detail line
  // (sm dim), chevron-right 16 faint. icon_art: 40 x 40, radius 6, text 8 %,
  // icon 18 dim.
  import type { Snippet } from "svelte";
  import Icon from "../../components/Icon.svelte";
  let { name, sub, icon, art, onclick, extra, delay = 0 }: { name: string; sub: string; icon?: string; art?: Snippet; onclick: () => void; extra?: Snippet; delay?: number } = $props();
</script>

<div class="er" role="button" tabindex="0" {onclick} onkeydown={(e) => e.key === "Enter" && onclick()} style:animation-delay="{delay}ms">
  {#if art}{@render art()}{:else if icon}<span class="ia"><Icon name={icon} size={18} color="var(--dim)" /></span>{/if}
  <span class="tx">
    <span class="n trunc">{name}</span>
    <span class="s trunc">{sub}</span>
  </span>
  {#if extra}{@render extra()}{/if}
  <Icon name="chevron-right" size={16} color="var(--faint)" />
</div>

<style>
  .er { display: flex; align-items: center; gap: 14px; height: 56px; padding: 0 10px; border-radius: 8px; cursor: pointer; transition: background-color .12s ease-out; animation: fade-in .25s ease-out both; }
  .er:hover { background: rgba(255, 255, 255, .05); }
  .ia { flex: none; width: 40px; height: 40px; border-radius: 6px; background: rgba(var(--ink), .08); display: grid; place-items: center; }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .n { font-size: var(--md); color: var(--text); }
  .s { font-size: var(--sm); color: var(--dim); }
</style>
