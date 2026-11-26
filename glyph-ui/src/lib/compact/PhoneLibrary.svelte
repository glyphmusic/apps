<script lang="ts">
  // compact.rs library: chips Playlists / Albums / Artists (tap again = all),
  // then rows: 56 px cover, title 15, "Playlist · 12 songs" 12.5 dim with a
  // pink pin when pinned; pinned lists first; followed artists round.
  // Folders: rows after Liked Songs / pinned lists with the folder picture; a
  // tap opens the folder (back row with its name, its playlists below;
  // Android back closes it). Playlists inside a folder leave the top level.
  import { fade } from "svelte/transition";
  import Icon from "../components/Icon.svelte";
  import Chip from "../components/Chip.svelte";
  import Cover from "../components/Cover.svelte";
  import ListCover from "../components/ListCover.svelte";
  import FolderCover from "../components/FolderCover.svelte";
  import { app } from "../state.svelte";
  import { t, tf } from "../i18n.svelte";
  import { sameList, type ListKey } from "../api/types";
  import { phone } from "./cstate.svelte";
  const f = $derived(phone.libFilter);
  const set = (n: number) => {
    const next = phone.libFilter === n ? 0 : n;
    phone.libFilter = next;
    app.libraryTab = next;
  };
  const folder = $derived(app.libraryFolder ? app.folder(app.libraryFolder) ?? null : null);
  $effect(() => { if (app.libraryFolder && !folder) app.libraryFolder = null; });
  const inFolder = $derived(new Set((app.library.folders ?? []).flatMap((x) => x.playlists)));
  const folders = $derived(!folder && f <= 1 ? app.library.folders ?? [] : []);
  const lists = $derived.by(() => {
    const out: { list: ListKey; sub: string }[] = [];
    const pl = t("list.kind.playlist");
    if (folder) {
      for (const p of app.folderPlaylists(folder.id)) out.push({ list: { kind: "playlist", id: p.id }, sub: `${pl} · ${tf("common.n_songs", { n: p.tracks.length })}` });
      return out;
    }
    if (f <= 1) {
      out.push({ list: { kind: "liked" }, sub: `${pl} · ${tf("common.n_songs", { n: app.library.liked.length })}` });
      if (f === 0 && app.library.recent.length) out.push({ list: { kind: "recent" }, sub: pl });
      if (f === 0 && app.library.downloads.length) out.push({ list: { kind: "downloads" }, sub: `${pl} · ${tf("common.n_songs", { n: app.library.downloads.length })}` });
      for (const p of app.library.playlists) {
        if (inFolder.has(p.id)) continue;
        out.push({ list: { kind: "playlist", id: p.id }, sub: `${pl} · ${tf("common.n_songs", { n: p.tracks.length })}` });
      }
    }
    if (f === 0 || f === 2) for (const a of app.library.saved_albums) out.push({ list: a, sub: a.kind === "album" ? `${t("list.kind.album")} · ${a.artist}` : t("list.kind.album") });
    // Pinned lists show here too, as in the sidebar on a computer (an album
    // pinned without saving it, Recent or Downloads under a filter, ...).
    if (f === 0) for (const p of app.library.pinned) {
      if (p.kind === "playlist" && inFolder.has(p.id)) continue;
      if (out.some((x) => sameList(x.list, p))) continue;
      const n = app.listTracks(p).length;
      out.push({ list: p, sub: p.kind === "album" ? `${t("list.kind.album")} · ${p.artist}` : `${pl} · ${tf("common.n_songs", { n })}` });
    }
    const rank = (l: ListKey) => { const i = app.library.pinned.findIndex((p) => sameList(p, l)); return i < 0 ? 1e9 : i; };
    return out.sort((a, b) => rank(a.list) - rank(b.list));
  });
  const artists = $derived(!folder && (f === 0 || f === 3) ? app.library.followed : []);
  // Liked Songs, Recent, Downloads and pinned lists stay above the folders.
  const headN = $derived.by(() => {
    if (folder) return 0;
    const head = new Set(["liked", "recent", "downloads"]);
    const i = lists.findIndex((x) => !head.has(x.list.kind) && !app.isPinned(x.list));
    return i < 0 ? lists.length : i;
  });
  function folderMenu(e: MouseEvent, id: string) {
    e.preventDefault(); e.stopPropagation();
    app.openCtx({ x: e.clientX, y: e.clientY, track: app.folderTracks(id)[0] ?? null, folder: id });
  }
  const artTrack = (name: string) => app.library.liked.find((x) => x.artist === name) ?? null;
  function listMenu(e: MouseEvent, l: ListKey) {
    e.preventDefault(); e.stopPropagation();
    app.openCtx({ x: e.clientX, y: e.clientY, track: app.listTracks(l)[0] ?? null, list: l });
  }
  function artistMenu(e: MouseEvent, name: string) {
    e.preventDefault(); e.stopPropagation();
    app.openCtx({ x: e.clientX, y: e.clientY, track: artTrack(name), artist: name });
  }
</script>

