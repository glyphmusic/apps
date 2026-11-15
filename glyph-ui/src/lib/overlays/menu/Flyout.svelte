<script lang="ts">
  // lists/ctx_menu.rs sub_row flyout: w 228, max-h 320 (scrolls), p 6,
  // radius 10, hsla(0,0,.075,.98), 1 px white 10 %. Hangs 5 px right of the
  // menu (or left when it would not fit), top -6; slides up to stay 8 px
  // inside the window.
  import type { Snippet } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { fadeEdges } from "../../ui.svelte";
  let { flip = false, top = -6, children, aside }: {
    flip?: boolean;
    top?: number;
    children: Snippet;
    aside?: Snippet;
  } = $props();
  let el: HTMLDivElement | undefined = $state();
  let dy = $state(0);
  $effect(() => {
    if (!el) return;
    const r = el.getBoundingClientRect();
    const over = r.bottom - dy - (innerHeight - 8);
    dy = over > 0 ? -over : 0;
  });
</script>

<div class="fly" class:flip bind:this={el} style:top="{top}px" style:transform="translateY({dy}px)"
  in:fly|global={{ x: flip ? 6 : -6, duration: 120, easing: cubicOut }} out:fade|global={{ duration: 90 }}
  onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")}
  onclick={(e) => e.stopPropagation()} onkeydown={() => {}} role="menu" tabindex="-1">
  <div class="fly-box">
    <div class="fly-in scroll" use:fadeEdges={{ top: 10, bottom: 10, minOverflow: 8 }}>
      {@render children()}
    </div>
  </div>
  {@render aside?.()}
</div>

<style>
  .fly { position: absolute; left: calc(100% + 12px); z-index: 1; width: 228px; cursor: default; }
  .fly-box { border-radius: 10px; background: #181818; overflow: hidden; }
  .fly-in { max-height: 320px; padding: 6px; display: flex; flex-direction: column; }
  .fly.flip { left: auto; right: calc(100% + 12px); }
</style>
