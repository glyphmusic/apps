<script lang="ts">
  // widgets/track_row.rs track_row: quiet track row, gap 14, px 10, py 6,
  // radius 8, hover card colour. Cover 44 radius 6 → [bars] [E] title md
  // (playing: pink + semibold) over "artist · album" sm dim → kind chip
  // "Song" (search only) → [DZ] source badges → play next / add to queue /
  // copy link (visible on hover) → heart (pink) → duration mono sm faint
  // (44 px). Search results decode in (ASCII, 0.9 s). The selected row
  // (clicked last) has white 8 % + a 1 px white 16 % ring. Hidden explicit
  // songs are dimmed and do not play.
  import Icon from "../../components/Icon.svelte";
  import Cover from "../../components/Cover.svelte";
  import SongTitle from "../../components/SongTitle.svelte";
  import Badges from "../../components/Badges.svelte";
  import Chip from "../../components/Chip.svelte";
  import PlayingBars from "../../components/PlayingBars.svelte";
  import ToggleButton from "../../components/ToggleButton.svelte";
  import IconButton from "../../components/IconButton.svelte";
  import Decode from "../../components/Decode.svelte";
  import { app } from "../../state.svelte";
  import { t } from "../../i18n.svelte";
  import { fmtMs } from "../../fmt";
  import { primaryArtist, rating, trackKey, type ListKey, type Track } from "../../api/types";
  let {
    tracks, index, kind = "section", from = null, context, lyric = null, chip,
  }: {
    tracks: Track[]; index: number; kind?: "search" | "section"; from?: ListKey | null; context?: string;
    lyric?: string | null; chip?: string;
  } = $props();
  const tr = $derived(tracks[index]);
  const key = $derived(trackKey(tr));
  const playing = $derived(app.isCurrent(tr));
  const liked = $derived(app.isLiked(tr));
  const blocked = $derived(!app.allowed(tr));
  const selId = $derived(lyric != null ? -(index + 1) : index);
  const selected = $derived(app.selectedRow === selId);
  const reveal = $derived(kind === "search" ? `search:${key}` : undefined);

  function play() {
    app.selectedRow = selId;
    if (blocked) return app.toast(t("toast.explicit_blocked"), "info");
    app.play(tracks, index, context);
  }
  function menu(e: MouseEvent) {
    e.preventDefault(); e.stopPropagation();
    app.openCtx({ x: e.clientX, y: e.clientY, track: tr, from: from ? { list: from, index } : null });
  }
  function artistMenu(e: MouseEvent) { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: null, artist: primaryArtist(tr.artist) }); }
  const stop = (f: () => void) => (e: MouseEvent) => { e.stopPropagation(); f(); };
</script>

<div class="row" class:lyrics={!!lyric} class:off={blocked} class:sel={selected} class:menu={app.ctx?.track === tr} onclick={play} oncontextmenu={menu} onkeydown={(e) => e.key === "Enter" && play()} role="row" tabindex="-1">
  <Cover track={tr} size={44} radius={6} />
  <div class="tx">
    <div class="tl" class:on={playing}>
      {#if playing}<PlayingBars size={13} />{/if}
      <SongTitle rating={rating(tr)}><Decode text={tr.title} key={reveal} /></SongTitle>
    </div>
    <div class="ar">
      {#if app.compact && (chip || kind === "search")}
        <span class="kind-txt">{chip ?? t("search.kind.song")}</span>
        <span class="dot">·</span>
      {/if}
      <button class="link trunc" onclick={stop(() => app.openArtistPage(tr.artist))} oncontextmenu={artistMenu}><Decode text={tr.artist} key={reveal} /></button>
      {#if tr.album && !app.compact}
        <span class="dot">·</span>
        <button class="link trunc al" onclick={stop(() => app.openAlbum(tr.album!, tr.artist, [], tr))}>{tr.album}</button>
      {/if}
    </div>
    {#if lyric}<div class="ly trunc">{"„"}{lyric}{"“"}</div>{/if}
  </div>
  {#if !app.compact}
    {#if chip || kind === "search"}<Chip kind="kind">{chip ?? t("search.kind.song")}</Chip>{/if}
    <Badges track={tr} />
    <div class="acts">
      <IconButton name="list-start" size={15} onclick={(e) => { e.stopPropagation(); app.queueAdd([tr], true); }} />
      <IconButton name="list-plus" size={15} onclick={(e) => { e.stopPropagation(); app.queueAdd([tr], false); }} />
      <IconButton name="link-2" size={15} onclick={(e) => { e.stopPropagation(); app.share({ track: tr }); }} />
    </div>
    <ToggleButton name="heart" size={15} on={liked} accent="var(--accent)" onclick={() => app.toggleLike(tr)} />
    <span class="du mono">{fmtMs(tr.duration_ms)}</span>
  {:else}
    <div class="m-acts">
      <ToggleButton name="heart" size={18} on={liked} accent="var(--accent)" notip onclick={() => app.toggleLike(tr)} />
      <button class="mb more" type="button" aria-label={t("tip.ellipsis-vertical")} onclick={menu}>
        <Icon name="ellipsis-vertical" size={18} color="var(--dim)" />
      </button>
    </div>
  {/if}
</div>

<style>
  .row { position: relative; display: flex; align-items: center; gap: 14px; padding: 6px 10px; border-radius: 8px; cursor: pointer; transition: background-color .24s ease-out; }
  .row.lyrics { padding: 8px 10px; }
  .row:hover, .row.menu { background: var(--card); transition-duration: .12s; }
  .row.sel { background: rgba(255, 255, 255, .1); }
  .row.off { opacity: .4; }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .tl { display: flex; align-items: center; gap: 6px; min-width: 0; font-size: var(--md); color: var(--text); }
  .tl.on { color: var(--accent); font-weight: 600; }
  .ar { display: flex; min-width: 0; font-size: var(--sm); color: var(--dim); }
  .ly { margin-top: 2px; font-size: var(--xs); font-style: italic; color: var(--faint); min-width: 0; }
  .dot { flex: none; white-space: pre; }
  .dot::before, .dot::after { content: " "; }
  .link { min-width: 0; max-width: 100%; }
  .al { flex: 0 1 auto; }
  .link:hover { color: var(--text); text-decoration: underline; }
  .acts { flex: none; display: flex; gap: 2px; opacity: 0; transition: opacity .12s ease-out; }
  .row:hover .acts, .row.menu .acts { opacity: 1; }
  .kind-txt { flex: none; }
  .m-acts { flex: none; display: flex; align-items: center; gap: 2px; }
  .mb { flex: none; width: 38px; height: 38px; display: grid; place-items: center; border-radius: 50%; }
  .mb:active { background: rgba(255, 255, 255, .08); }
  :global(html.compact) .row { gap: 12px; padding: 6px 4px; }
  .du { flex: none; width: 44px; font-size: var(--sm); color: var(--faint); }
</style>
