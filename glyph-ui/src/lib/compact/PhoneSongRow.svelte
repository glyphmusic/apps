<script lang="ts">
  // compact.rs song_row: h 56, gap 12, cover 48 (radius 4), title 15 (accent
  // while it plays), artist 12.5 dim with a downloaded mark; a pink heart
  // when liked; ⋮ 44 px. Swipe right adds it to the queue.
  import Icon from "../components/Icon.svelte";
  import Cover from "../components/Cover.svelte";
  import SongTitle from "../components/SongTitle.svelte";
  import { app } from "../state.svelte";
  import { rating, type ListKey, type Track } from "../api/types";
  import { t } from "../i18n.svelte";
  import { drag, SWIPE_ACT } from "./cstate.svelte";
  let { tracks, index, from = null, context, number }: { tracks: Track[]; index: number; from?: ListKey | null; context?: string; number?: number } = $props();
  const tr = $derived(tracks[index]);
  const now = $derived(app.isCurrent(tr));
  let sx = $state(0);
  function menu(e: MouseEvent) { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: tr, from: from ? { list: from, index } : null }); }
</script>

<div class="wrap">
  <span class="under" style:opacity={Math.min(1, sx / SWIPE_ACT)}><Icon name="list-plus" size={20} color="var(--accent)" /></span>
  <div class="row" class:off={!app.allowed(tr)} style:transform={sx ? `translateX(${sx}px)` : undefined} class:settle={sx === 0}
    onclick={() => app.allowed(tr) && app.play(tracks, index, context)} oncontextmenu={menu} onkeydown={() => {}} role="row" tabindex="-1"
    use:drag={{ axis: "x", move: (dx) => (sx = Math.max(0, Math.min(120, dx))), end: (dx) => { if (dx >= SWIPE_ACT) app.queueAdd([tr], false); sx = 0; } }}>
    {#if number !== undefined}<span class="num mono">{number}</span>{/if}
    <Cover track={tr} size={48} radius={4} />
    <span class="tx">
      <span class="t" class:on={now}><SongTitle rating={rating(tr)}>{tr.title}</SongTitle></span>
      <span class="a">
        {#if app.isDownloaded(tr)}<Icon name="arrow-down-to-line" size={12} color="var(--accent)" />{/if}
        <span class="trunc">{tr.artist}</span>
      </span>
    </span>
    {#if app.isLiked(tr)}
      <button class="b" aria-label={t("tip.heart")} onpointerdown={(e) => e.stopPropagation()} onclick={(e) => { e.stopPropagation(); app.toggleLike(tr); }}><Icon name="heart-fill" size={18} color="var(--accent)" /></button>
    {/if}
    <button class="b more" aria-label={t("tip.ellipsis-vertical")} onpointerdown={(e) => e.stopPropagation()} onclick={menu}><Icon name="ellipsis-vertical" size={18} color="var(--dim)" /></button>
  </div>
</div>

<style>
  .wrap { position: relative; }
  .under { position: absolute; left: 16px; top: 0; bottom: 0; display: flex; align-items: center; }
  .row { position: relative; height: 56px; display: flex; align-items: center; gap: 12px; touch-action: pan-y; background: transparent; border-radius: 8px; transition: background-color .12s var(--ease), transform .16s var(--ease); }
  .row.settle { transition: transform .24s var(--ease); }
  .row:active:not(:has(.b:active)) { background: rgba(255, 255, 255, .05); transform: scale(.982); }
  .row.off { opacity: .4; }
  .num { flex: none; width: 20px; text-align: center; font-size: 13px; color: var(--dim); }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .t { font-size: 15px; color: var(--text); min-width: 0; }
  .t.on { color: var(--accent); }
  .a { display: flex; align-items: center; gap: 5px; min-width: 0; font-size: 12.5px; color: var(--dim); }
  .b { flex: none; width: 40px; height: 40px; display: grid; place-items: center; border-radius: 50%; }
  .b:active { background: rgba(255, 255, 255, .08); }
</style>
