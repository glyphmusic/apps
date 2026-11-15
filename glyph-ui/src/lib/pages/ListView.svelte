<script lang="ts">
  // Desktop & web list view (Spotify-inspired top hero + unified control bar
  // above full-width song rows). `liked`, `recent` and `downloads` omit cover
  // art (`noCover`) by design; playlists, albums and shared lists display their
  // cover in the top hero with extracted subtle header tinting. Scrolling past
  // the action bar docks a frosted compact bar + column header at the top of
  // `#main-scroll`.
  import { tick } from "svelte";
  import Icon from "../components/Icon.svelte";
  import VirtualRows from "../components/VirtualRows.svelte";
  import IconButton from "../components/IconButton.svelte";
  import ToggleButton from "../components/ToggleButton.svelte";
  import Button from "../components/Button.svelte";
  import ListCover from "../components/ListCover.svelte";
  import SongRow from "../components/SongRow.svelte";
  import Cover from "../components/Cover.svelte";
  import SongTitle from "../components/SongTitle.svelte";
  import DownloadBadge from "../components/DownloadBadge.svelte";
  import EmptyState from "../components/EmptyState.svelte";
  import Skeletons from "./search/Skeletons.svelte";
  import Select from "../settings/Select.svelte";
  import LikedByArtist from "./lists/LikedByArtist.svelte";
  import RecentArtistRow from "./lists/RecentArtistRow.svelte";
  import RoundPlay from "./lists/RoundPlay.svelte";
  import { app } from "../state.svelte";
  import { onWeb } from "../web";
  import { backend } from "../api/backend";
  import { paletteFromImage, type Rgb } from "../glow";
  import { t, tf } from "../i18n.svelte";
  import { ink, pageChrome, tip } from "../ui.svelte";
  import { listMeta } from "../fmt";
  import { rating, trackKey, type ListKey, type Track } from "../api/types";
  import {
    downloadTip, downloadsView, listSort, listSortDir, playShuffled, playingFrom, playlistX,
    removeDownloads, scopeKey, setListSort, shownTracks, sortId, openNameDialog,
  } from "./lists/actions";

  const PITCH = 56;
  const PIN_H = 56;
  const l = $derived(app.openList as ListKey);
  const tracks = $derived(l ? shownTracks(l) : []);
  const title = $derived(l ? app.listTitle(l) : "");
  const has = $derived(tracks.length > 0);
  const own = $derived(l?.kind === "playlist" ? playlistX(l.id) : undefined);
  const author = $derived(l?.kind === "album" && l.artist.startsWith("@") ? l.artist : null);
  const sort = $derived(l ? listSort(l) : "custom");
  const dir = $derived(l ? listSortDir(l) : "asc");
  const total = $derived(tracks.reduce((s, x) => s + (x.duration_ms ?? 0), 0));
  const noCover = $derived(l?.kind === "liked" || l?.kind === "recent" || l?.kind === "downloads");

  // Bottom recommended songs for own playlists (same suggestion engine as AddSongsPanel).
  let rawRecs = $state<Track[]>([]);
  let recsLoading = $state(false);
  let recEpoch = $state(0);
  const ownId = $derived(own?.id ?? "");
  const haveKeys = $derived(new Set((own?.tracks ?? []).map(trackKey)));
  $effect(() => {
    const pid = ownId;
    void recEpoch;
    rawRecs = [];
    if (!pid) { recsLoading = false; return; }
    let alive = true;
    recsLoading = true;
    backend().call<Track[]>("add_suggestions", { id: pid }).then((v) => {
      if (!alive) return;
      rawRecs = (v ?? []).filter((x) => app.allowed(x));
      recsLoading = false;
    }).catch(() => { if (alive) recsLoading = false; });
    return () => { alive = false; };
  });
  const recTracks = $derived(rawRecs.filter((x) => !haveKeys.has(trackKey(x))).slice(0, 6));
  function addRec(tr: Track) { if (own) app.addToPlaylist(own.id, [tr]); }
  function refreshRecs() { recEpoch++; }

  let lvW = $state(900);
  const narrow = $derived(lvW < 540);
  const coverSize = $derived(lvW >= 740 ? 180 : 144);
  const showAlbumCol = $derived(!!l && (l.kind !== "album" || !!author) && lvW >= 700);
  const heroTitleSize = $derived(
    narrow ? "calc(24px * var(--k))"
    : lvW < 740 ? "calc(clamp(24px, 3.6vw, 32px) * var(--k))"
    : "calc(clamp(28px, 3.4vw, 42px) * var(--k))",
  );

  // Subtle cover-derived tint for covered headers; dark accent/neutral tint otherwise.
  let tint = $state<Rgb | null>(null);
  const coverTrack = $derived(l?.kind === "album" ? (l.cover ?? tracks[0] ?? null) : (tracks[0] ?? null));
  const firstArtKey = $derived(!noCover ? (own?.cover ?? (coverTrack ? trackKey(coverTrack) : "")) : "");
  $effect(() => {
    void firstArtKey;
    tint = null;
    if (noCover || !l) return;
    let alive = true;
    const applyFromUrl = async (url: string | null) => {
      if (!alive || !url) return;
      const pal = await paletteFromImage(url);
      if (!alive || !pal) return;
      const c = pal[1] ?? pal[0];
      const maxC = Math.max(c[0], c[1], c[2], 0.01);
      const cap = maxC > 0.72 ? 0.72 / maxC : 1;
      tint = [c[0] * cap, c[1] * cap, c[2] * cap];
    };
    if (own?.cover) void applyFromUrl(own.cover);
    else if (coverTrack) backend().cover(coverTrack).then(applyFromUrl);
    return () => { alive = false; };
  });

  const heroWash = $derived.by(() => {
    if (tint) {
      const [r, g, b] = tint.map((v) => Math.round(v * 255));
      return `linear-gradient(180deg, rgba(${r}, ${g}, ${b}, .34) 0%, rgba(${r}, ${g}, ${b}, .12) 58%, rgba(${r}, ${g}, ${b}, 0) 100%)`;
    }
    if (l?.kind === "liked" || l?.kind === "downloads") {
      return "linear-gradient(180deg, rgba(var(--accent-rgb), .24) 0%, rgba(var(--accent-rgb), .08) 58%, rgba(var(--accent-rgb), 0) 100%)";
    }
    return "linear-gradient(180deg, rgba(255, 255, 255, .07) 0%, rgba(255, 255, 255, .02) 58%, rgba(255, 255, 255, 0) 100%)";
  });
  const pinBg = $derived.by(() => {
    if (tint) {
      const [r, g, b] = tint.map((v) => Math.round(14 + v * 46));
      return `rgb(${r}, ${g}, ${b})`;
    }
    if (l?.kind === "liked" || l?.kind === "downloads") {
      return "color-mix(in srgb, var(--accent) 16%, #111114)";
    }
    return "#141417";
  });

  // Kind label; own playlists and Downloads add how much is saved offline.
  let bytes = $state(0);
  $effect(() => {
    if (!(l?.kind === "playlist" || l?.kind === "downloads")) return;
    const ts = $state.snapshot(tracks) as Track[];
    void app.library.downloads.length;
    let alive = true;
    backend().call<number>("downloaded_bytes", { tracks: ts }).then((b) => alive && (bytes = b ?? 0));
    return () => { alive = false; };
  });
  const kind = $derived.by(() => {
    if (!l) return "";
    const base =
      l.kind === "playlist" ? t(scopeKey(own))
      : l.kind === "album" && !author && !app.sharedPreview ? t("list.kind.album")
      : t("list.kind.playlist");
    if (l.kind !== "playlist" && l.kind !== "downloads") return base;
    const saved = tracks.filter((x) => app.isDownloaded(x)).length;
    return `${base} · ${tf("playlist.offline_progress", { done: saved, total: tracks.length })} · ${(bytes / 1e6).toFixed(1)} MB`;
  });

  // Playing from this list: the button pauses / resumes it.
  const here = $derived(has && playingFrom(tracks) && (app.queue.context == null || app.queue.context === title));
  const dl = $derived(downloadsView(tracks));
  const saveable = $derived(l?.kind === "album" && !app.sharedPreview);
  async function addShared() {
    const sp = app.sharedPreview;
    if (!sp) return;
    const id = await backend().call<string | null>("share_save", { code: sp.code, link: sp.link || null, author: sp.author });
    app.sharedPreview = null;
    if (id) app.openListView({ kind: "playlist", id });
  }
  const adding = $derived(!!own && app.addSongs?.id === own.id);

  function playAll() { if (here) app.togglePlay(); else app.play(tracks, 0, title); }
  function addSongs() {
    if (!own) return;
    if (adding) { app.addSongs = null; return; }
    app.showQueue = false;
    app.addSongs = { id: own.id, tab: 0 };
  }
  function openImportTab() {
    if (!own) return;
    app.showQueue = false;
    app.addSongs = { id: own.id, tab: 3 };
  }
  const importOptions = $derived(
    onWeb()
      ? [{ value: "link" as const, label: t("add.tab.import"), icon: "link-2" }]
      : [
          { value: "link" as const, label: t("add.tab.import"), icon: "link-2" },
          { value: "file" as const, label: t("dialog.playlist.file"), icon: "folder-open" },
        ],
  );
  function pickImport(v: "link" | "file" | null) {
    if (!own || !v) return;
    if (v === "link") openImportTab();
    else backend().call("import_playlist_file", { playlist_id: own.id });
  }
  function sortMenu(e: PointerEvent) {
    e.stopPropagation();
    app.ctx = null;
    app.controlMenu = { menu: "sort", x: e.clientX, y: e.clientY };
  }
  function more(e: PointerEvent) {
    e.stopPropagation();
    app.openCtx({ x: e.clientX, y: e.clientY, track: tracks[0] ?? null, list: l });
  }
  const sortLabel = $derived.by(() => {
    switch (sort) {
      case "title": return t("sort.by_title");
      case "artist": return t("sort.by_artist");
      case "album": return t("sort.by_album");
      case "duration": return t("sort.by_length");
      default: return l?.kind === "liked" ? t("sort.added") : t("sort.custom");
    }
  });
  function cycleTitleSort() {
    if (!l || !sortId(l)) return;
    if (sort !== "title" && sort !== "artist") setListSort(l, "title", "asc");
    else if (sort === "title" && dir === "asc") setListSort(l, "title", "desc");
    else if (sort === "title") setListSort(l, "artist", "asc");
    else if (dir === "asc") setListSort(l, "artist", "desc");
    else setListSort(l, "custom", "asc");
  }
  function toggleColSort(next: "album" | "duration") {
    if (!l || !sortId(l)) return;
    if (sort !== next) setListSort(l, next, "asc");
    else if (dir === "asc") setListSort(l, next, "desc");
    else setListSort(l, "custom", "asc");
  }

  // Inline list filter ("Search songs")
  let searchOpen = $state(false);
  let filterQ = $state("");
  let searchInput: HTMLInputElement | undefined = $state();
  const q = $derived(filterQ.trim().toLowerCase());
  const matchTrack = (tr: Track) =>
    !q || tr.title.toLowerCase().includes(q) || tr.artist.toLowerCase().includes(q) || (tr.album ?? "").toLowerCase().includes(q);
  function openSearch() {
    searchOpen = true;
    tick().then(() => searchInput?.focus());
  }
  function clearSearch(close = true) {
    filterQ = "";
    if (close) searchOpen = false;
  }

  // Recently played mixes songs and artists; the rows follow that order.
  const recent = $derived.by(() => {
    if (l?.kind !== "recent") return null;
    const songs: Track[] = [];
    const rows: ({ kind: "song"; at: number } | { kind: "artist"; name: string; track: Track | null })[] = [];
    for (const it of app.library.recent) {
      if (it.kind === "song") { rows.push({ kind: "song", at: songs.length }); songs.push(it.track); }
      else if (it.kind === "artist") rows.push({ kind: "artist", name: it.name, track: it.track ?? null });
    }
    return { songs, rows };
  });
  const recentRows = $derived.by(() => {
    if (!recent) return [];
    if (!q) return recent.rows;
    return recent.rows.filter((r) => (r.kind === "artist" ? r.name.toLowerCase().includes(q) : matchTrack(recent.songs[r.at])));
  });
  const filteredTracks = $derived.by(() => {
    const out: { track: Track; idx: number }[] = [];
    for (let i = 0; i < tracks.length; i++) {
      if (matchTrack(tracks[i])) out.push({ track: tracks[i], idx: i });
    }
    return out;
  });

  const albumLoading = $derived(l?.kind === "album" && !!l.loading && !has);
  const empty = $derived(!albumLoading && (l?.kind === "recent" ? !app.library.recent.length : !has));
  const filterEmpty = $derived(!empty && !!q && (recent ? !recentRows.length : !filteredTracks.length));
  const emptyText = $derived.by(() => {
    switch (l?.kind) {
      case "liked": return [t("library.liked.empty"), t("library.liked.empty.sub")];
      case "recent": return [t("home.empty.title"), t("library.recent.empty.sub")];
      case "downloads": return [t("library.downloads.empty"), t("library.downloads.empty.sub")];
      case "playlist": return [t("library.playlist.empty"), t("list.empty.sub.user")];
      default: return [t("library.playlist.empty"), t("list.empty.sub")];
    }
  });

  // ---- sticky compact bar + column header on scroll ----
  let actsEl: HTMLDivElement | undefined = $state();
  let colHeadEl: HTMLDivElement | undefined = $state();
  let stuck = $state(false);
  let stuckCols = $state(false);

  function syncScroll() {
    const sc = document.getElementById("main-scroll");
    if (!sc || !actsEl || !has) { stuck = false; stuckCols = false; return; }
    const st = sc.getBoundingClientRect().top;
    const nextStuck = actsEl.getBoundingClientRect().bottom <= st + 6;
    stuck = nextStuck;
    stuckCols = nextStuck && !recent && !!colHeadEl && colHeadEl.getBoundingClientRect().top <= st + PIN_H;
  }

  $effect(() => {
    const sc = document.getElementById("main-scroll");
    if (!sc) return;
    sc.addEventListener("scroll", syncScroll, { passive: true });
    addEventListener("resize", syncScroll);
    return () => {
      sc.removeEventListener("scroll", syncScroll);
      removeEventListener("resize", syncScroll);
      pageChrome.stickyTop = false;
    };
  });
  $effect(() => {
    void [tracks.length, q, lvW, recent?.rows.length];
    tick().then(syncScroll);
  });
  $effect(() => { pageChrome.stickyTop = stuck; });

  // ---- drag to reorder (own playlists, own order, unfiltered) ----
  const reorder = $derived(l?.kind === "playlist" && sort === "custom" && !q ? l.id : null);
  let listEl: HTMLDivElement | undefined = $state();
  let drag = $state<{ from: number; slot: number; x: number; y: number; dy: number; track: Track } | null>(null);
  let settle = $state(false);
  function shift(i: number): number {
    if (!drag || i === drag.from) return 0;
    const { from, slot } = drag;
    if (from < slot && i > from && i <= slot) return -PITCH;
    if (slot < from && i >= slot && i < from) return PITCH;
    return 0;
  }
  function slotAt(y: number): number {
    const top = listEl?.getBoundingClientRect().top ?? 0;
    return Math.max(0, Math.min(tracks.length - 1, Math.floor((y - top) / PITCH)));
  }
  function startDrag(i: number, e: PointerEvent) {
    if (!reorder) return;
    const x0 = e.clientX, y0 = e.clientY;
    const rowTop = (e.currentTarget as HTMLElement).getBoundingClientRect().top;
    let live = false;
    const move = (ev: PointerEvent) => {
      if (!live && Math.hypot(ev.clientX - x0, ev.clientY - y0) < 5) return;
      if (!live) { live = true; app.ctx = null; }
      drag = { from: i, slot: slotAt(ev.clientY), x: ev.clientX, y: ev.clientY, dy: y0 - rowTop, track: tracks[i] };
    };
    const up = (ev: PointerEvent) => {
      removeEventListener("pointermove", move);
      removeEventListener("pointerup", up);
      removeEventListener("pointercancel", up);
      if (!live || !drag) return;
      // The click that follows the drag must not play the row.
      const eat = (c: MouseEvent) => { c.stopPropagation(); c.preventDefault(); };
      addEventListener("click", eat, { capture: true, once: true });
      setTimeout(() => removeEventListener("click", eat, true), 0);
      const r = listEl?.getBoundingClientRect();
      const inside = ev.type === "pointerup" && !!r && ev.clientX >= r.left && ev.clientX <= r.right && ev.clientY >= r.top - PITCH && ev.clientY <= r.bottom + PITCH;
      const { from, slot } = drag;
      settle = true;
      drag = null;
      if (inside && from !== slot && reorder) app.movePlaylistItem(reorder, from, slot);
      requestAnimationFrame(() => requestAnimationFrame(() => (settle = false)));
    };
    addEventListener("pointermove", move);
    addEventListener("pointerup", up);
    addEventListener("pointercancel", up);
  }
