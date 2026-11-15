<script lang="ts">
  // lists/list_view.rs liked_by_artist: one artist's liked songs read like a
  // "Show all" page (no cover column): back pill, column gap 16; the round
  // play button (44) left of the heading (gap 14), under it (gap 6) the
  // artist link and the meta line (gap 10), then the song rows.
  import SongRow from "../../components/SongRow.svelte";
  import SectionTitle from "../../components/SectionTitle.svelte";
  import EmptyState from "../../components/EmptyState.svelte";
  import RoundPlay from "./RoundPlay.svelte";
  import { app } from "../../state.svelte";
  import { t, tf } from "../../i18n.svelte";
  import { listMeta } from "../../fmt";
  import { playingFrom } from "./actions";
  import type { ListKey } from "../../api/types";
  let { artist }: { artist: string } = $props();
  const l = $derived<ListKey>({ kind: "artist_liked", artist });
  const tracks = $derived(app.listTracks(l));
  const here = $derived(playingFrom(tracks));
  const total = $derived(tracks.reduce((s, x) => s + (x.duration_ms ?? 0), 0));
  function play() { if (here) app.togglePlay(); else app.play(tracks, 0, app.listTitle(l)); }
</script>

<div class="lba">
  <div class="hd">
    <div class="row">
      {#if tracks.length}<RoundPlay size={44} playing={here && app.playing} loading={here && app.loading} onclick={play} />{/if}
      <SectionTitle heading title={t("library.liked")} />
    </div>
    <div class="info">
      <button class="link trunc" onclick={() => app.openArtistPage(artist)}>{artist}</button>
      <span class="meta">{listMeta(tracks.length, total, t("search.kind.song"), tf("common.n_songs", { n: tracks.length }))}</span>
    </div>
  </div>
  {#if !tracks.length}
    <EmptyState title={t("library.liked.empty")} sub={t("library.liked.empty.sub")} />
  {:else}
    <div class="rows">
      {#each tracks as _, i (i)}<SongRow {tracks} index={i} from={l} context={app.listTitle(l)} selected={app.selectedRow === i} />{/each}
    </div>
  {/if}
</div>

<style>
  .lba { display: flex; flex-direction: column; gap: 16px; padding-bottom: 20px; }
  .hd { display: flex; flex-direction: column; gap: 6px; }
  .row { display: flex; align-items: center; gap: 14px; }
  .info { display: flex; align-items: center; gap: 10px; font-size: var(--sm); color: var(--dim); }
  .link:hover { text-decoration: underline; }
  .meta { font-size: var(--sm); color: var(--faint); }
  .rows { display: flex; flex-direction: column; }
  .rows > :global(*) { content-visibility: auto; contain-intrinsic-size: auto 56px; }
</style>
