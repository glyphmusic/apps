<script lang="ts">
  // lists/list_view.rs liked_by_artist: one artist's liked songs read like a
  // "Show all" page (no cover column): back pill, column gap 16; the round
  // play button (44) left of the heading (gap 14), under it (gap 6) the
  // artist link and the meta line (gap 10), then the song rows.
  import SongRow from "../../components/SongRow.svelte";
  import SectionTitle from "../../components/SectionTitle.svelte";
  import EmptyState from "../../components/EmptyState.svelte";
  import Icon from "../../components/Icon.svelte";
  import RoundPlay from "./RoundPlay.svelte";
  import { app } from "../../state.svelte";
  import { registerListNav, scrollDomRowIntoView } from "../../state/list-nav.svelte";
  import { t, tf } from "../../i18n.svelte";
  import { listMeta } from "../../fmt";
  import { tip } from "../../ui.svelte";
  import { listSort, listSortDir, playingFrom, removeShown, setListSort, shownTracks } from "./actions";
  import type { ListKey } from "../../api/types";
  let { artist }: { artist: string } = $props();
  const l = $derived<ListKey>({ kind: "artist_liked", artist });
  const tracks = $derived(shownTracks(l));
  const sort = $derived(listSort(l));
  const dir = $derived(listSortDir(l));
  const here = $derived(playingFrom(tracks));
  const total = $derived(tracks.reduce((s, x) => s + (x.duration_ms ?? 0), 0));
  function play() { if (here) app.togglePlay(); else app.play(tracks, 0, app.listTitle(l)); }

  const sortLabel = $derived.by(() => {
    switch (sort) {
      case "title": return t("sort.by_title");
      case "artist": return t("sort.by_artist");
      case "album": return t("sort.by_album");
      case "duration": return t("sort.by_length");
      default: return t("sort.added");
    }
  });
  function sortMenu(e: PointerEvent) {
    e.stopPropagation();
    app.controlMenu = { menu: "sort", x: e.clientX, y: e.clientY };
  }
  function cycleTitleSort() {
    if (sort !== "title" && sort !== "artist") setListSort(l, "title", "asc");
    else if (sort === "title" && dir === "asc") setListSort(l, "title", "desc");
    else if (sort === "title") setListSort(l, "artist", "asc");
    else if (dir === "asc") setListSort(l, "artist", "desc");
    else setListSort(l, "custom", "asc");
  }
  let rowsEl: HTMLDivElement | undefined = $state();
  $effect(() => {
    return registerListNav({
      count: () => tracks.length,
      play: (i) => {
        const tr = tracks[i];
        if (tr && app.allowed(tr)) app.play(tracks, i, app.listTitle(l));
      },
      remove: (i) => removeShown(l, i),
      scrollTo: (i) => scrollDomRowIntoView(rowsEl, i),
    });
  });

  function toggleColSort(next: "album" | "duration") {
    if (sort !== next) setListSort(l, next, "asc");
    else if (dir === "asc") setListSort(l, next, "desc");
    else setListSort(l, "custom", "asc");
  }
</script>

