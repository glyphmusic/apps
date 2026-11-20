<script lang="ts">
  // compact.rs song_row: h 56, gap 12, cover 48 (radius 4), title 15 (accent
  // while it plays), artist 12.5 dim with a downloaded mark; a pink heart
  // when liked; ⋮ 44 px. Swipe right adds it to the queue.
  import Icon from "../components/Icon.svelte";
  import Cover from "../components/Cover.svelte";
  import SongTitle from "../components/SongTitle.svelte";
  import { app } from "../state.svelte";
  import { saveMenu } from "../pages/lists/actions";
  import { holdMenu } from "../ui.svelte";
  import { rating, type ListKey, type Track } from "../api/types";
  import { t } from "../i18n.svelte";
  import { drag, SWIPE_ACT } from "./cstate.svelte";
  let { tracks, index, from = null, context, number }: { tracks: Track[]; index: number; from?: ListKey | null; context?: string; number?: number } = $props();
  const tr = $derived(tracks[index]);
  const now = $derived(app.isCurrent(tr));
  // Select mode of this list: a tap picks the row (check over the cover).
  const picking = $derived(!!from && app.isPicking(from));
  const picked = $derived(picking && app.isPicked(tr));
  function tap() {
    if (picking) { app.togglePicked(tr); return; }
    if (app.allowed(tr)) app.play(tracks, index, context);
  }
  let sx = $state(0);
  function menu(e: MouseEvent) { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: tr, from: from ? { list: from, index } : null }); }
</script>

<div class="wrap">
  <span class="under" style:opacity={Math.min(1, sx / SWIPE_ACT)}><Icon name="list-plus" size={20} color="var(--accent)" /></span>
  <div class="row" class:off={!app.allowed(tr) && !picking} class:picked style:transform={sx ? `translateX(${sx}px)` : undefined} class:settle={sx === 0}
    onclick={tap} oncontextmenu={(e) => (picking ? e.preventDefault() : menu(e))} onkeydown={() => {}} role="row" tabindex="-1" aria-selected={picking ? picked : undefined}
    use:drag={{ axis: "x", move: (dx) => (sx = picking ? 0 : Math.max(0, Math.min(120, dx))), end: (dx) => { if (!picking && dx >= SWIPE_ACT) app.queueAdd([tr], false); sx = 0; } }}>
    {#if number !== undefined}<span class="num mono">{number}</span>{/if}
    <span class="cvw">
      <Cover track={tr} size={48} radius={4} />
      {#if picking}<span class="ck" class:on={picked}><Icon name={picked ? "check" : "circle"} size={picked ? 20 : 22} color={picked ? "#000" : "rgba(255,255,255,.9)"} /></span>{/if}
    </span>
    <span class="tx">
      <span class="t" class:on={now}><SongTitle rating={rating(tr)}>{tr.title}</SongTitle></span>
      <span class="a">
        {#if app.isDownloaded(tr)}<Icon name="arrow-down-to-line" size={12} color="var(--accent)" />{/if}
        <span class="trunc">{tr.artist}</span>
      </span>
    </span>
    {#if app.isLiked(tr) && !picking}
      <button class="b" aria-label={t("tip.heart")} onpointerdown={(e) => e.stopPropagation()} onclick={(e) => { e.stopPropagation(); app.toggleLike(tr); }} oncontextmenu={saveMenu(tr)} use:holdMenu={saveMenu(tr)}><Icon name="heart-fill" size={18} color="var(--accent)" /></button>
    {/if}
    {#if !picking}<button class="b more" aria-label={t("tip.ellipsis-vertical")} onpointerdown={(e) => e.stopPropagation()} onclick={menu}><Icon name="ellipsis-vertical" size={18} color="var(--dim)" /></button>{/if}
  </div>
</div>

<style>
  .wrap { position: relative; }
  .under { position: absolute; left: 16px; top: 0; bottom: 0; display: flex; align-items: center; }
  .row { position: relative; height: 56px; display: flex; align-items: center; gap: 12px; touch-action: pan-y; background: transparent; border-radius: 8px; transition: background-color .12s var(--ease), transform .16s var(--ease); }
  .row.settle { transition: transform .24s var(--ease); }
  .row:active:not(:has(.b:active)) { background: rgba(255, 255, 255, .05); transform: scale(.982); }
  .row.off { opacity: .4; }
  .row.picked { background: color-mix(in srgb, var(--accent) 14%, transparent); }
  .cvw { position: relative; flex: none; display: block; }
  .ck { position: absolute; inset: 0; display: grid; place-items: center; border-radius: 4px; background: rgba(0, 0, 0, .45); }
  .ck.on { background: color-mix(in srgb, var(--accent) 88%, transparent); }
  .num { flex: none; width: 20px; text-align: center; font-size: 13px; color: var(--dim); }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .t { font-size: 15px; color: var(--text); min-width: 0; }
  .t.on { color: var(--accent); }
  .a { display: flex; align-items: center; gap: 5px; min-width: 0; font-size: 12.5px; color: var(--dim); }
  .b { flex: none; width: 40px; height: 40px; display: grid; place-items: center; border-radius: 50%; }
  .b:active { background: rgba(255, 255, 255, .08); }
</style>
