<script lang="ts">
  // compact.rs song_row: h 56, gap 12, cover 48 (radius 4), title 15 (accent
  // while it plays), artist 12.5 dim with a downloaded mark; a pink heart
  // when liked; ⋮ 44 px. Swipe right adds it to the queue.
  import Icon from "../components/Icon.svelte";
  import Cover from "../components/Cover.svelte";
  import SongTitle from "../components/SongTitle.svelte";
  import DownloadBadge from "../components/DownloadBadge.svelte";
  import { app } from "../state.svelte";
  import { saveMenu } from "../pages/lists/actions";
  import { holdMenu } from "../ui.svelte";
  import { rating, trackKey, type ListKey, type Track } from "../api/types";
  import { t } from "../i18n.svelte";
  import { drag, SWIPE_ACT } from "./cstate.svelte";
  let { tracks, index, from = null, context, number }: { tracks: Track[]; index: number; from?: ListKey | null; context?: string; number?: number } = $props();
  const tr = $derived(tracks[index]);
  const key = $derived(trackKey(tr));
  const dlBusy = $derived(key in app.downloading);
  const now = $derived(app.isCurrent(tr));
  // Select mode of this list: a tap picks the row (check over the cover).
  const picking = $derived(!!from && app.isPicking(from));
  const picked = $derived(picking && app.isPicked(tr));
  function tap() {
    if (picking) { app.togglePicked(tr); return; }
    if (app.allowed(tr)) app.play(tracks, index, context);
  }
  let sx = $state(0);
  const armed = $derived(sx >= SWIPE_ACT);
  function dampSwipe(dx: number): number {
    if (dx <= 0) return 0;
    if (dx <= SWIPE_ACT) return dx;
    return Math.min(124, SWIPE_ACT + (dx - SWIPE_ACT) * 0.35);
  }
  function menu(e: MouseEvent) { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: tr, from: from ? { list: from, index } : null }); }
</script>

<div class="wrap" class:swiping-wrap={sx > 0}>
  {#if sx > 0}
    <div class="under" class:armed style:opacity={Math.min(1, sx / 22)}>
      <span class="u-ic" class:pop={armed}><Icon name="list-plus" size={21} color={armed ? "#0d0d0d" : "var(--accent)"} /></span>
    </div>
  {/if}
  <div class="row" class:off={!app.allowed(tr) && !picking} class:picked class:swiping={sx > 0} style:transform={sx ? `translateX(${sx}px)` : undefined} class:settle={sx === 0}
    onclick={tap} oncontextmenu={(e) => (picking ? e.preventDefault() : menu(e))} onkeydown={() => {}} role="row" tabindex="-1" aria-selected={picking ? picked : undefined}
    use:drag={{ axis: "x", move: (dx) => (sx = picking ? 0 : dampSwipe(dx)), end: (dx, _dy, vx) => { if (!picking && (dx >= SWIPE_ACT || (vx > 0.5 && dx > 28))) app.queueAdd([tr], false); sx = 0; } }}>
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
    {#if dlBusy && !picking}
      <button type="button" class="b" aria-label={t("ctx.cancel_download")} onpointerdown={(e) => e.stopPropagation()} onclick={(e) => { e.stopPropagation(); app.cancelDownloads([tr]); }}>
        <DownloadBadge state="busy" progress={app.downloading[key] ?? null} size={18} arrow="x" />
      </button>
    {/if}
    {#if app.isLiked(tr) && !picking}
      <button class="b" aria-label={t("tip.heart")} onpointerdown={(e) => e.stopPropagation()} onclick={(e) => { e.stopPropagation(); app.toggleLike(tr); }} oncontextmenu={saveMenu(tr)} use:holdMenu={saveMenu(tr)}><Icon name="heart-fill" size={18} color="var(--accent)" /></button>
    {/if}
    {#if !picking}<button class="b more" aria-label={t("tip.ellipsis-vertical")} onpointerdown={(e) => e.stopPropagation()} onclick={menu}><Icon name="ellipsis-vertical" size={18} color="var(--dim)" /></button>{/if}
  </div>
</div>

<style>
  .wrap { position: relative; border-radius: 10px; }
  .wrap.swiping-wrap { overflow: hidden; }
  .under {
    position: absolute;
    inset: 0;
    border-radius: 10px;
    padding: 0 16px;
    display: flex;
    align-items: center;
    gap: 8px;
    background: color-mix(in srgb, var(--accent) 16%, #141416);
    color: var(--accent);
    pointer-events: none;
    transition: background-color .16s ease-out, color .16s ease-out;
  }
  .under.armed { background: var(--accent); color: #0d0d0d; }
  .u-ic { width: 38px; height: 38px; border-radius: 50%; display: grid; place-items: center; transition: transform .18s var(--ease); }
  .u-ic.pop { transform: scale(1.18); }
  .row { position: relative; height: 56px; display: flex; align-items: center; gap: 12px; touch-action: pan-y; background: transparent; border-radius: 10px; transition: background-color .12s var(--ease), transform .16s var(--ease); }
  .row.swiping { transition: background-color .12s ease-out; background: #141416; }
  .row.settle { transition: transform .22s var(--ease); }
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
