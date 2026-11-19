<script lang="ts">
  // ui/local.rs view() + body() + crumbs(): "Local files", the music folders
  // on this computer by folder, artist, album or as one list of songs. Files
  // play where they are; nothing is copied or uploaded.
  //  page: gap 16; back pill; head (round play 44 + heading + info line
  //  mono 11.5 faint; bordered actions: shuffle / rescan / add folder, h 32,
  //  px 14, radius 8); tabs (chips: folders / artists / albums / songs) with
  //  a 240 x 32 filter field; crumbs ("Folders > Music > Album", the last
  //  one semibold); body: empty state with "add folder" (no folders), the
  //  folder rows (each with a remove x), subfolders + songs, artists,
  //  albums (cover 40) or songs (rows 56, virtualised).
  import { onMount } from "svelte";
  import { backend } from "../api/backend";
  import { app } from "../state.svelte";
  import { t, tf } from "../i18n.svelte";
  import { ink } from "../ui.svelte";
  import Button from "../components/Button.svelte";
  import Chip from "../components/Chip.svelte";
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import Cover from "../components/Cover.svelte";
  import EmptyState from "../components/EmptyState.svelte";
  import PlayGlyph from "../components/PlayGlyph.svelte";
  import SectionTitle from "../components/SectionTitle.svelte";
  import SongRow from "../components/SongRow.svelte";
  import { tip } from "../ui.svelte";
  import EntryRow from "./local/EntryRow.svelte";
  import VirtualRows from "../components/VirtualRows.svelte";
  import {
    addFolder, displayName, emptyPage, local, LOCAL_ALBUMS, LOCAL_ARTISTS, LOCAL_FOLDERS, LOCAL_SONGS, openAlbum, openArtist, openDir, openLocal, pageArgs, playLocal,
    removeFolder, rescan, roots, setTab, up, type LocalPage,
  } from "./local/local.svelte";

  let page = $state<LocalPage>(emptyPage());
  let loaded = $state(false);
  let tick = $state(0);
  let query = $state(local.query);
  let lastLocalQuery = local.query;
  const rs = $derived(roots());
  const filtered = $derived(local.query.trim() !== "");

  onMount(() => {
    openLocal();
    // While a scan runs the page asks again for the progress.
    const id = setInterval(() => { if (local.scanning) tick++; }, 700);
    return () => clearInterval(id);
  });

  $effect(() => {
    void [local.tab, local.dir, local.artist, local.album, local.query, local.gen, local.scanning, tick, JSON.stringify(rs)];
    let alive = true;
    backend().call<LocalPage>("local_page", pageArgs()).then((p) => { if (alive && p) { page = p; loaded = true; } });
    return () => { alive = false; };
  });

  // Keep the input field aligned when URL navigation updates `local.query`.
  $effect(() => {
    const lq = local.query;
    if (lq !== lastLocalQuery) { lastLocalQuery = lq; query = lq; }
  });

  // The filter is applied a moment after typing.
  $effect(() => {
    const q = query;
    const id = setTimeout(() => { lastLocalQuery = q; local.query = q; }, 150);
    return () => clearTimeout(id);
  });

  // Back to the top when the tab, folder, artist or album changes.
  $effect(() => {
    void [local.tab, local.dir, local.artist, local.album];
    document.getElementById("main-scroll")?.scrollTo(0, 0);
  });

  const hasSongs = $derived(page.total > 0);
  const here = $derived(!!app.player.current?.id.startsWith("file:"));
  const nSongs = (n: number) => tf("common.n_songs", { n });
  const info = $derived(
    page.progress ? tf("local.scanning", { done: page.progress[0], total: page.progress[1] })
      : local.scanning ? t("local.scanning_start")
        : tf("local.info", { n: page.total, folders: rs.length }),
  );

  const TABS: [number, string][] = [[LOCAL_FOLDERS, "local.tab.folders"], [LOCAL_ARTISTS, "local.tab.artists"], [LOCAL_ALBUMS, "local.tab.albums"], [LOCAL_SONGS, "local.tab.songs"]];

  /** "Folders > Music > Album" (folders tab), "Artists > Name", "Albums > Title". */
  const crumbs = $derived.by((): { label: string; go: (() => void) | null }[] | null => {
    if (local.tab === LOCAL_FOLDERS) {
      const dir = local.dir;
      if (!dir) return null;
      const out: { label: string; go: (() => void) | null }[] = [{ label: t("local.tab.folders"), go: () => openDir(null) }];
      const root = rs.find((r) => dir.startsWith(r));
      if (root) {
        let cur = root;
        out.push({ label: displayName(cur), go: () => openDir(root) });
        for (const part of dir.slice(root.length).split(/[\\/]/).filter(Boolean)) {
          cur = `${cur.replace(/[\\/]+$/, "")}/${part}`;
          const target = cur;
          out.push({ label: part, go: () => openDir(target) });
        }
      }
      return out;
    }
    if (local.tab === LOCAL_ARTISTS && local.artist) return [{ label: t("local.tab.artists"), go: () => openArtist(null) }, { label: local.artist, go: null }];
    if (local.tab === LOCAL_ALBUMS && local.album) return [{ label: t("local.tab.albums"), go: () => openAlbum(null) }, { label: `${local.album[0]} · ${local.album[1]}`, go: null }];
    return null;
  });

  function playPause() { if (here) app.togglePlay(); else playLocal(false); }
