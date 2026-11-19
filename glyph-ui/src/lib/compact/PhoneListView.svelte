<script lang="ts">
  // compact.rs list_view: big cover (62 % of the width, 140..280) centred,
  // title 24 bold, "n songs" 13 dim, actions (download, shuffle, 52 px
  // accent play), then the song rows (60 px pitch). Scrolling down shrinks
  // and fades the cover: its bottom edge stays on the title while the top
  // lags behind, so it seems to sink away under the bar.
  import ListCover from "../components/ListCover.svelte";
  import PlayGlyph from "../components/PlayGlyph.svelte";
  import IconAction from "./IconAction.svelte";
  import Icon from "../components/Icon.svelte";
  import PhoneSongRow from "./PhoneSongRow.svelte";
  import VirtualRows from "../components/VirtualRows.svelte";
  import Skeletons from "../pages/search/Skeletons.svelte";
  import { app } from "../state.svelte";
  import { onWeb } from "../web";
  import { t, tf } from "../i18n.svelte";
  import { listId, trackKey } from "../api/types";
  import { openNameDialog } from "../pages/lists/actions";
  import { phone, scrollFx } from "./cstate.svelte";
  let w = $state(390);
  const l = $derived(app.openList!);
  const tracks = $derived(app.listTracks(l));
  const albumLoading = $derived(l.kind === "album" && !!l.loading && !tracks.length);
  const art = $derived(Math.min(280, Math.max(140, w * 0.62)));
  /** The cover follows the scroll step by step: down to 50 % over as many px
   *  as it is tall, back up when scrolling back; it fades only in the second half. */
  const sk = $derived(Math.min(1, Math.max(0, phone.scroll / Math.max(1, art))));
  const coverFx = $derived.by(() => {
    if (sk <= 0) return { transform: undefined as string | undefined, opacity: 1 };
    const s = 1 - 0.5 * sk;
    // Moving down by what the shrink takes off keeps the bottom edge on the title.
    return { transform: `translateY(${(art * 0.5 * sk).toFixed(1)}px) scale(${s.toFixed(4)})`, opacity: Math.min(1, Math.max(0, (1 - sk) * 2)) };
  });
  // Select mode (Liked Songs, own playlists): tap rows, then remove them together.
  const pickable = $derived(app.canPick(l));
  const pickMode = $derived(app.isPicking(l));
  const pickedN = $derived(pickMode ? tracks.filter((x) => app.isPicked(x)).length : 0);
  const allPicked = $derived(pickMode && pickedN === tracks.length && tracks.length > 0);
  $effect(() => { void listId(l); return () => app.stopPicking(); });
  const here = $derived(app.player.current ? tracks.some((x) => trackKey(x) === app.currentKey) : false);
  const allDl = $derived(tracks.length > 0 && tracks.every((x) => app.isDownloaded(x)));
  function play() {
    if (here) return app.togglePlay();
    const i = tracks.findIndex((x) => app.allowed(x));
    if (i >= 0) app.play(tracks, i, app.listTitle(l));
  }
  function shuffle() { app.setShuffle(true); app.play(tracks, Math.floor(Math.random() * tracks.length), app.listTitle(l)); }
  const noArt = $derived(l.kind === "liked" || l.kind === "artist_liked" || l.kind === "recent" || l.kind === "downloads");
  const emptySub = $derived(
    l.kind === "liked" || l.kind === "artist_liked" ? t("library.liked.empty.sub")
    : l.kind === "recent" ? t("library.recent.empty.sub")
    : l.kind === "downloads" ? t("library.downloads.empty.sub")
    : t("mobile.list.empty"),
  );
</script>