</script>

{#snippet sortArrow(d: "asc" | "desc")}
  <svg class="sarr" width="9" height="6" viewBox="0 0 9 6" aria-hidden="true">
    {#if d === "asc"}<polygon points="4.5,0.5 8.5,5.5 0.5,5.5" />{:else}<polygon points="0.5,0.5 8.5,0.5 4.5,5.5" />{/if}
  </svg>
{/snippet}

{#snippet colHead(pinned: boolean)}
  {@const sortable = !!sortId(l)}
  {@const titleActive = sort === "title" || sort === "artist"}
  <div class="colhead" class:pinned class:hidden={!pinned && stuckCols}>
    {#if sortable}
      <button type="button" class="ch ch-num" onclick={() => setListSort(l, "custom", "asc")}>#</button>
      <button type="button" class="ch ch-title" class:on={titleActive} onclick={cycleTitleSort}>
        <span>{sort === "artist" ? t("sort.by_artist") : t("sort.by_title")}</span>
        {#if titleActive}{@render sortArrow(dir)}{/if}
      </button>
      {#if showAlbumCol}
        <button type="button" class="ch ch-al" class:on={sort === "album"} onclick={() => toggleColSort("album")}>
          <span>{t("sort.by_album")}</span>
          {#if sort === "album"}{@render sortArrow(dir)}{/if}
        </button>
      {/if}
      <span class="ch-like"></span>
      <button type="button" class="ch ch-du" class:on={sort === "duration"} aria-label={t("sort.by_length")} onclick={() => toggleColSort("duration")}>
        <Icon name="clock" size={14} />
        {#if sort === "duration"}{@render sortArrow(dir)}{/if}
      </button>
    {:else}
      <span class="ch ch-num">#</span>
      <span class="ch ch-title"><span>{t("sort.by_title")}</span></span>
      {#if showAlbumCol}<span class="ch ch-al"><span>{t("sort.by_album")}</span></span>{/if}
      <span class="ch-like"></span>
      <span class="ch ch-du"><Icon name="clock" size={14} /></span>
    {/if}
    <span class="ch-more"></span>
  </div>
{/snippet}

{#snippet recsSection()}
  {#if own && !q}
    <section class="recs" aria-label={t("playlist.recs.title")}>
      <div class="recs-head">
        <div class="recs-ht">
          <h2 class="recs-title">{t("playlist.recs.title")}</h2>
          <p class="recs-sub">{t("playlist.recs.sub")}</p>
        </div>
        <div class="recs-acts">
          <Button kind={adding ? "primary" : "default"} icon="circle-plus" onclick={addSongs}>{t("add.title")}</Button>
          <IconButton name="refresh-cw" size={15} box={32} label={t("plugins.refresh")} onclick={refreshRecs} />
        </div>
      </div>
      {#if recsLoading && !recTracks.length}
        <div class="recs-sk" aria-busy="true">
          {#each Array(4) as _, i}<div class="rec-sk" style:opacity={0.72 - i * 0.12}></div>{/each}
        </div>
      {:else if recTracks.length}
        <div class="recs-list">
          {#each recTracks as tr, i (`${trackKey(tr)}:${i}`)}
            <div class="rec-row" role="row" tabindex="-1" onclick={() => app.play(recTracks, i)} onkeydown={(e) => e.key === "Enter" && app.play(recTracks, i)}>
              <Cover track={tr} size={40} radius={6} />
              <span class="rec-tx">
                <span class="rec-tl"><SongTitle rating={rating(tr)}>{tr.title}</SongTitle></span>
                <span class="rec-ar">
                  <button class="link trunc" onclick={(e) => { e.stopPropagation(); app.openArtistPage(tr.artist); }}>{tr.artist}</button>
                </span>
              </span>
              {#if showAlbumCol}
                <span class="rec-al trunc">
                  {#if tr.album}<button class="link trunc" onclick={(e) => { e.stopPropagation(); app.openAlbum(tr.album!, tr.artist, [], tr); }}>{tr.album}</button>{/if}
                </span>
              {/if}
              <button type="button" class="rec-add" onclick={(e) => { e.stopPropagation(); addRec(tr); }}>{t("playlist.recs.add")}</button>
            </div>
          {/each}
        </div>
      {/if}
    </section>
  {/if}
{/snippet}

{#snippet listBody()}
  {#if albumLoading}
    <div class="rows">
      <Skeletons kind="rows" n={8} prefix="album" />
    </div>
  {:else if empty}
    <div class="body-pad">
      <EmptyState title={emptyText[0]} sub={emptyText[1]}>
        {#if own}
          <Select value={null as "link" | "file" | null} label={t("ctx.more")} options={importOptions} onpick={pickImport} />
        {/if}
      </EmptyState>
    </div>
    {@render recsSection()}
  {:else if filterEmpty}
    <div class="body-pad">
      <EmptyState icon="search" title={t("search.none.title")} sub={t("local.none.sub")}>
        <Button onclick={() => clearSearch(true)}>{t("lyrics.timing.reset")}</Button>
      </EmptyState>
    </div>
  {:else if recent}
    <div class="rows">
      {#each recentRows as r, i (i)}
        {#if r.kind === "artist"}<RecentArtistRow name={r.name} track={r.track} />
        {:else}<SongRow tracks={recent.songs} index={r.at} from={l} context={title} selected={app.selectedRow === r.at} />{/if}
      {/each}
    </div>
  {:else}
    <div class="colwrap" bind:this={colHeadEl}>
      {@render colHead(false)}
    </div>
    <div class="rows" bind:this={listEl} class:settle class:dragging={!!drag}>
      <VirtualRows count={filteredTracks.length} pitch={PITCH}>
        {#snippet row(i)}
          {@const entry = filteredTracks[i]}
          <div class="slot">
            <div class="mover" class:lifted={drag?.from === entry.idx} style:transform="translateY({shift(entry.idx)}px)">
              <SongRow {tracks} index={entry.idx} number={entry.idx + 1} from={l} album={l.kind !== "album" || !!author} context={title} selected={app.selectedRow === entry.idx}
                draggable={!!reorder} ondragstart={(e) => startDrag(entry.idx, e)} />
            </div>
          </div>
        {/snippet}
      </VirtualRows>
    </div>
    {@render recsSection()}
  {/if}
{/snippet}

{#if l?.kind === "artist_liked"}
  <LikedByArtist artist={l.artist} />
{:else if l}
  <div class="lv" class:narrow bind:clientWidth={lvW}>
    {#if stuck}
      <div class="pin-anchor">
        <div class="pin-bar" style:background={pinBg}>
          <div class="pin-top">
            {#if has}<RoundPlay size={36} playing={here && app.playing} loading={here && app.loading} onclick={playAll} />{/if}
            <span class="pin-title trunc">{title}</span>
            <span class="flex1"></span>
            {#if q}
              <button class="pin-chip" onclick={() => clearSearch(true)}>
                <span class="trunc">{filterQ}</span>
                <Icon name="x" size={12} />
              </button>
            {/if}
            <IconButton name="ellipsis-vertical" size={16} box={34} label={t("ctx.more")} onpointerdown={more} />
          </div>
          {#if stuckCols}
            <div class="pin-cols">{@render colHead(true)}</div>
          {/if}
        </div>
      </div>
    {/if}

    <header class="hero" class:no-cover={noCover} style:background={heroWash}>
      <div class="hero-main" class:narrow>
        {#if !noCover}
          {#if own}
            <button class="art edit" style:width="{coverSize}px" style:height="{coverSize}px" onclick={() => openNameDialog([], own.id)} aria-label={t("list.edit")}>
              <ListCover list={l} size={coverSize} />
              <span class="veil"><Icon name="pencil" size={26} color="rgba(255,255,255,.95)" /><span>{t("list.edit")}</span></span>
            </button>
          {:else}
            <div class="art" style:width="{coverSize}px" style:height="{coverSize}px"><ListCover list={l} size={coverSize} /></div>
          {/if}
        {/if}

        <div class="info">
          <span class="kind" use:ink>{kind}</span>
          <div class="trow">
            <h1 class="title" style:font-size={heroTitleSize} use:ink>{title}</h1>
            {#if author}<button class="author" onclick={() => app.openProfilePage(author.slice(1))}>{author}</button>{/if}
          </div>
          <div class="sub">
            {#if l.kind === "album" && !author}
              <button class="link trunc" onclick={() => app.openArtistPage(l.artist)}>{l.artist}</button>
              {#if !albumLoading}<span class="sep">·</span>{/if}
            {:else if own?.shared_by}
              <span>{tf("list.shared_by", { handle: own.shared_by })}</span>
              <span class="sep">·</span>
            {/if}
            {#if !albumLoading}
              <span class="meta" use:ink>{listMeta(tracks.length, total, t("search.kind.song"), tf("common.n_songs", { n: tracks.length }))}</span>
            {/if}
          </div>
        </div>
      </div>

      <div class="acts" bind:this={actsEl}>
        <div class="acts-l">
          {#if has}
            <RoundPlay size={48} playing={here && app.playing} loading={here && app.loading} onclick={playAll} />
            <IconButton name="shuffle" size={19} box={38} label={t("tip.shuffle")} onclick={() => playShuffled(tracks, title)} />
          {/if}
          {#if !onWeb() && has && l.kind !== "downloads"}
            {#if dl.state === "busy"}
              <span class="dlb" use:tip={downloadTip(tracks)}><DownloadBadge state="busy" progress={dl.progress} size={22} arrow="download" /></span>
            {:else if dl.state === "done"}
              <button type="button" class="dlb act-btn" use:tip={t("ctx.remove_downloads")} aria-label={t("ctx.remove_downloads")} onclick={() => removeDownloads(tracks)}>
                <DownloadBadge state="done" size={20} />
              </button>
            {:else}
              <IconButton name="arrow-down-to-line" size={18} box={38} label={downloadTip(tracks)} onclick={() => app.download(tracks)} />
            {/if}
          {:else if !onWeb() && has && dl.state === "busy"}
            <span class="dlb" use:tip={downloadTip(tracks)}><DownloadBadge state="busy" progress={dl.progress} size={22} arrow="download" /></span>
          {/if}
          {#if app.sharedPreview}
            <Button kind="primary" onclick={addShared}>{t("list.add_shared")}</Button>
          {/if}
          {#if saveable}
            <ToggleButton name="heart" size={18} on={app.isSavedAlbum(l)} accent="var(--accent)" label={t(app.isSavedAlbum(l) ? "ctx.unsave" : "ctx.save")} onclick={() => app.toggleSaveAlbum(l)} />
          {/if}
          {#if own}
            <IconButton name="pencil" size={17} box={36} label={t("list.edit")} onclick={() => openNameDialog([], own.id)} />
          {/if}
          <IconButton name="ellipsis-vertical" size={18} box={38} label={t("ctx.more")} onpointerdown={more} />
        </div>

        <div class="acts-r">
          {#if has}
            {#if searchOpen || filterQ}
              <div class="qbox" class:narrow>
                <Icon name="search" size={14} color="var(--dim)" />
                <input
                  bind:this={searchInput}
                  bind:value={filterQ}
                  placeholder={t("add.search")}
                  spellcheck="false"
                  onkeydown={(e) => {
                    if (e.key === "Escape") { e.stopPropagation(); clearSearch(true); }
                  }}
                />
                <button type="button" class="qclr" aria-label={t("tip.x")} onclick={() => clearSearch(true)}>
                  <Icon name="x" size={13} color="var(--dim)" />
                </button>
              </div>
            {:else}
              <IconButton name="search" size={17} box={36} label={t("add.search")} onclick={openSearch} />
            {/if}
          {/if}
          {#if sortId(l) && has}
            <button type="button" class="sort-btn" class:on={sort !== "custom"} use:tip={t("tip.arrow-up-down")} onpointerdown={sortMenu}>
              {#if lvW >= 440}<span class="trunc">{sortLabel}</span>{/if}
              {#if sort !== "custom"}{@render sortArrow(dir)}{/if}
              <Icon name="arrow-up-down" size={15} />
            </button>
          {/if}
        </div>
      </div>
    </header>

    {@render listBody()}
  </div>

  {#if drag}
    <!-- lists.rs SongDrag: w 420, h 56, px 10, gap 14, radius 8, hsla(0,0,.16), 1 px white 10 %. -->
    <div class="dragview" style:left="{drag.x - 24}px" style:top="{drag.y - drag.dy}px">
      <Icon name="grip-vertical" size={14} color="var(--text)" />
      <Cover track={drag.track} size={40} radius={6} />
      <span class="dt">
        <span class="dtt"><SongTitle rating={rating(drag.track)}>{drag.track.title}</SongTitle></span>
        <span class="dta trunc">{drag.track.artist}</span>
      </span>
    </div>
  {/if}
{/if}

<style>
  .lv { display: flex; flex-direction: column; min-width: 0; padding-bottom: 20px; }

  /* Sticky compact bar on scroll (height 0 anchor avoids any flow shift). */
  .pin-anchor { position: sticky; top: 0; z-index: 20; height: 0; overflow: visible; margin: 0 calc(-1 * min(24px, var(--page-x))); }
  .pin-bar { border-radius: 14px; overflow: hidden; box-shadow: 0 12px 30px rgba(0, 0, 0, .52); animation: fade-in .14s ease-out; }
  .pin-top { height: 56px; padding: 0 min(24px, var(--page-x)); display: flex; align-items: center; gap: 12px; min-width: 0; }
  .pin-title { font-size: calc(18px * var(--k)); font-weight: 700; color: var(--text); min-width: 0; }
  .pin-chip { display: inline-flex; align-items: center; gap: 6px; max-width: 160px; height: 26px; padding: 0 9px; border-radius: 99px; background: #202024; font-size: var(--xs); color: var(--text); }
  .pin-chip:hover { background: #2c2c32; }
  .pin-cols { padding: 0 min(24px, var(--page-x)); border-top: 1px solid rgba(255, 255, 255, .08); }

  /* Top Spotify-style hero */
  .hero { margin: -12px -24px 0; padding: 20px 24px 16px; border-radius: 14px 14px 0 0; display: flex; flex-direction: column; gap: 18px; }
  .hero.no-cover { padding-bottom: 14px; }
  .hero-main { display: flex; align-items: flex-end; gap: 22px; min-width: 0; }
  .hero-main.narrow { flex-direction: column; align-items: flex-start; gap: 14px; }
  .art { position: relative; flex: none; display: block; border-radius: 12px; box-shadow: 0 12px 32px rgba(0, 0, 0, .42); }
  .veil { position: absolute; inset: 0; border-radius: 12px; background: rgba(0, 0, 0, .55); display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; font-size: var(--sm); color: rgba(255, 255, 255, .95); opacity: 0; transition: opacity .15s ease-out; }
  .edit:hover .veil, .edit:focus-visible .veil { opacity: 1; }
  .info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; padding-bottom: 2px; }
  .kind { font-size: var(--sm); font-weight: 500; line-height: 18px; color: var(--dim); }
  .trow { display: flex; flex-wrap: wrap; align-items: baseline; column-gap: 10px; min-width: 0; }
  .title { margin: 0; font-weight: 700; line-height: 1.12; letter-spacing: -.02em; color: var(--text); overflow: hidden; display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow-wrap: anywhere; }
  .author { font-size: var(--sm); color: var(--faint); transition: color .12s; }
  .author:hover { color: var(--text); }
  .sub { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; font-size: var(--sm); color: var(--dim); min-width: 0; }
  .sep { color: var(--faint); }
  .link { max-width: 100%; font-weight: 500; color: var(--text); }
  .link:hover { text-decoration: underline; }
  .meta { font-size: var(--sm); color: var(--faint); }

  /* Unified options & filter/sort bar */
  .acts { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 12px; padding-top: 2px; min-width: 0; }
  .acts-l { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; min-width: 0; }
  .acts-r { display: flex; align-items: center; gap: 8px; margin-left: auto; min-width: 0; }
  .dlb { flex: none; width: 38px; height: 38px; display: grid; place-items: center; border-radius: 8px; transition: background-color .12s ease-out; }
  .act-btn:hover { background: var(--hover); }
  .act-btn:active { background: var(--active); }
  .qbox { display: flex; align-items: center; gap: 8px; width: 204px; height: 32px; padding: 0 8px 0 10px; border-radius: 8px; background: #1a1a1d; transition: background-color .14s; }
  .qbox.narrow { width: 156px; }
  .qbox:focus-within { background: #242428; }
  .qbox input { flex: 1; min-width: 0; background: none; border: 0; padding: 0; outline: none; font-size: var(--sm); color: var(--text); }
  .qbox input::placeholder { color: var(--faint); }
  .qclr { flex: none; width: 22px; height: 22px; border-radius: 5px; display: grid; place-items: center; transition: background-color .12s; }
  .qclr:hover { background: rgba(255, 255, 255, .1); }
  .sort-btn { display: inline-flex; align-items: center; gap: 7px; max-width: 220px; height: 32px; padding: 0 10px; border-radius: 8px; font-size: var(--sm); color: var(--dim); transition: background-color .12s ease-out, color .12s; }
  .sort-btn:hover { background: var(--hover); color: var(--text); }
  .sort-btn.on { color: var(--accent); background: rgba(var(--accent-rgb), .12); }

  /* Column headers aligned pixel-for-pixel with SongRow */
  .colwrap { margin-top: 6px; }
  .colhead { display: flex; align-items: center; gap: 14px; height: 34px; padding: 0 10px; border-bottom: 1px solid rgba(255, 255, 255, .08); font-size: 12px; font-weight: 500; color: var(--faint); }
  .colhead.pinned { border-bottom: 0; }
  .colhead.hidden { visibility: hidden; }
  .ch { display: inline-flex; align-items: center; gap: 6px; text-align: left; color: inherit; transition: color .12s; white-space: nowrap; }
  button.ch:hover { color: var(--text); }
  button.ch.on { color: var(--text); }
  .sarr { flex: none; fill: var(--accent); }
  .ch-num { flex: none; width: 26px; justify-content: center; text-align: center; }
  .ch-title { flex: 1; min-width: 214px; }
  .ch-al { flex: 0 1 200px; width: 200px; min-width: 0; }
  .ch-like { flex: none; width: 29px; }
  .ch-du { flex: none; width: 40px; padding-left: 6px; gap: 4px; }
  .ch-more { flex: none; width: 28px; }

  .body-pad { padding-top: 12px; }
  .rows { display: flex; flex-direction: column; padding-top: 4px; }

  /* Bottom Recommended section for own playlists */
  .recs { margin-top: 28px; padding-top: 20px; border-top: 1px solid rgba(255, 255, 255, .08); display: flex; flex-direction: column; gap: 12px; }
  .recs-head { display: flex; align-items: flex-end; justify-content: space-between; flex-wrap: wrap; gap: 12px; padding: 0 10px; }
  .recs-ht { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .recs-title { margin: 0; font-size: calc(18px * var(--k)); font-weight: 700; color: var(--text); }
  .recs-sub { margin: 0; font-size: var(--sm); color: var(--dim); }
  .recs-acts { display: flex; align-items: center; gap: 8px; flex: none; }
  .recs-sk { display: flex; flex-direction: column; gap: 6px; }
  .rec-sk { height: 56px; border-radius: 8px; background: rgba(255, 255, 255, .04); }
  .recs-list { display: flex; flex-direction: column; }
  .rec-row { display: flex; align-items: center; gap: 14px; height: 56px; padding: 0 10px; border-radius: 8px; cursor: pointer; transition: background-color .12s ease-out; }
  .rec-row:hover { background: rgba(255, 255, 255, .05); }
  .rec-tx { flex: 1; min-width: 160px; display: flex; flex-direction: column; gap: 2px; }
  .rec-tl { font-size: var(--md); color: var(--text); min-width: 0; }
  .rec-ar { display: flex; align-items: center; min-width: 0; font-size: var(--sm); color: var(--dim); }
  .rec-al { flex: 0 1 200px; width: 200px; min-width: 0; display: flex; font-size: var(--sm); color: var(--faint); }
  .rec-add { flex: none; height: 30px; padding: 0 14px; border-radius: 99px; background: #222226; font-size: var(--xs); font-weight: 600; color: var(--text); transition: background-color .12s ease-out, transform .12s ease-out; }
  .rec-add:hover { background: #303036; }
  .rec-add:active { transform: scale(.96); }
  .slot { position: relative; height: 56px; content-visibility: auto; contain-intrinsic-size: auto 56px; }
  .mover { position: absolute; left: 0; right: 0; top: 0; transition: transform .18s var(--ease); }
  .settle .mover { transition: none; }
  .mover.lifted { opacity: 0; }
  .dragging { cursor: grabbing; }
  .dragview { position: fixed; z-index: 60; width: 420px; height: 56px; padding: 0 10px; display: flex; align-items: center; gap: 14px; border-radius: 8px; background: hsl(0 0% 16%); border: 1px solid rgba(255, 255, 255, .1); color: var(--text); cursor: grabbing; pointer-events: none; animation: fade-in .12s ease-out; }
  .dt { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .dtt { font-size: var(--md); min-width: 0; }
  .dta { font-size: var(--sm); color: var(--dim); }
</style>
