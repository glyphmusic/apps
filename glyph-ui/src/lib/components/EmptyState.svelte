<script lang="ts">
  // widgets/bars.rs ascii_state: a small icon in a soft circle (44 px, text
  // 8 %), title md semibold and one hint line, left-aligned on the background.
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import { ink } from "../ui.svelte";
  let { icon = "music-2", title, sub = "", children }: { icon?: string; title: string; sub?: string; children?: Snippet } = $props();
</script>

<div class="es" use:ink>
  <div class="c"><Icon name={icon} size={18} color="var(--dim)" /></div>
  <div class="tx">
    <div class="t">{title}</div>
    {#if sub}<div class="s">{sub}</div>{/if}
    {#if children}<div class="a">{@render children()}</div>{/if}
  </div>
</div>

<style>
  .es { align-self: flex-start; max-width: 100%; display: flex; align-items: center; gap: 14px; padding: 8px 0; }
  .c { flex: none; width: 44px; height: 44px; border-radius: 50%; display: grid; place-items: center; background: rgba(var(--ink), .08); }
  .tx { min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .t { font-size: var(--md); font-weight: 600; color: var(--text); }
  .s { font-size: var(--sm); color: var(--dim); }
  .a { margin-top: 8px; display: flex; align-items: center; gap: 8px; }
</style>