{#snippet sortArrow(d: "asc" | "desc")}
  <svg class="sarr" width="9" height="6" viewBox="0 0 9 6" aria-hidden="true">
    {#if d === "asc"}<polygon points="4.5,0.5 8.5,5.5 0.5,5.5" />{:else}<polygon points="0.5,0.5 8.5,0.5 4.5,5.5" />{/if}
  </svg>
{/snippet}

<div class="lba">
  <div class="hd">
    <div class="row">
      {#if tracks.length}<RoundPlay size={44} playing={here && app.playing} loading={here && app.loading} onclick={play} />{/if}
      <div class="titles">
        <SectionTitle heading title={t("library.liked")} />
        <div class="info">
          <span class="meta">{listMeta(tracks.length, total, t("search.kind.song"), tf("common.n_songs", { n: tracks.length }))}</span>
          <span class="sep">·</span>
          <button class="link trunc" onclick={() => app.openArtistPage(artist)}>{artist}</button>
        </div>
      </div>
      {#if tracks.length}
        <button type="button" class="sort-btn" class:on={sort !== "custom"} use:tip={t("tip.arrow-up-down")} onpointerdown={sortMenu}>
          <span class="trunc">{sortLabel}</span>
          {#if sort !== "custom"}{@render sortArrow(dir)}{/if}
          <Icon name="arrow-up-down" size={15} />
        </button>
      {/if}
    </div>
  </div>
  {#if !tracks.length}
    <EmptyState title={t("library.liked.empty")} sub={t("library.liked.empty.sub")} />
  {:else}
    {@const titleActive = sort === "title" || sort === "artist"}
    <div class="colhead">
      <button type="button" class="ch ch-num" onclick={() => setListSort(l, "custom", "asc")}>#</button>
      <button type="button" class="ch ch-title" class:on={titleActive} onclick={cycleTitleSort}>
        <span>{sort === "artist" ? t("sort.by_artist") : t("sort.by_title")}</span>
        {#if titleActive}{@render sortArrow(dir)}{/if}
      </button>
      <button type="button" class="ch ch-al" class:on={sort === "album"} onclick={() => toggleColSort("album")}>
        <span>{t("sort.by_album")}</span>
        {#if sort === "album"}{@render sortArrow(dir)}{/if}
      </button>
      <span class="ch-like"></span>
      <button type="button" class="ch ch-du" class:on={sort === "duration"} aria-label={t("sort.by_length")} onclick={() => toggleColSort("duration")}>
        <Icon name="clock" size={14} />
        {#if sort === "duration"}{@render sortArrow(dir)}{/if}
      </button>
      <span class="ch-more"></span>
    </div>
    <div class="rows" bind:this={rowsEl}>
      {#each tracks as _, i (i)}<SongRow {tracks} index={i} from={l} context={app.listTitle(l)} selected={app.selectedRow === i} />{/each}
    </div>
  {/if}
</div>

<style>
  .lba { display: flex; flex-direction: column; gap: 14px; padding-bottom: 20px; }
  .hd { display: flex; flex-direction: column; }
  .row { display: flex; align-items: center; gap: 14px; min-width: 0; }
  .titles { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .info { display: flex; align-items: center; gap: 8px; font-size: var(--sm); color: var(--dim); min-width: 0; }
  .sep { color: var(--faint); }
  .link:hover { text-decoration: underline; color: var(--text); }
  .meta { font-size: var(--sm); color: var(--faint); }
  .sort-btn { margin-left: auto; flex: none; display: inline-flex; align-items: center; gap: 7px; max-width: 220px; height: 32px; padding: 0 10px; border-radius: 8px; font-size: var(--sm); color: var(--dim); transition: background-color .12s ease-out, color .12s; }
  .sort-btn:hover { background: var(--hover); color: var(--text); }
  .sort-btn.on { color: var(--accent); background: rgba(var(--accent-rgb), .12); }
  .colhead { container-type: inline-size; display: flex; align-items: center; gap: 14px; height: 34px; padding: 0 10px; border-bottom: 1px solid rgba(255, 255, 255, .08); font-size: 12px; font-weight: 500; color: var(--faint); }
  .ch { display: inline-flex; align-items: center; gap: 6px; text-align: left; color: inherit; transition: color .12s; white-space: nowrap; }
  button.ch:hover, button.ch.on { color: var(--text); }
  .sarr { flex: none; fill: var(--accent); }
  .ch-num { flex: none; width: 26px; justify-content: center; text-align: center; }
  .ch-title { flex: 1; min-width: 0; }
  .ch-al { flex: 0 1 200px; width: 200px; min-width: 0; }
  @container (max-width: 699px) { .ch-al { display: none; } }
  .ch-like { flex: none; width: 29px; }
  .ch-du { flex: none; width: 40px; padding-left: 6px; gap: 4px; }
  .ch-more { flex: none; width: 28px; }
  @container (max-width: 360px) { .ch-num { width: 20px; } .ch-du { display: none; } }
  .rows { display: flex; flex-direction: column; }
  .rows > :global(*) { content-visibility: auto; contain-intrinsic-size: auto 56px; }
</style>
