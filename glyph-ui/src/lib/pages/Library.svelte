<script lang="ts">
  // lists/library.rs library: heading + inline quick filter & sort + actions,
  // tiles (Liked, Recent, Downloads, own playlists, saved albums), followed
  // artists below. Phones (compact.rs library): filter chips Playlists / Albums / Artists.
  import { slide } from "svelte/transition";
  import { tick } from "svelte";
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import Button from "../components/Button.svelte";
  import EmptyState from "../components/EmptyState.svelte";
  import ListCover from "../components/ListCover.svelte";
  import SectionTitle from "../components/SectionTitle.svelte";
  import Chip from "../components/Chip.svelte";
  import Select from "../settings/Select.svelte";
  import Spin from "../settings/Spin.svelte";
  import ArtistCard from "./lists/ArtistCard.svelte";
  import { app } from "../state.svelte";
  import { phone } from "../compact/cstate.svelte";
  import { onWeb } from "../web";
  import { t, tf } from "../i18n.svelte";
  import { sameList, type ListKey } from "../api/types";
  import { playlistX, scopeKey } from "./lists/actions";
  import { filterAndSortTiles, filterArtists, type LibrarySort, type LibraryTile } from "./lists/library-filter";
  import { mg, migrateRunning, serviceOf } from "../settings/migrate-state.svelte";

  const running = $derived(migrateRunning());
  const runSvc = $derived(running && mg.service ? serviceOf(mg.service) : null);
  const runPct = $derived(running && running.total > 0 ? Math.min(100, Math.round((running.done * 100) / running.total)) : 0);

  const touch = $derived(app.platform === "android" || app.platform === "ios");
  const filter = $derived(app.compact ? app.libraryTab : 0);

  let filterOpen = $state(false);
  let filterQ = $state("");
  let filterInput: HTMLInputElement | undefined = $state();
  let sort = $state<LibrarySort>("default");

  const sortOptions = $derived([
    { value: "default" as LibrarySort, label: t("library.sort.default"), icon: "pin" },
    { value: "name" as LibrarySort, label: t("library.sort.name"), icon: "arrow-up-down" },
    { value: "songs" as LibrarySort, label: t("library.sort.songs"), icon: "list-music" },
  ]);

  function openFilter() {
    filterOpen = true;
    tick().then(() => filterInput?.focus());
  }
  function clearFilter(close = true) {
    filterQ = "";
    if (close) filterOpen = false;
  }

  const rawTiles = $derived.by<LibraryTile[]>(() => {
    const out: { l: ListKey; kind: string }[] = [];
    const pl = t("list.kind.playlist");
    if (filter <= 1) {
      out.push({ l: { kind: "liked" }, kind: pl });
      if (filter === 0 && app.library.recent.length) out.push({ l: { kind: "recent" }, kind: pl });
      if (filter === 0 && app.library.downloads.length) out.push({ l: { kind: "downloads" }, kind: pl });
      for (const p of app.library.playlists) out.push({ l: { kind: "playlist", id: p.id }, kind: t(scopeKey(playlistX(p.id))) });
    }
    if (filter === 0 || filter === 2) {
      for (const k of app.library.saved_albums) out.push({ l: k, kind: k.kind === "album" ? `${t("list.kind.album")} · ${k.artist}` : t("list.kind.album") });
    }
    const rank = (l: ListKey) => { const i = app.library.pinned.findIndex((p) => sameList(p, l)); return i < 0 ? Infinity : i; };
    return out.map((x, i) => ({
      l: x.l,
      kind: x.kind,
      title: app.listTitle(x.l),
      count: app.listTracks(x.l).length,
      pinRank: rank(x.l),
      origIndex: i,
    }));
  });

  const tiles = $derived(filterAndSortTiles(rawTiles, filterQ, sort));
  const rawFollowed = $derived(filter === 0 || filter === 3 ? app.library.followed : []);
  const followed = $derived(filterArtists(rawFollowed, filterQ, sort));
  const filterEmpty = $derived(filterQ.trim().length > 0 && tiles.length === 0 && followed.length === 0);

  function menu(e: MouseEvent, l: ListKey) {
    e.preventDefault(); e.stopPropagation();
    const first = app.listTracks(l)[0];
    if (!first && l.kind !== "playlist") return;
    app.openCtx({ x: e.clientX, y: e.clientY, track: first ?? null, list: l });
  }
  const pick = (f: number) => {
    const next = app.libraryTab === f ? 0 : f;
    app.libraryTab = next;
    phone.libFilter = next;
  };

  type LibAction = "local" | "migrate" | "stats" | "new";
  const actionOptions = $derived.by(() => {
    const opts: { value?: LibAction; label?: string; icon?: string; sep?: boolean }[] = [];
    if (!touch && !onWeb()) opts.push({ value: "local", label: t("local.title"), icon: "hard-drive" });
    if (!onWeb()) opts.push({ value: "migrate", label: t("library.migrate"), icon: "arrow-right-left" });
    opts.push({ value: "stats", label: t("stats.title"), icon: "chart-column" });
    opts.push({ sep: true });
    opts.push({ value: "new", label: t("library.playlist.new"), icon: "plus" });
    return opts;
  });
  function pickAction(v: LibAction | null) {
    if (v === "local") app.navigate("local");
    else if (v === "migrate") app.openSettings("migrate");
    else if (v === "stats") app.navigate("stats");
    else if (v === "new") app.nameDialog = { tracks: [], rename: null, initial: "" };
  }
