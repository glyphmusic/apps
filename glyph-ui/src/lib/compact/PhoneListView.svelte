<script lang="ts">
  // compact.rs list_view: big cover (62 % of the width, 140..280) centred,
  // title 24 bold, "n songs" 13 dim, actions (download, shuffle, 52 px
  // accent play), then the song rows (60 px pitch).
  import ListCover from "../components/ListCover.svelte";
  import PlayGlyph from "../components/PlayGlyph.svelte";
  import IconAction from "./IconAction.svelte";
  import PhoneSongRow from "./PhoneSongRow.svelte";
  import Skeletons from "../pages/search/Skeletons.svelte";
  import { app } from "../state.svelte";
  import { onWeb } from "../web";
  import { t, tf } from "../i18n.svelte";
  import { trackKey } from "../api/types";
  import { openNameDialog } from "../pages/lists/actions";
  let w = $state(390);
  const l = $derived(app.openList!);
  const tracks = $derived(app.listTracks(l));
  const albumLoading = $derived(l.kind === "album" && !!l.loading && !tracks.length);
  const art = $derived(Math.min(280, Math.max(140, w * 0.62)));
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
      {#if l.kind === "playlist"}
        <button class="art edit" onclick={() => openNameDialog([], l.id)} aria-label={t("list.edit")}><ListCover list={l} size={art} /></button>
      {:else}
        <div class="art"><ListCover list={l} size={art} /></div>
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
    <div class="acts">
      {#if tracks.length && !onWeb()}<IconAction name="arrow-down-to-line" label={t("tip.download")} bg={allDl ? "rgba(255,255,255,.1)" : undefined} onclick={() => app.download(tracks)} />{/if}
      {#if l.kind === "playlist"}<IconAction name="pencil" label={t("list.edit")} onclick={() => openNameDialog([], l.id)} />{/if}
      <span class="flex1"></span>
      {#if tracks.length}
        <IconAction name="shuffle" label={t("tip.shuffle")} onclick={shuffle} />
        <button class="play" onclick={play} aria-label={t("common.play")}><PlayGlyph loading={here && app.loading} playing={here && app.playing} size={20} button={52} /></button>
      {/if}
    </div>
  </div>
  {#if albumLoading}
    <div class="rows"><Skeletons kind="rows" n={8} prefix="album" /></div>
  {:else if !tracks.length}
    <p class="empty">{emptySub}</p>
  {:else}
    <div class="rows">
      {#each tracks as _, i (i)}
        <PhoneSongRow {tracks} index={i} from={l.kind === "album" ? null : l} context={app.listTitle(l)} />
      {/each}
    </div>
  {/if}
  <div style:height="24px"></div>
</div>

<style>
  .lv { display: flex; flex-direction: column; gap: 8px; }
  .head { display: flex; flex-direction: column; gap: 4px; padding: 0 16px; }
  .head.no-art { padding-top: 56px; }
  .art { display: flex; justify-content: center; padding: 14px 0 16px; }
  .art.edit { align-self: center; transition: transform .14s var(--ease); }
  .art.edit:active { transform: scale(.98); }
  h1 { margin: 0; font-size: 24px; font-weight: 700; color: var(--text); }
  .n { font-size: 13px; color: var(--dim); }
  .link:hover { color: var(--text); text-decoration: underline; }
  .acts { margin-top: 8px; display: flex; align-items: center; gap: 4px; }
  .play { flex: none; width: 52px; height: 52px; border-radius: 50%; background: var(--accent); display: grid; place-items: center; }
  .empty { margin: 0; padding: 8px 16px 0; font-size: 14px; color: var(--dim); }
  .rows { padding: 0 16px; }
  .rows :global(.row) { height: 60px; }
</style>
