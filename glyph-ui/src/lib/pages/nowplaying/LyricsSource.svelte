<script lang="ts">
  import Icon from "../../components/Icon.svelte";
  import LyricsOptions from "./LyricsOptions.svelte";
  import { app } from "../../state.svelte";
  import { offset } from "./lyrics-state.svelte";

  const l = $derived(app.lyrics.lyrics);
  const off = $derived(offset());
  let open = $state(false);

  $effect(() => { void app.lyrics.key; open = false; });

  function onKey(e: KeyboardEvent) {
    if (open && e.key === "Escape") { e.stopPropagation(); open = false; }
  }
  function stop(e: Event) { e.stopPropagation(); }
</script>

<svelte:window onkeydown={onKey} />

{#if l && !app.lyrics.loading}
  {#if open}
    <div
      class="backdrop"
      onpointerdown={(e) => { e.stopPropagation(); e.preventDefault(); }}
      onmousedown={(e) => { e.stopPropagation(); e.preventDefault(); }}
      onclick={(e) => { e.stopPropagation(); e.preventDefault(); open = false; }}
      oncontextmenu={(e) => { e.stopPropagation(); e.preventDefault(); open = false; }}
      role="presentation"
    ></div>
  {/if}
  <div class="optw" class:open>
    <button
      class="ob"
      class:open
      type="button"
      onpointerdown={stop}
      onmousedown={stop}
      onclick={(e) => { e.stopPropagation(); open = !open; }}
    >
      <Icon name="sliders-horizontal" size={14} />
      <span>{l.source ?? ""}</span>
      {#if off !== 0}<span class="of mono">{off >= 0 ? "+" : "-"}{(Math.abs(off) / 1000).toFixed(1)}s</span>{/if}
      <Icon name="chevron-down" size={14} />
    </button>
    {#if open}<div class="pop"><LyricsOptions {l} /></div>{/if}
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 29; }
  .optw { position: relative; z-index: 1; }
  .optw.open { z-index: 30; }
  .ob {
    display: flex; align-items: center; gap: 7px; height: 34px; padding: 0 10px 0 12px;
    border-radius: 8px; background: #1e1e20;
    font-size: 13px; font-weight: 500; color: rgba(255, 255, 255, .88);
    transition: background-color .14s ease-out, color .14s ease-out;
  }
  .ob:hover, .ob.open { background: #2a2a2e; color: #fff; }
  .ob :global(.ic) { color: rgba(255, 255, 255, .65); transition: color .14s ease-out, transform .14s var(--ease); }
  .ob:hover :global(.ic), .ob.open :global(.ic) { color: #fff; }
  .ob :global(.ic:last-child) { color: rgba(255, 255, 255, .5); }
  .ob.open :global(.ic:last-child) { transform: rotate(180deg); color: rgba(255, 255, 255, .8); }
  .of { font-size: 11px; color: rgba(255, 255, 255, .6); }
  .pop { position: absolute; z-index: 31; top: 40px; right: 0; }
</style>
