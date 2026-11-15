<script lang="ts">
  // views.rs queue_row_inner: h 44, px 6, gap 8. Handle 18 x 32: the number
  // (mono xs faint, "01") fades out and a grip (14, dim) fades in on hover;
  // cover 32 radius 4; title sm (E badge) over artist xs dim. Hover fill =
  // card colour, radius 8. `lifted` = the drag image: grip in text colour,
  // flat lighter surface (hsl 0 0 16 %), 1 px white 10 % border, no hover.
  import Cover from "../../components/Cover.svelte";
  import Icon from "../../components/Icon.svelte";
  import IconButton from "../../components/IconButton.svelte";
  import SongTitle from "../../components/SongTitle.svelte";
  import { rating, type Track } from "../../api/types";
  let { track, n, lifted = false, off = false, onclick, onremove, ondown }: {
    track: Track; n: number; lifted?: boolean; off?: boolean; onclick?: () => void; onremove?: () => void; ondown?: (e: PointerEvent) => void;
  } = $props();
</script>

<div class="qr" class:lifted class:off onclick={onclick} onpointerdown={ondown} onkeydown={(e) => e.key === "Enter" && onclick?.()} role="row" tabindex="-1">
  <span class="h">
    {#if lifted}
      <Icon name="grip-vertical" size={14} color="var(--text)" />
    {:else}
      <span class="num">{String(n).padStart(2, "0")}</span>
      <span class="grip"><Icon name="grip-vertical" size={14} color="var(--dim)" /></span>
    {/if}
  </span>
  <Cover {track} size={32} radius={4} />
  <span class="tx">
    <span class="t"><SongTitle rating={rating(track)}>{track.title}</SongTitle></span>
    <span class="a trunc">{track.artist}</span>
  </span>
  {#if onremove}
    <span class="rm"><IconButton name="x" size={14} onclick={(e) => { e.stopPropagation(); onremove?.(); }} /></span>
  {/if}
</div>

<style>
  .qr { position: relative; display: flex; align-items: center; gap: 8px; height: 44px; padding: 0 8px; border-radius: 7px; cursor: pointer; touch-action: none; transition: background-color .24s ease-out; }
  .qr:not(.lifted):hover { background: rgba(255, 255, 255, .055); transition-duration: .12s; }
  .qr.off { opacity: .4; }
  .qr.lifted { background: hsl(0 0% 14%); border: 1px solid rgba(255, 255, 255, .1); cursor: grabbing; font-family: var(--font); color: var(--text); border-radius: 8px; }
  .h { position: relative; flex: none; width: 18px; height: 32px; display: grid; place-items: center; }
  .num { position: absolute; font-size: var(--xs); font-variant-numeric: tabular-nums; color: var(--faint); transition: opacity .12s; }
  .grip { position: absolute; display: grid; place-items: center; opacity: 0; transition: opacity .12s; }
  .qr:hover .num { opacity: 0; }
  .qr:hover .grip { opacity: 1; }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .t { font-size: var(--sm); min-width: 0; }
  .a { font-size: var(--xs); color: var(--dim); }
  .rm { flex: none; opacity: 0; transition: opacity .12s; }
  .qr:hover .rm { opacity: 1; }
</style>