{#snippet listRow(it: { list: ListKey; sub: string })}
  <button class="lr" onclick={() => app.openListView(it.list)} oncontextmenu={(e) => listMenu(e, it.list)}>
    <ListCover list={it.list} size={56} />
    <span class="tx">
      <span class="t trunc">{app.listTitle(it.list)}</span>
      <span class="s">{#if app.isPinned(it.list)}<Icon name="pin-fill" size={12} color="var(--accent)" />{/if}<span class="trunc">{it.sub}</span></span>
    </span>
  </button>
{/snippet}

{#if folder}
  <div class="fbar">
    <button class="fback" onclick={() => (app.libraryFolder = null)} aria-label={t("library.folder.back")}>
      <Icon name="chevron-left" size={20} color="rgba(255,255,255,.85)" />
    </button>
    <span class="ftx">
      <span class="ft trunc">{folder.name}</span>
      <span class="fs trunc">{t("nav.library")}</span>
    </span>
    <button class="fmore" onclick={(e) => folderMenu(e, folder.id)} aria-label={t("ctx.more")}>
      <Icon name="ellipsis" size={20} color="rgba(255,255,255,.85)" />
    </button>
  </div>
{:else}
  <div class="chips">
    <Chip on={f === 1} onclick={() => set(1)}>{t("library.filter.playlists")}</Chip>
    <Chip on={f === 2} onclick={() => set(2)}>{t("search.filter.albums")}</Chip>
    <Chip on={f === 3} onclick={() => set(3)}>{t("search.filter.artists")}</Chip>
  </div>
{/if}
<div class="stage">
  {#key `${f}:${folder?.id ?? ""}`}
    <div
      class="slot"
      in:fade={{ duration: 150, delay: 95 }}
      out:fade={{ duration: 95 }}
      onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")}
    >
      {#each lists.slice(0, headN) as it}{@render listRow(it)}{/each}
      {#each folders as fo (fo.id)}
        {@const n = app.folderPlaylists(fo.id).length}
        <button class="lr" onclick={() => (app.libraryFolder = fo.id)} oncontextmenu={(e) => folderMenu(e, fo.id)}>
          <FolderCover id={fo.id} size={56} />
          <span class="tx">
            <span class="t trunc">{fo.name}</span>
            <span class="s"><span class="trunc">{t("list.kind.folder")} · {tf(n === 1 ? "library.folder.one_playlist" : "library.folder.n_playlists", { n })}</span></span>
          </span>
        </button>
      {/each}
      {#each lists.slice(headN) as it}{@render listRow(it)}{/each}
      {#if folder && !lists.length}
        <div class="fempty">
          <Icon name="folder-open" size={28} color="rgba(255,255,255,.4)" />
          <span class="fe1">{t("library.folder.empty.title")}</span>
          <span class="fe2">{t("library.folder.empty.sub_touch")}</span>
        </div>
      {/if}
      {#each artists as name}
        {@const tr = artTrack(name)}
        <button class="lr" onclick={() => app.openArtistPage(name)} oncontextmenu={(e) => artistMenu(e, name)}>
          {#if tr}<Cover track={tr} size={56} round />{:else}<span class="ph"><Icon name="user-round" size={22} color="rgba(255,255,255,.5)" /></span>{/if}
          <span class="tx"><span class="t trunc">{name}</span><span class="s">{t("search.kind.artist")}</span></span>
        </button>
      {/each}
      <div style:height="24px"></div>
    </div>
  {/key}
</div>

<style>
  .chips { display: flex; gap: 8px; padding: 0 16px 10px; }
  .fbar { display: flex; align-items: center; gap: 6px; padding: 0 8px 10px; }
  .fback, .fmore { flex: none; width: 40px; height: 40px; border-radius: 50%; display: grid; place-items: center; transition: background-color .12s var(--ease), transform .16s var(--ease); }
  .fback:active, .fmore:active { background: rgba(255, 255, 255, .08); transform: scale(.94); }
  .ftx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .ft { font-size: 17px; font-weight: 600; color: var(--text); }
  .fs { font-size: 12.5px; color: var(--dim); }
  .fempty { display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 48px 32px; text-align: center; }
  .fe1 { margin-top: 6px; font-size: 15px; font-weight: 500; color: var(--text); }
  .fe2 { font-size: 13px; color: var(--dim); }
  .stage { display: grid; }
  .slot { grid-area: 1 / 1; min-width: 0; }
  .lr { width: 100%; display: flex; align-items: center; gap: 12px; padding: 6px 16px; text-align: left; border-radius: 8px; transition: background-color .12s var(--ease), transform .16s var(--ease); }
  .lr:active { background: rgba(255, 255, 255, .05); transform: scale(.98); }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .t { font-size: 15px; color: var(--text); }
  .s { display: flex; align-items: center; gap: 5px; min-width: 0; font-size: 12.5px; color: var(--dim); }
  .ph { width: 56px; height: 56px; border-radius: 50%; background: var(--card); display: grid; place-items: center; }
</style>