<div class="lv" bind:clientWidth={w}>
  <div class="head" class:no-art={noArt}>
    {#if !noArt}
      {#snippet cover()}<span class="cv" class:sd={scrollFx} style:--art={art} style:transform={scrollFx ? undefined : coverFx.transform} style:opacity={scrollFx ? undefined : coverFx.opacity}><ListCover list={l} size={art} /></span>{/snippet}
      {#if l.kind === "playlist"}
        <button class="art edit" onclick={() => openNameDialog([], l.id)} aria-label={t("list.edit")}>{@render cover()}</button>
      {:else}
        <div class="art">{@render cover()}</div>
      {/if}
    {/if}
    <h1>{app.listTitle(l)}</h1>
    <div class="n">
      {#if albumLoading && l.kind === "album"}
        {l.artist}
      {:else}
        {tf("common.n_songs", { n: tracks.length })}{#if l.kind === "artist_liked"} · <button class="link" onclick={() => app.openArtistPage(l.artist)}>{l.artist}</button>{/if}
      {/if}
    </div>
    {#if !pickMode}
    <div class="acts">
      {#if tracks.length && !onWeb()}<IconAction name="arrow-down-to-line" label={t("tip.download")} bg={allDl ? "rgba(255,255,255,.1)" : undefined} onclick={() => app.download(tracks)} />{/if}
      {#if l.kind === "playlist"}<IconAction name="pencil" label={t("list.edit")} onclick={() => openNameDialog([], l.id)} />{/if}
      {#if pickable && tracks.length}<IconAction name="circle-check" label={t("pick.select")} onclick={() => app.startPicking(l)} />{/if}
      <span class="flex1"></span>
      {#if tracks.length}
        <IconAction name="shuffle" label={t("tip.shuffle")} onclick={shuffle} />
        <button class="play" onclick={play} aria-label={t("common.play")}><PlayGlyph loading={here && app.loading} playing={here && app.playing} size={20} button={52} /></button>
      {/if}
    </div>
    {/if}
  </div>
  {#if pickMode}
    <div class="picks">
      <div class="pickbar" role="toolbar">
        <button class="pk" onclick={() => app.stopPicking()}>{t("pick.done")}</button>
        <span class="pick-n">{tf("pick.n", { n: pickedN })}</span>
        <button class="pk danger" disabled={!pickedN} onclick={() => app.removePicked(l)} aria-label={t("pick.remove")}><Icon name="trash-2" size={18} />{t("pick.remove")}</button>
      </div>
    </div>
    <div class="pick-more">
      <button class="chip" onclick={() => app.pickAll(tracks)}><Icon name={allPicked ? "circle" : "circle-check"} size={15} />{t(allPicked ? "pick.none" : "pick.all")}</button>
      {#if l.kind === "liked"}<button class="chip" onclick={() => app.clearLiked()}><Icon name="trash-2" size={15} />{t("liked.clear")}</button>{/if}
    </div>
  {/if}
  {#if albumLoading}
    <div class="rows"><Skeletons kind="rows" n={8} prefix="album" /></div>
  {:else if !tracks.length}
    <p class="empty">{emptySub}</p>
  {:else}
    <div class="rows">
      <VirtualRows count={tracks.length} pitch={60} overscan={12}>
        {#snippet row(i)}
          <PhoneSongRow {tracks} index={i} from={l.kind === "album" ? null : l} context={app.listTitle(l)} />
        {/snippet}
      </VirtualRows>
    </div>
  {/if}
  <div style:height="24px"></div>
</div>

<style>
  /* Starts under the status bar (the bar above is see-through until the title comes up). */
  .lv { display: flex; flex-direction: column; gap: 8px; padding-top: var(--safe-top, env(safe-area-inset-top)); }
  .head { display: flex; flex-direction: column; gap: 4px; padding: 0 16px; }
  .head.no-art { padding-top: 56px; }
  .art { display: flex; justify-content: center; padding: 14px 0 16px; }
  .art.edit { align-self: center; transition: transform .14s var(--ease); }
  .art.edit:active { transform: scale(.98); }
  .cv { display: block; transform-origin: 50% 0; will-change: transform, opacity; }
  /* Same effect as coverFx, run by the browser along the page scroll. */
  .cv.sd { animation: cv-sink linear both; animation-timeline: --page; animation-range: 0px calc(var(--art) * 1px); }
  @keyframes cv-sink {
    0% { transform: none; opacity: 1; }
    50% { opacity: 1; }
    100% { transform: translateY(calc(var(--art) * .5px)) scale(.5); opacity: 0; }
  }
  h1 { margin: 0; font-size: 24px; font-weight: 700; color: var(--text); }
  .n { font-size: 13px; color: var(--dim); }
  .link:hover { color: var(--text); text-decoration: underline; }
  .acts { margin-top: 8px; display: flex; align-items: center; gap: 4px; }
  .play { flex: none; width: 52px; height: 52px; border-radius: 50%; background: var(--accent); display: grid; place-items: center; }
  /* Stays under the top bar while the rows scroll. */
  .picks { position: sticky; top: calc(52px + var(--safe-top, env(safe-area-inset-top))); z-index: 3; padding: 0 16px; display: flex; flex-direction: column; }
  .pickbar { margin-top: 4px; height: 52px; padding: 0 6px; display: flex; align-items: center; gap: 4px; border-radius: 14px; background: #161618; }
  .pick-n { flex: 1; min-width: 0; padding: 0 6px; font-size: 15px; font-weight: 600; color: var(--text); white-space: nowrap; }
  .pk { flex: none; height: 40px; padding: 0 12px; display: inline-flex; align-items: center; gap: 6px; border-radius: 20px; font-size: 14px; font-weight: 500; color: rgba(255, 255, 255, .85); }
  .pk:active { background: rgba(255, 255, 255, .08); }
  .pk.danger { color: #fff; background: var(--danger); }
  .pk.danger:disabled { opacity: .4; }
  .pick-more { padding: 8px 16px 2px; display: flex; flex-wrap: wrap; gap: 8px; }
  .chip { height: 36px; padding: 0 12px 0 10px; display: inline-flex; align-items: center; gap: 6px; border-radius: 18px; font-size: 13.5px; color: rgba(255, 255, 255, .8); background: rgba(255, 255, 255, .07); }
  .chip:active { background: rgba(255, 255, 255, .13); }
  .empty { margin: 0; padding: 8px 16px 0; font-size: 14px; color: var(--dim); }
  .rows { padding: 0 16px; }
  .rows :global(.row) { height: 60px; }
</style>
