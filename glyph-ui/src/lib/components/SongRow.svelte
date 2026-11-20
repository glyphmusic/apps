<script lang="ts" module>
  import { backend as getBackend } from "../api/backend";
  import { trackKey as keyOfTrack, type Track as TrackItem } from "../api/types";

  // Songs without an album look it up once they show (one or two at a time).
  // The core remembers every answer on disk (album_names.rs), so the next
  // time the album is there at once; a found album is put on the row right
  // away instead of waiting for the library event.
  const attemptedAlbums = new Set<string>();
  const albumQueue: { track: TrackItem; signal: AbortSignal }[] = [];
  const ALBUM_WORKERS = 2;
  let albumBusy = 0;

  function scheduleAlbumBackfill(track: TrackItem, signal: AbortSignal) {
    if (track.album?.trim() || !track.title?.trim() || !track.artist?.trim()) return;
    const k = keyOfTrack(track);
    if (attemptedAlbums.has(k)) return;
    if (albumQueue.some((x) => keyOfTrack(x.track) === k)) return;
    albumQueue.push({ track, signal });
    while (albumBusy < ALBUM_WORKERS && albumBusy < albumQueue.length) void drainAlbumQueue();
  }

  async function drainAlbumQueue() {
    albumBusy++;
    try {
      while (albumQueue.length > 0) {
        const next = albumQueue.shift()!;
        if (next.signal.aborted) continue;
        const k = keyOfTrack(next.track);
        if (attemptedAlbums.has(k) || next.track.album?.trim()) continue;
        attemptedAlbums.add(k);
        if (attemptedAlbums.size > 4000) attemptedAlbums.clear();
        try {
          const album = await getBackend().call<string | null>("track_album", { track: next.track });
          if (album?.trim() && !next.track.album?.trim()) next.track.album = album.trim();
        } catch {
          /* transient/offline ignored */
        }
        if (albumQueue.length > 0) {
          await new Promise((r) => setTimeout(r, 60));
        }
      }
    } finally {
      albumBusy--;
    }
  }
</script>

<script lang="ts">
  // lists/rows.rs song_row: h 56, px 10, gap 14, radius 8, hover white 5 %.
  // Number (playing bars while it plays; a play icon on hover) → cover 40
  // → title / artist (download badge) → album 200 (only when the row is at
  // least 700 px wide) → heart (on hover unless liked) → duration 40 → ⋮.
  // Click plays the list from this row; right click / ⋮ opens the menu.
  import Icon from "./Icon.svelte";
  import Cover from "./Cover.svelte";
  import SongTitle from "./SongTitle.svelte";
  import ToggleButton from "./ToggleButton.svelte";
  import PlayingBars from "./PlayingBars.svelte";
  import DownloadBadge from "./DownloadBadge.svelte";
  import { app } from "../state.svelte";
  import { saveMenu } from "../pages/lists/actions";
  import { fmtMs } from "../fmt";
  import { rating, trackKey, type ListKey, type Track } from "../api/types";
  import { tip } from "../ui.svelte";
  import { t, tf } from "../i18n.svelte";
  import { backend } from "../api/backend";
  let {
    tracks, index, fromIndex, from = null, album = true, context, selected = false, draggable = false, ondragstart, number, picking = false, picked = false,
  }: {
    tracks: Track[]; index: number; fromIndex?: number; from?: ListKey | null; album?: boolean; context?: string; selected?: boolean;
    draggable?: boolean; ondragstart?: (e: PointerEvent) => void; number?: number;
    /** Select mode: a click picks the row instead of playing it. */
    picking?: boolean; picked?: boolean;
  } = $props();
  const tr = $derived(tracks[index]);
  const playing = $derived(app.isCurrent(tr));
  const liked = $derived(app.isLiked(tr));
  const blocked = $derived(!app.allowed(tr));
  const key = $derived(trackKey(tr));
  const dl = $derived(key in app.downloading ? "busy" : app.isDownloaded(tr) ? "done" : "none");
  // ui/lists/rows.rs download_tip_one: the phase and, while bytes flow, how far it is.
  const dlTip = $derived.by(() => {
    if (dl === "done") return t("tip.dl_song_done");
    const p = app.downloading[key];
    return p == null ? t("tip.dl_song_prepare") : p >= 0.995 ? t("tip.dl_song_finish") : tf("tip.dl_song_busy", { pct: Math.round(p * 100) });
  });
  // ui/ytdlp.rs prefetch_on_hover: after a short rest on a row the song is looked up, so a click plays at once.
  let hoverTimer: ReturnType<typeof setTimeout> | undefined;
  function hoverIn() {
    if (blocked || playing || app.platform === "android" || app.platform === "ios") return;
    clearTimeout(hoverTimer);
    hoverTimer = setTimeout(() => { void backend().call("prefetch_hover", { track: tr }); }, 250);
  }
  function hoverOut() { clearTimeout(hoverTimer); }
  $effect(() => {
    if (!album || tr.album?.trim()) return;
    const ac = new AbortController();
    const timer = setTimeout(() => scheduleAlbumBackfill(tr, ac.signal), 120);
    return () => { clearTimeout(timer); ac.abort(); };
  });
  function play() {
    if (picking) { app.togglePicked(tr); return; }
    app.selectedRow = index;
    if (!blocked) app.play(tracks, index, context);
  }
  function menu(e: MouseEvent) {
    e.preventDefault(); e.stopPropagation();
    app.openCtx({ x: e.clientX, y: e.clientY, track: tr, from: from ? { list: from, index: fromIndex ?? index } : null });
  }
