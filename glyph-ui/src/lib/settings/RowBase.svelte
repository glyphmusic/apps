<script lang="ts">
  // settings.rs row_full: label (14 medium white) + an (i) that explains it
  // on hover, an optional live status line (12.5, white 55 %) left, control
  // right (gap 8). py 14, gap 24. Phones (narrow): the control wraps under
  // the label only when the two would collide. A row picked in the settings
  // search lights up (accent 14 %) for FLASH_S and scrolls into view.
  import type { Snippet } from "svelte";
  import Icon from "../components/Icon.svelte";
  import { tip } from "../ui.svelte";
  import { sui } from "./settings.svelte";
  import { app } from "../state.svelte";
  let { label, info, status, statusColor, leading, children }: { label: string; info?: string | null; status?: string | null; statusColor?: string; leading?: Snippet; children?: Snippet } = $props();
  let el: HTMLDivElement | undefined = $state();
  const lit = $derived(sui.flash != null && sui.flash.label === label);
  // Phones have no hover: the explanation sits under the label (two lines,
  // a tap shows all of it) instead of behind an (i).
  let more = $state(false);
  $effect(() => { if (lit && el) el.scrollIntoView({ block: "center", behavior: "smooth" }); });
</script>

<div class="row" class:lit bind:this={el}>
  <div class="lead-row">
    {#if leading}<div class="lead">{@render leading()}</div>{/if}
    <div class="l">
      <div class="lt">
        <span class="lab">{label}</span>
        {#if info && !app.compact}<span class="info" use:tip={{ label: info, wrap: true }}><Icon name="info" size={13} color="rgba(255,255,255,.42)" /></span>{/if}
      </div>
      {#if info && app.compact}<button class="desc" class:more onclick={() => (more = !more)} aria-expanded={more}>{info}</button>{/if}
      {#if status}<div class="st" style:color={statusColor}>{status}</div>{/if}
    </div>
  </div>
  {#if children}<div class="c">{@render children()}</div>{/if}
</div>

<style>
  .row { position: relative; display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 10px 24px; padding: 14px 0; border-radius: 8px; }
  .row.lit { margin: 0 -10px; padding: 14px 10px; animation: flash 1.8s linear forwards; }
  @keyframes flash { 0%, 39% { background: rgba(var(--accent-rgb, 255, 95, 210), .14); } 100% { background: rgba(var(--accent-rgb, 255, 95, 210), 0); } }
  .lead-row { display: flex; align-items: center; gap: 12px; min-width: 0; flex: 1 1 200px; }
  .lead { flex: none; display: flex; align-items: center; }
  .l { display: flex; flex-direction: column; gap: 3px; min-width: 0; flex: 1; }
  .lt { display: flex; align-items: center; gap: 6px; }
  .lab { min-width: 0; font-size: 14px; font-weight: 500; color: #fff; }
  .info { flex: none; width: 18px; height: 18px; border-radius: 9px; display: grid; place-items: center; transition: background-color .12s; }
  .info:hover { background: rgba(255, 255, 255, .08); }
  .desc { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; overflow: hidden; margin-top: 2px; padding: 0; text-align: left; font: 400 13px/18px var(--font); color: rgba(255, 255, 255, .55); }
  .desc.more { -webkit-line-clamp: unset; line-clamp: unset; }
  /* Bigger tap target on touch screens (a tap shows the explanation). */
  .info { position: relative; cursor: help; }
  .info::after { content: ""; position: absolute; inset: -10px; }
  .st { font-size: 12.5px; color: rgba(255, 255, 255, .55); overflow-wrap: anywhere; }
  .c { display: flex; flex-wrap: wrap; justify-content: flex-end; align-items: center; gap: 8px; flex: none; max-width: 100%; margin-left: auto; }
</style>
