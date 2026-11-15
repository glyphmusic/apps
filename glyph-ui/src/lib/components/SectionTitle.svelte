<script lang="ts">
  // widgets/buttons.rs heading (24 bold) and section_title (18 px icon, lg
  // semibold, gap 10). Both sit on the background: adaptive ink.
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import { ink } from "../ui.svelte";
  let { icon, title, heading = false, children }: { icon?: string; title: string; heading?: boolean; children?: Snippet } = $props();
</script>

{#if heading}
  <h1 class="hd" use:ink>{title}</h1>
{:else}
  <div class="row">
    <h2 class="sec" use:ink>
      {#if icon}<Icon name={icon} size={18} />{/if}
      <span>{title}</span>
    </h2>
    {#if children}<div class="extra">{@render children()}</div>{/if}
  </div>
{/if}

<style>
  .hd { margin: 0; align-self: flex-start; font-size: calc(24px * var(--k)); line-height: 1.25; font-weight: 700; color: var(--text); }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .sec { margin: 0; display: flex; align-items: center; gap: 10px; font-size: var(--lg); font-weight: 600; color: var(--text); }
  .extra { display: flex; align-items: center; gap: 6px; }
</style>