</script>

<div class="row" class:off={blocked && !picking} class:sel={selected && !picking} class:picking class:picked class:menu={app.ctx?.track === tr} onclick={play} oncontextmenu={menu}
  onpointerenter={hoverIn} onpointerleave={hoverOut}
  onpointerdown={(e) => draggable && !picking && e.button === 0 && ondragstart?.(e)} onkeydown={(e) => e.key === "Enter" && play()} role="row" tabindex="-1" aria-selected={picking ? picked : undefined}>
  <span class="num">
    {#if picking}
      <span class="ck"><Icon name={picked ? "circle-check" : "circle"} size={18} color={picked ? "var(--accent)" : "var(--faint)"} /></span>
    {:else}
      <span class="n">{#if playing}<PlayingBars size={14} />{:else}<span class="mono">{number ?? index + 1}</span>{/if}</span>
      <span class="p"><Icon name="play" size={13} color="var(--text)" /></span>
    {/if}
  </span>
  <Cover track={tr} size={40} radius={6} />
  <span class="tx">
    <span class="tl" class:on={playing}><SongTitle rating={rating(tr)}>{tr.title}</SongTitle></span>
    <span class="ar">
      {#if dl !== "none"}<span class="dl" use:tip={dlTip}><DownloadBadge state={dl} progress={app.downloading[key] ?? null} size={14} /></span>{/if}
      <button class="link trunc" onclick={(e) => { e.stopPropagation(); app.openArtistPage(tr.artist); }}>{tr.artist}</button>
    </span>
  </span>
  {#if album}
    <span class="al trunc">{#if tr.album}<button class="link trunc" onclick={(e) => { e.stopPropagation(); app.openAlbum(tr.album!, tr.artist, [], tr); }}>{tr.album}</button>{/if}</span>
  {/if}
  <span class="like" class:keep={liked}><ToggleButton name="heart" size={15} on={liked} accent="var(--accent)" onclick={() => app.toggleLike(tr)} oncontextmenu={saveMenu(tr)} /></span>
  <span class="du mono">{fmtMs(tr.duration_ms)}</span>
  <button class="more" aria-label={t("tip.ellipsis-vertical")} onclick={menu}><Icon name="ellipsis-vertical" size={16} color="var(--dim)" /></button>
</div>

<style>
  .row { container-type: inline-size; display: flex; align-items: center; gap: 14px; height: 56px; padding: 0 10px; border-radius: 8px; cursor: pointer; transition: background-color .12s ease-out; min-width: 0; }
  .row:hover, .row.menu { background: rgba(255, 255, 255, .05); transition-duration: .12s; }
  .row:not(:hover) { transition-duration: .24s; }
  .row.sel { background: rgba(255, 255, 255, .1); }
  .row.off { opacity: .4; }
  .row.picked { background: color-mix(in srgb, var(--accent) 12%, transparent); }
  .ck { position: absolute; inset: 0; display: grid; place-items: center; }
  .num { position: relative; flex: none; width: 26px; height: 20px; }
  .n, .p { position: absolute; inset: 0; display: grid; place-items: center; transition: opacity .12s; }
  .n .mono { font-size: var(--sm); color: var(--faint); }
  .p { opacity: 0; }
  .row:hover .n { opacity: 0; }
  .row:hover .p { opacity: 1; }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .tl { font-size: var(--md); color: var(--text); min-width: 0; }
  .tl.on { color: var(--accent); font-weight: 600; }
  .ar { display: flex; align-items: center; gap: 6px; min-width: 0; font-size: var(--sm); color: var(--dim); }
  .dl { flex: none; width: 16px; height: 16px; display: grid; place-items: center; }
  .link { min-width: 0; max-width: 100%; }
  .link:hover { color: var(--text); text-decoration: underline; }
  .al { flex: 0 1 200px; width: 200px; min-width: 0; display: flex; font-size: var(--sm); color: var(--faint); }
  @container (max-width: 699px) { .al { display: none; } }
  .like { flex: none; opacity: 0; transition: opacity .12s; }
  .like.keep, .row:hover .like { opacity: 1; }
  .du { flex: none; width: 40px; font-size: var(--sm); color: var(--faint); }
  .more { flex: none; width: 28px; height: 28px; display: grid; place-items: center; border-radius: 6px; }
  .more:hover { background: rgba(255, 255, 255, .08); }
  @container (max-width: 360px) { .num { width: 20px; } .du { display: none; } }
</style>