</script>

<div class="pg">
  {#if app.history.length}<div class="bk"><Button kind="back" icon="arrow-left" onclick={() => app.back()}>{t("common.back")}</Button></div>{/if}

  <div class="head">
    <div class="hl">
      {#if hasSongs}
        <button class="play" onclick={playPause} use:tip={t(here && app.playing ? "common.pause" : "common.play")}>
          <span class:nudge={!(here && app.playing) && !(here && app.loading)}><PlayGlyph loading={here && app.loading} playing={here && app.playing} size={18} button={44} /></span>
        </button>
      {/if}
      <div class="tt">
        <SectionTitle heading title={t("local.title")} />
        <div class="info" use:ink>{info}</div>
      </div>
    </div>
    <div class="acts">
      {#if hasSongs}<button class="act" onclick={() => playLocal(true)}><Icon name="shuffle" size={14} color="rgba(255,255,255,.9)" />{t("local.shuffle")}</button>{/if}
      {#if rs.length}<button class="act" class:busy={local.scanning} onclick={rescan}><Icon name="refresh-cw" size={14} color="rgba(255,255,255,.9)" />{t("local.rescan")}</button>{/if}
      <button class="act" onclick={addFolder}><Icon name="folder-plus" size={14} color="rgba(255,255,255,.9)" />{t("local.add")}</button>
    </div>
  </div>

  <div class="tabs">
    <div class="chips">
      {#each TABS as [k, key] (k)}<Chip on={local.tab === k} onclick={() => setTab(k)}>{t(key)}</Chip>{/each}
    </div>
    {#if hasSongs}
      <label class="field">
        <Icon name="search" size={14} color="rgba(255,255,255,.5)" />
        <input bind:value={query} placeholder={t("local.search")} spellcheck="false" />
      </label>
    {/if}
  </div>

  {#if crumbs}
    <div class="crumbs">
      <IconButton name="arrow-left" size={14} color="var(--dim)" label={t("common.back")} onclick={() => up()} />
      {#each crumbs as c, i (i)}
        <span class="crumb">
          {#if i > 0}<Icon name="chevron-right" size={13} color="var(--faint)" />{/if}
          {#if c.go && i + 1 < crumbs.length}
            <button class="cl" onclick={c.go}>{c.label}</button>
          {:else}
            <span class="cl last">{c.label}</span>
          {/if}
        </span>
      {/each}
    </div>
  {/if}

  <div class="body">
    {#if !rs.length}
      <div class="empty">
        <EmptyState icon="music-2" title={t("local.empty.title")} sub={t("local.empty.sub")} />
        <Button kind="primary" onclick={addFolder}>{t("local.add")}</Button>
      </div>
    {:else if !loaded}
      <div class="load" use:ink>{t("local.scanning_start")}</div>
    {:else if local.tab === LOCAL_FOLDERS && local.dir == null}
      <!-- The music folders themselves; each can be removed again. -->
      <div class="list">
        {#each page.dirs as d, i (d.path)}
          <EntryRow icon="folder" name={d.name || displayName(d.path)} sub="{nSongs(d.count)} · {d.path}" delay={Math.min(i, 8) * 30} onclick={() => openDir(d.path)}>
            {#snippet extra()}
              <span use:tip={t("local.remove")} class="rm"><IconButton name="x" size={14} color="var(--dim)" notip label={t("local.remove")} onclick={(e) => { e.stopPropagation(); removeFolder(d.path); }} /></span>
            {/snippet}
          </EntryRow>
        {/each}
        <div class="note" use:ink>{t("local.note")}</div>
      </div>
    {:else if local.tab === LOCAL_FOLDERS}
      {#if !page.dirs.length && !page.tracks.length}
        {#if filtered}<EmptyState icon="music-2" title={t("local.none.title")} sub={t("local.none.sub")} />
        {:else}<EmptyState icon="music-2" title={t("local.folder_empty.title")} sub={t("local.folder_empty.sub")} />{/if}
      {:else}
        <div class="list">
          {#each page.dirs as d, i (d.path)}
            <EntryRow icon="folder" name={d.name || displayName(d.path)} sub={nSongs(d.count)} delay={Math.min(i, 8) * 30} onclick={() => openDir(d.path)} />
          {/each}
        </div>
        <VirtualRows count={page.tracks.length}>
          {#snippet row(i)}<SongRow tracks={page.tracks} index={i} context={t("local.title")} />{/snippet}
        </VirtualRows>
      {/if}
    {:else if local.tab === LOCAL_ARTISTS && local.artist == null}
      {#if !page.artists.length}
        <EmptyState icon="music-2" title={t("local.none.title")} sub={t("local.none.sub")} />
      {:else}
        <VirtualRows count={page.artists.length}>
          {#snippet row(i)}<EntryRow icon="user-round" name={page.artists[i].name} sub={nSongs(page.artists[i].count)} onclick={() => openArtist(page.artists[i].name)} />{/snippet}
        </VirtualRows>
      {/if}
    {:else if local.tab === LOCAL_ALBUMS && local.album == null}
      {#if !page.albums.length}
        <EmptyState icon="music-2" title={t("local.none.title")} sub={t("local.none.sub")} />
      {:else}
        <VirtualRows count={page.albums.length}>
          {#snippet row(i)}
            {@const a = page.albums[i]}
            <EntryRow name={a.title} sub="{a.artist} · {nSongs(a.count)}" onclick={() => openAlbum([a.title, a.artist])}>
              {#snippet art()}<Cover track={a.cover} size={40} radius={6} />{/snippet}
            </EntryRow>
          {/snippet}
        </VirtualRows>
      {/if}
    {:else if !page.tracks.length}
      {#if filtered || local.scanning}<EmptyState icon="music-2" title={t("local.none.title")} sub={t("local.none.sub")} />
      {:else}<EmptyState icon="music-2" title={t("local.no_songs.title")} sub={t("local.no_songs.sub")} />{/if}
    {:else}
      <VirtualRows count={page.tracks.length}>
        {#snippet row(i)}<SongRow tracks={page.tracks} index={i} context={t("local.title")} />{/snippet}
      </VirtualRows>
    {/if}
  </div>
</div>

<style>
  .pg { display: flex; flex-direction: column; gap: 16px; }
  .bk { display: flex; }
  .head { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 12px; }
  .hl { display: flex; align-items: center; gap: 14px; }
  .tt { display: flex; flex-direction: column; gap: 4px; }
  .info { font-size: var(--sm); color: var(--dim); }
  /* round_play: 44, white, hover .1 black fill */
  .play { flex: none; width: 44px; height: 44px; border-radius: 50%; background: #fff; display: grid; place-items: center; position: relative; transition: transform .15s var(--ease); }
  .play::after { content: ""; position: absolute; inset: 0; border-radius: 50%; background: #000; opacity: 0; transition: opacity .12s ease-out; }
  .play:hover::after { opacity: .1; }
  .play:active { transform: scale(.94); }
  .nudge { padding-left: 2px; display: grid; }
  .acts { display: flex; gap: 8px; }
  /* action: h 32, px 14, radius 8, 1 px border, solid dark surface, sm, white 90 % */
  .act { display: flex; align-items: center; gap: 6px; height: 32px; padding: 0 14px; border-radius: 8px; background: #222226; font-size: var(--sm); color: rgba(255, 255, 255, .9); transition: background-color .12s ease-out, opacity .15s; }
  .act:hover { background: #242428; }
  .act.busy { opacity: .5; }
  .tabs { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 10px; }
  .chips { display: flex; gap: 8px; }
  .field { display: flex; align-items: center; gap: 8px; width: 240px; height: 32px; padding: 0 10px; border-radius: 8px; background: #1c1c1f; font-size: var(--sm); transition: border-color .15s; }
  .field:focus-within { border-color: rgba(255, 255, 255, .2); }
  .field input { flex: 1; min-width: 0; background: none; border: 0; padding: 0; outline: none; font-size: var(--sm); }
  .field input::placeholder { color: rgba(255, 255, 255, .4); }
  .crumbs { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; }
  .crumb { display: flex; align-items: center; gap: 4px; }
  .cl { padding: 0 4px; border-radius: 6px; font-size: var(--sm); color: var(--dim); transition: color .12s; }
  button.cl:hover { color: var(--text); }
  .cl.last { font-weight: 600; color: var(--text); }
  .body { display: flex; flex-direction: column; }
  .empty { display: flex; flex-direction: column; align-items: flex-start; gap: 14px; }
  .list { display: flex; flex-direction: column; }
  .rm { display: inline-flex; }
  .note { padding-top: 12px; font-size: var(--xs); color: var(--faint); }
  .load { padding: 24px 0; font-size: var(--sm); color: var(--dim); }
</style>
