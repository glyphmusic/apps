<script lang="ts">
  // widgets/tooltip.rs: the one open tooltip, next to its button with a
  // small stepped arrow pointing at it.
  import { tipState } from "../ui.svelte";
  let el: HTMLDivElement | undefined = $state();
  let pos = $state({ x: 0, y: 0 });
  $effect(() => {
    const r = tipState.rect;
    if (!r || !el || !tipState.shown) return;
    void tipState.label;
    const w = el.offsetWidth, h = el.offsetHeight;
    if (tipState.side === "right") pos = { x: r.right + 4, y: r.top + r.height / 2 - h / 2 };
    else if (tipState.side === "left") pos = { x: r.left - w - 4, y: r.top + r.height / 2 - h / 2 };
    else {
      const x = Math.min(innerWidth - w - 6, Math.max(6, r.left + r.width / 2 - w / 2));
      pos = { x, y: tipState.side === "below" ? r.bottom + 4 : r.top - h - 4 };
    }
  });
  const steps = [2, 4, 6, 8, 10];
  const arrowX = $derived(tipState.rect ? tipState.rect.left + tipState.rect.width / 2 - pos.x : 0);
</script>

<div class="tip {tipState.side}" data-part="tooltip" class:show={tipState.shown} class:wrap={tipState.wrap} bind:this={el} style:left="{pos.x}px" style:top="{pos.y}px" role="tooltip">
  {#if tipState.side === "right"}
    <span class="arrow l">{#each steps as s}<i style:height="{s}px"></i>{/each}</span>
  {:else if tipState.side === "left"}
    <span class="arrow l">{#each [...steps].reverse() as s}<i style:height="{s}px"></i>{/each}</span>
  {:else}
    <span class="arrow" style:left="{arrowX - 5}px">{#each tipState.side === "below" ? steps : [...steps].reverse() as s}<i style:width="{s}px"></i>{/each}</span>
  {/if}
  <div class="box">{tipState.label}</div>
</div>

<style>
  .tip { position: fixed; z-index: 100; pointer-events: none; opacity: 0; transition: opacity .12s ease-out; display: flex; flex-direction: column; }
  .tip.show { opacity: 1; }
  .tip.above { flex-direction: column-reverse; }
  .tip.right { flex-direction: row; align-items: center; }
  .tip.left { flex-direction: row-reverse; align-items: center; }
  .box { padding: 5px 9px; border-radius: 6px; background: var(--tip-bg); white-space: nowrap; font: 500 12.5px var(--font); color: rgba(255, 255, 255, .92); }
  .wrap .box { width: 280px; padding: 8px 11px; white-space: normal; font-weight: 400; line-height: 18px; color: rgba(255, 255, 255, .86); }
  .arrow { position: relative; width: 10px; display: flex; flex-direction: column; align-items: center; }
  .arrow i { display: block; height: 1px; background: var(--tip-bg); }
  .arrow.l { width: auto; flex-direction: row; }
  .arrow.l i { width: 1px; }
</style>