</script>

<div class="lib">
  <div class="head">
    <SectionTitle heading title={t("nav.library")} />
    <div class="hacts">
      {#if !app.compact}
        {#if filterOpen || filterQ}
          <div class="qbox">
            <Icon name="search" size={14} color="var(--dim)" />
            <input
              bind:this={filterInput}
              bind:value={filterQ}
              placeholder={t("library.filter.placeholder")}
              spellcheck="false"
              onkeydown={(e) => {
                if (e.key === "Escape") { e.stopPropagation(); clearFilter(true); }
              }}
            />
            <button type="button" class="qclr" aria-label={t("tip.x")} onclick={() => clearFilter(true)}>
              <Icon name="x" size={13} color="var(--dim)" />
            </button>
          </div>
        {:else}
          <IconButton name="search" size={16} box={32} label={t("library.filter.placeholder")} onclick={openFilter} />
        {/if}
        <Select value={sort} options={sortOptions} onpick={(v) => (sort = v)} />
      {/if}
      <Select value={null as LibAction | null} label={t("ctx.more")} options={actionOptions} onpick={pickAction} />
    </div>
  </div>

  {#if running && runSvc}
    <button class="trban" transition:slide={{ duration: 180 }} onclick={() => app.openSettings("migrate")}>
      <div class="trtop">
        <Spin />
        <span class="trtx trunc">{tf("migrate.banner.progress", { service: runSvc.name, i: running.index + 1, n: Math.max(1, running.lists) })}</span>
        {#if running.total > 0}
          <span class="mono trpct">{runPct}%</span>
        {/if}
        <span class="trview">{t("migrate.banner.view")}<Icon name="arrow-right" size={12} color="var(--accent)" /></span>
      </div>
      <div class="trbar"><div class="trfill" class:ind={running.total === 0} style:width={running.total > 0 ? `${runPct}%` : undefined}></div></div>
    </button>
  {/if}

  {#if app.compact}
    <div class="chips">
      <Chip on={filter === 1} onclick={() => pick(1)}>{t("library.filter.playlists")}</Chip>
      <Chip on={filter === 2} onclick={() => pick(2)}>{t("search.filter.albums")}</Chip>
      <Chip on={filter === 3} onclick={() => pick(3)}>{t("search.filter.artists")}</Chip>
    </div>
  {/if}

  {#if filterEmpty}
    <div class="empty-wrap">
      <EmptyState icon="search" title={t("search.none.title")} sub={t("local.none.sub")}>
        <Button onclick={() => clearFilter(true)}>{t("lyrics.timing.reset")}</Button>
      </EmptyState>
    </div>
  {:else}
    {#if tiles.length}
      <div class="grid">
        {#each tiles as { l, kind, title, count } (JSON.stringify({ ...l, tracks: undefined }))}
          <button class="tile" onclick={() => app.openListView(l)} oncontextmenu={(e) => menu(e, l)}>
            <span class="art">
              <ListCover list={l} size={168} />
              {#if app.isPinned(l)}<span class="pin"><Icon name="pin-fill" size={12} color="var(--accent)" /></span>{/if}
            </span>
            <span class="tl trunc">{title}</span>
            <span class="sub trunc">{kind} · {tf("common.n_songs", { n: count })}</span>
          </button>
        {/each}
      </div>
    {/if}

    {#if followed.length}
      <div class="artists">
        <SectionTitle icon="user-round" title={t("library.artists")} />
        <div class="grid">
          {#each followed as name (name)}<ArtistCard {name} />{/each}
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .lib { display: flex; flex-direction: column; gap: 18px; }
  .head { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 12px; }
  .hacts { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
  .qbox { display: flex; align-items: center; gap: 8px; width: 204px; height: 32px; padding: 0 8px 0 10px; border-radius: 8px; background: #1a1a1d; transition: background-color .14s; }
  .qbox:focus-within { background: #242428; }
  .qbox input { flex: 1; min-width: 0; background: none; border: 0; padding: 0; outline: none; font-size: var(--sm); color: var(--text); }
  .qbox input::placeholder { color: var(--faint); }
  .qclr { flex: none; width: 22px; height: 22px; border-radius: 5px; display: grid; place-items: center; transition: background-color .12s; }
  .qclr:hover { background: rgba(255, 255, 255, .1); }
  .empty-wrap { padding-top: 12px; }
  .trban {
    display: flex; flex-direction: column; gap: 8px; padding: 11px 14px; border-radius: 10px;
    background: rgba(255, 255, 255, .045); border: 1px solid rgba(255, 255, 255, .07);
    text-align: left; transition: background-color .14s ease-out, border-color .14s ease-out;
  }
  .trban:hover { background: rgba(255, 255, 255, .075); border-color: rgba(255, 255, 255, .13); }
  .trtop { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .trtx { flex: 1; min-width: 0; font-size: 13px; font-weight: 500; color: #fff; }
  .trpct { font-size: 12px; color: rgba(255, 255, 255, .65); }
  .trview { display: inline-flex; align-items: center; gap: 4px; font-size: 12px; font-weight: 500; color: var(--accent); flex: none; }
  .trbar { height: 4px; width: 100%; border-radius: 999px; background: rgba(255, 255, 255, .08); overflow: hidden; }
  .trfill { height: 100%; border-radius: 999px; background: var(--accent); transition: width .18s ease-out; }
  .trfill.ind { width: 35%; animation: trslide 1.2s ease-in-out infinite; }
  @keyframes trslide { 0% { transform: translateX(-110%); } 100% { transform: translateX(300%); } }
  .chips { display: flex; gap: 8px; }
  .grid { display: flex; flex-wrap: wrap; gap: 8px; margin-left: -8px; }
  .tile { flex: none; width: 184px; padding: 8px; border-radius: 10px; display: flex; flex-direction: column; gap: 4px; transition: background-color .12s ease-out; animation: rise-in .25s var(--ease) both; }
  .tile:hover { background: rgba(255, 255, 255, .05); }
  .art { position: relative; display: block; }
  .pin { position: absolute; top: 8px; right: 8px; width: 24px; height: 24px; border-radius: 50%; background: rgba(0, 0, 0, .6); display: grid; place-items: center; animation: pop .26s var(--ease); }
  .tl { padding-top: 6px; font-size: var(--md); font-weight: 500; color: var(--text); }
  .sub { font-size: var(--sm); color: var(--dim); }
  .artists { display: flex; flex-direction: column; gap: 10px; padding-top: 10px; }
</style>
