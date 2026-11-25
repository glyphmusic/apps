<script lang="ts">
  // lists/ctx_menu.rs ctx_menu: w 244 (MENU_W), p 6, radius 10,
  // hsla(0,0,.075,.98), 1 px white 10 %; opens with a fade and a 6 px slide
  // (140 ms, ease-out cubic), fades out in 110 ms. Kept 8 px inside the
  // window; closes on a click outside, Escape or the wheel.
  // Artist menu: name header → queue, play next, artist radio, follow,
  // copy link | hide artist. List menu: cover header → save (albums), add
  // all to playlist | play, shuffle, queue, radio, download all | pin, open,
  // share, go to artist; own playlists: edit, storage, versions, delete.
  // Song menu: cover header → play, play next, queue | like, add to
  // playlist (+ remove in playlists) | artist, album, radio | recommendations
  // (+ remove recent), more (download, copy link, video, wrong song).
  // List menu: play, shuffle, queue | save / add to playlist, (own: add songs,
  // edit) | more | delete. Sidebar entries add "Hide from sidebar".
  // Flyouts (add to playlist, recommendations, storage, versions) open on
  // hover, right of the menu or left when they would not fit.
  // Own playlists: "Move to folder" flyout (folders with a round check,
  // "Remove from folder", "New folder"). Folder menu: folder header → play,
  // shuffle, queue | rename | delete folder (its playlists stay).
  // "Save to" (right-click on a heart): song header, then Liked Songs and
  // every playlist with a round check (filled = the song is in it); a click
  // adds or removes and the menu stays open; "New playlist" at the end.
  import { tick } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Cover from "../components/Cover.svelte";
  import ListCover from "../components/ListCover.svelte";
  import FolderCover from "../components/FolderCover.svelte";
  import ArtistAvatar from "../components/ArtistAvatar.svelte";
  import Icon from "../components/Icon.svelte";
  import MenuItem from "./menu/MenuItem.svelte";
  import Sep from "./menu/Sep.svelte";
  import Flyout from "./menu/Flyout.svelte";
  import { app } from "../state.svelte";
  import { onWeb } from "../web";
  import { t, tf } from "../i18n.svelte";
  import { fadeEdges, outside, eatTap } from "../ui.svelte";
  import { drag, phone, sheetIn, sheetOut, shouldClose } from "../compact/cstate.svelte";
  import { videoState } from "../pages/nowplaying/video-state.svelte";
  import { splitArtists, trackKey, type ListKey, type Track } from "../api/types";
  import {
    artistPage, artistRadio, contextOf, copyArtistLink, deletePlaylist, hideSidebar, isDownloading, isHiddenArtist, lib, listRadio,
    openNameDialog, playNextAll, playShuffled, queueAll, recommendFeedback, removeDownloads, removeShown, restoreVersion,
    setPlaylistStorage, shareList, shownTracks, songRadio, toggleHiddenArtist, indexInPlaylist, toggleInPlaylist,
  } from "../pages/lists/actions";

  const MENU_W = 244, SUB_W = 228, SUB_GAP = 5;
  type Sub = "playlist" | "recommend" | "more" | "storage" | "versions" | "artists" | "folder" | null;
  type MoreSub = "artists" | "storage" | "versions" | null;

  let m = $state(app.ctx!);
  $effect.pre(() => { if (app.ctx) m = app.ctx; });
  const tr = $derived(m.track);
  const list = $derived(m.list ?? null);
  const artist = $derived(m.artist ?? null);
  const fold = $derived(m.folder ? app.folder(m.folder) ?? null : null);
  // Songs the menu acts on: one song, a whole list or a whole folder.
  const songs = $derived<Track[]>(m.tracks?.length ? m.tracks : fold ? app.folderTracks(fold.id) : list ? shownTracks(list) : tr ? [tr] : []);
  let sub = $state<Sub>(null);
  let directArtists = $state(false);
  let moreSub = $state<MoreSub>(null);
  let moreSubTop = $state(-6);
  let subDir = $state<1 | -1 | 0>(0);
  let pull = $state(0);
  let vh = $state(typeof window !== "undefined" ? window.innerHeight : 800);
  let grabEl: HTMLDivElement | undefined = $state();
  let bodyEl: HTMLDivElement | undefined = $state();
  let bodyH = $state<string | undefined>(undefined);
  const compact = $derived(app.compact);
  const subOpen = (s: Sub) => sub === s;
  const closesSub = () => { if (!compact) { sub = null; moreSub = null; } };
  function enterMoreSub(next: MoreSub, e?: MouseEvent) {
    if (compact) return;
    moreSub = next;
    if (next && e) moreSubTop = (e.currentTarget as HTMLElement).offsetTop - 6;
  }
  const bodyMax = $derived(Math.max(120, Math.min(vh * 0.82, vh - 16) - (grabEl?.offsetHeight ?? 76) - 16));
  async function setSub(next: Sub, dirOverride?: 1 | -1) {
    if (sub === next) return;
    const dir: 1 | -1 = dirOverride ?? (next ? 1 : -1);
    if (compact && bodyEl) {
      const from = bodyEl.offsetHeight;
      bodyH = `${from}px`;
      subDir = dir;
      sub = next;
      await tick();
      if (!bodyEl) { bodyH = undefined; return; }
      const cur = bodyEl.querySelector<HTMLElement>(".bs-slot:not(.leaving)");
      const to = cur ? Math.min(cur.offsetHeight, bodyMax) : from;
      if (Math.abs(to - from) < 1) { bodyH = undefined; return; }
      void bodyEl.offsetHeight;
      bodyH = `${to}px`;
    } else {
      subDir = dir;
      sub = next;
    }
  }
  const subTitle = $derived(
    sub === "playlist" ? t(list ? "ctx.add_all_playlist" : "ctx.add_playlist")
    : sub === "recommend" ? t("ctx.recommend")
    : sub === "more" ? t("ctx.more")
    : sub === "storage" ? t("ctx.storage")
    : sub === "versions" ? t("ctx.versions")
    : sub === "artists" ? t("library.artists")
    : sub === "folder" ? t("ctx.move_to_folder")
    : "",
  );
  // List and folder menus name the list, not its first song.
  const subContext = $derived(fold ? fold.name : list ? app.listTitle(list) : tr ? tr.title : artist ?? "");

  // Position: at the pointer, kept 8 px inside the window.
  let el: HTMLDivElement | undefined = $state();
  let w = $state(MENU_W), h = $state(0);
  const left = $derived(Math.max(8, Math.min(m.x, innerWidth - w - 8)));
  const top = $derived(Math.max(8, Math.min(m.y, innerHeight - h - 8)));
  const flip = $derived(left + w + SUB_GAP + SUB_W + 8 > innerWidth);
  const nestedFlip = $derived(flip || left + w + SUB_GAP + SUB_W * 2 + 12 > innerWidth);

  function close() { app.ctx = null; }
  /** begin_ctx_close, then the action. */
  function run(f: () => unknown) { return () => { close(); f(); }; }
  $effect(() => {
    const wheel = () => { if (!compact) close(); };
    addEventListener("wheel", wheel, { passive: true });
    return () => removeEventListener("wheel", wheel);
  });
  // A new menu (another row right-clicked or preset artist picker) initializes cleanly.
  $effect.pre(() => {
    const ctx = app.ctx;
    if (!ctx) return;
    const preset = ctx.tracks?.length ? "playlist" : ctx.artists?.length ? "artists" : null;
    directArtists = !!preset;
    sub = preset;
    moreSub = null;
    moreSubTop = -6;
    subDir = 0;
    bodyH = undefined;
    pull = 0;
  });

  // ---- artist ----
  const aSongs = $derived(artist ? artistPage(artist)?.tracks ?? [] : []);
  // ---- list ----
  const own = $derived(list?.kind === "playlist" ? list.id : null);
  const album = $derived(list?.kind === "album");
  const has = $derived(songs.length > 0);
  const allDl = $derived(has && songs.every((x) => app.isDownloaded(x)));
  const anyDlBusy = $derived(has && songs.some(isDownloading));
  const allDlOrBusy = $derived(has && songs.every((x) => app.isDownloaded(x) || isDownloading(x)));
  const versions = $derived(own ? (lib().playlist_versions ?? []).filter((v) => v.id === own).slice(0, 10) : []);
  const listKind = $derived(list?.kind === "album" ? list.artist : t("list.kind.playlist"));
  // ---- song ----
  const removable = $derived(m.from && (m.from.list.kind === "playlist" || m.from.list.kind === "recent") ? m.from : null);
  const pickedFrom = $derived(m.tracks?.length && m.from?.list.kind === "playlist" ? m.from.list.id : null);
  const targets = $derived(app.library.playlists.filter((p) => p.id !== own && p.id !== pickedFrom));

  function addTo(id: string) { close(); app.addToPlaylist(id, songs); if (m.tracks?.length) app.stopPicking(); }
  function newPlaylist() { openNameDialog(songs, null); if (m.tracks?.length) app.stopPicking(); }
  function openList(l: ListKey) { close(); app.openListView(l); }
  function openAddSongs(id: string) {
    app.showQueue = false;
    if (app.view !== "list" || app.openList?.kind !== "playlist" || app.openList.id !== id) {
      app.openListView({ kind: "playlist", id });
    }
    app.addSongs = { id, tab: 0 };
  }

  const sheetArtists = $derived(
    m.artists?.length ? m.artists
    : tr ? splitArtists(tr.artist)
    : list && (list.kind === "album" || list.kind === "artist_liked") ? splitArtists(list.artist)
    : [],
  );
  function openArtist(name: string) {
    close();
    if (compact) phone.npOpen = false;
    app.openArtistPage(name);
  }
  function openAlbum(albumName: string, artistName: string, song: Track) {
    close();
    if (compact) phone.npOpen = false;
    app.openAlbum(albumName, artistName, [], song);
  }
  function openVideo(song: Track) {
    close();
    const isCurrent = app.player.current && trackKey(app.player.current) === trackKey(song);
    if (!isCurrent) void app.play([song], 0);
    if (compact) {
      phone.npOpen = true;
      phone.videoMode = true;
    } else {
      app.lyricsOnly = false;
      app.videoMode = true;
      app.navigate("now_playing");
    }
  }
</script>

{#snippet playlistBody()}
  <MenuItem icon="plus" label={t("library.playlist.new")} onclick={newPlaylist} />
  <Sep />
  {#if !targets.length}
    <div class="none">
      <span class="n1">{t("ctx.no_playlists")}</span>
      <span class="n2">{t("ctx.no_playlists.sub")}</span>
    </div>
  {/if}
  {#each targets as p (p.id)}
    <button class="pl-row" role="menuitem" onclick={() => addTo(p.id)}>
      <ListCover list={{ kind: "playlist", id: p.id }} size={compact ? 38 : 28} />
      <span class="pl-tx">
        <span class="pl-nm trunc">{p.name}</span>
        <span class="pl-ct trunc">{tf("common.n_songs", { n: p.tracks.length })}</span>
      </span>
    </button>
  {/each}
{/snippet}

{#snippet folderBody(id: string)}
  {@const cur = app.folderOf(id)?.id ?? null}
  <MenuItem icon="folder-plus" label={t("library.folder.new")} onclick={run(() => app.newFolder([id]))} />
  {#if app.library.folders?.length}<Sep />{/if}
  {#each app.library.folders ?? [] as f (f.id)}
    {@const on = cur === f.id}
    <button class="pl-row" role="menuitemradio" aria-checked={on} onclick={run(() => app.movePlaylistToFolder(id, on ? null : f.id))}>
      <FolderCover id={f.id} size={compact ? 38 : 28} />
      <span class="pl-tx">
        <span class="pl-nm trunc">{f.name}</span>
        <span class="pl-ct trunc">{tf(app.folderPlaylists(f.id).length === 1 ? "library.folder.one_playlist" : "library.folder.n_playlists", { n: app.folderPlaylists(f.id).length })}</span>
      </span>
      {@render saveMark(on)}
    </button>
  {/each}
  {#if cur}
    <Sep />
    <MenuItem icon="circle-minus" label={t("ctx.remove_from_folder")} onclick={run(() => app.movePlaylistToFolder(id, null))} />
  {/if}
{/snippet}

{#snippet saveMark(on: boolean)}
  <span class="mark" class:on aria-hidden="true">{#if on}<Icon name="check" size={compact ? 14 : 12} />{/if}</span>
{/snippet}

{#snippet saveBody(song: Track)}
  {@const liked = app.isLiked(song)}
  <div class="save-lbl">{t("ctx.save_to")}</div>
  <div class="save-list" class:save-scroll={!compact} use:fadeEdges={{ top: 10, bottom: 10, minOverflow: 8 }}>
  <button class="pl-row" role="menuitemcheckbox" aria-checked={liked} onclick={() => app.toggleLike(song)}>
    <ListCover list={{ kind: "liked" }} size={compact ? 38 : 28} />
    <span class="pl-tx">
      <span class="pl-nm trunc">{t("library.liked")}</span>
      <span class="pl-ct trunc">{tf("common.n_songs", { n: app.library.liked.length })}</span>
    </span>
    {@render saveMark(liked)}
  </button>
  {#each app.library.playlists as p (p.id)}
    {@const inside = indexInPlaylist(p.id, song) >= 0}
    <button class="pl-row" role="menuitemcheckbox" aria-checked={inside} onclick={() => toggleInPlaylist(p.id, song)}>
      <ListCover list={{ kind: "playlist", id: p.id }} size={compact ? 38 : 28} />
      <span class="pl-tx">
        <span class="pl-nm trunc">{p.name}</span>
        <span class="pl-ct trunc">{tf("common.n_songs", { n: p.tracks.length })}</span>
      </span>
      {@render saveMark(inside)}
    </button>
  {/each}
  </div>
  <Sep />
  <MenuItem icon="plus" label={t("library.playlist.new")} onclick={newPlaylist} />
{/snippet}

{#snippet recommendBody(song: Track)}
  {@const disliked = app.isDislikedTrack(song)}
  <MenuItem
    icon={disliked ? "rotate-ccw" : "circle-minus"}
    label={t(disliked ? "taste.ctx.restore_song" : "taste.ctx.dislike_song")}
    onclick={run(() => app.toggleDislikeTrack(song))}
  />
  <Sep />
  {#each [["recommend.more", 1, "thumbs-up"], ["recommend.less", -1, "thumbs-down"], ["recommend.block", -2, "ban"], ["recommend.reset", null, "rotate-ccw"]] as const as [key, pref, icon]}
    <MenuItem {icon} label={t(key)} onclick={run(() => recommendFeedback(song.artist, pref))} />
  {/each}
  {#if removable && removable.list.kind === "recent"}
    {@const from = removable}
    <Sep />
    <MenuItem icon="circle-minus" label={t("ctx.remove_recent")} danger onclick={run(() => removeShown(from.list, from.index))} />
  {/if}
{/snippet}

{#snippet artistsBody(names: string[], avatarSize: number)}
  {#each names as a (a)}
    <button class="art-row" onclick={() => openArtist(a)}>
      <ArtistAvatar name={a} size={avatarSize} />
      <span class="art-nm trunc">{a}</span>
    </button>
  {/each}
{/snippet}

{#snippet moreBody(song: Track)}
  {#if !onWeb() && !isDownloading(song)}
    {#if app.isDownloaded(song)}
      <MenuItem icon="trash-2" label={t("ctx.remove_download")} danger onclick={run(() => removeDownloads([song]))} />
    {:else}
      <MenuItem icon="download" label={t("ctx.download")} onclick={run(() => app.download([song]))} />
    {/if}
  {/if}
  <MenuItem icon={app.compact ? "share-2" : "link-2"} label={t(app.compact ? "share.title" : "ctx.copy_link")} onclick={run(() => app.share({ track: song }))} />
  {#if !(compact && m.nowPlaying && phone.videoMode)}
    <MenuItem icon="tv" label={t("ctx.watch_video")} onclick={() => openVideo(song)} />
  {/if}
  <Sep />
  <MenuItem icon="arrow-right-left" label={t("ctx.wrong_song")} onclick={run(() => app.openWrongSong(song))} />
{/snippet}

{#snippet listMoreBody(k: ListKey)}
  {@const pinned = app.isPinned(k)}
  {#if has}
    <MenuItem icon="radio" label={t("ctx.radio")} onenter={() => (moreSub = null)} onclick={run(() => listRadio(k))} />
    {#if !onWeb()}
      {#if allDl}
        <MenuItem icon="trash-2" label={t("ctx.remove_downloads")} danger onenter={() => (moreSub = null)} onclick={run(() => removeDownloads(songs))} />
      {:else if !allDlOrBusy}
        <MenuItem icon="download" label={t("ctx.download_all")} onenter={() => (moreSub = null)} onclick={run(() => app.download(songs))} />
      {:else}
        <MenuItem icon="x" label={t("ctx.cancel_downloads")} danger onenter={() => (moreSub = null)} onclick={run(() => app.cancelDownloads(songs))} />
      {/if}
    {/if}
    <Sep />
  {/if}
  <MenuItem icon={pinned ? "pin-fill" : "pin"} label={t(pinned ? "ctx.unpin" : "ctx.pin")} onenter={() => (moreSub = null)} onclick={run(() => app.togglePin(k))} />
  <MenuItem icon="list-music" label={t("ctx.open")} onenter={() => (moreSub = null)} onclick={() => openList(k)} />
  {#if has}<MenuItem icon={app.compact ? "share-2" : "link-2"} label={t("ctx.share_list")} onenter={() => (moreSub = null)} onclick={run(() => shareList(k))} />{/if}
  {#if k.kind === "album" || k.kind === "artist_liked"}
    {@const a = k.artist}
    {@const listArtists = splitArtists(a)}
    {#if listArtists.length > 1 && !compact}
      <div class="sub-wrap" role="none" onmouseenter={(e) => enterMoreSub("artists", e)}>
        <MenuItem icon="user-round" label={t("ctx.artist")} sub open={moreSub === "artists"} onenter={() => (moreSub = "artists")} />
      </div>
    {:else if listArtists.length > 1}
      <MenuItem icon="user-round" label={t("ctx.artist")} sub onclick={() => setSub("artists")} />
    {:else}
      <MenuItem icon="user-round" label={t("ctx.artist")} onenter={() => (moreSub = null)} onclick={() => openArtist(a)} />
    {/if}
  {:else if own}
    {@const id = own}
    {#if app.account || versions.length}<Sep />{/if}
    {#if app.account}
      {#if !compact}
        <div class="sub-wrap" role="none" onmouseenter={(e) => enterMoreSub("storage", e)}>
          <MenuItem icon="hard-drive" label={t("ctx.storage")} sub open={moreSub === "storage"} onenter={() => (moreSub = "storage")} />
        </div>
      {:else}
        <MenuItem icon="hard-drive" label={t("ctx.storage")} sub onclick={() => setSub("storage")} />
      {/if}
    {/if}
    {#if versions.length}
      {#if !compact}
        <div class="sub-wrap" role="none" onmouseenter={(e) => enterMoreSub("versions", e)}>
          <MenuItem icon="history" label={t("ctx.versions")} sub open={moreSub === "versions"} onenter={() => (moreSub = "versions")} />
        </div>
      {:else}
        <MenuItem icon="history" label={t("ctx.versions")} sub onclick={() => setSub("versions")} />
      {/if}
    {/if}
  {/if}
{/snippet}

{#snippet storageBody(id: string)}
  {#each [[0, "playlist.scope.local"], [1, "playlist.scope.account"], [2, "playlist.scope.offline"]] as const as [mode, key]}
    <MenuItem icon="hard-drive" label={t(key)} onclick={run(() => setPlaylistStorage(id, mode))} />
  {/each}
{/snippet}

{#snippet versionsBody()}
  {#each versions as v (v.revision)}
    <MenuItem icon="history" label={tf("playlist.restore_version", { date: v.saved_at })} onclick={run(() => restoreVersion(v.revision))} />
  {/each}
{/snippet}

{#snippet playlistSub()}
  <MenuItem icon="list-music" label={t(list ? "ctx.add_all_playlist" : "ctx.add_playlist")} sub open={!compact && subOpen("playlist")}
    onenter={() => { if (!compact) sub = "playlist"; }} onclick={compact ? () => setSub("playlist") : undefined}>
    {#if !compact && subOpen("playlist")}
      <Flyout {flip}>{@render playlistBody()}</Flyout>
    {/if}
  </MenuItem>
{/snippet}

{#snippet header()}
  {#if artist}
    <div class="ah trunc">{artist}</div>
  {:else if fold}
    {@const n = app.folderPlaylists(fold.id).length}
    <div class="hd">
      <FolderCover id={fold.id} size={compact ? 42 : 36} />
      <div class="ht">
        <span class="t1 trunc">{fold.name}</span>
        <span class="t2 trunc">{t("list.kind.folder")} · {tf(n === 1 ? "library.folder.one_playlist" : "library.folder.n_playlists", { n })}</span>
      </div>
    </div>
  {:else if list}
    <div class="hd">
      <ListCover {list} size={compact ? 42 : 36} />
      <div class="ht">
        <span class="t1 trunc">{app.listTitle(list)}</span>
        <span class="t2 trunc">{listKind}</span>
      </div>
    </div>
  {:else if tr}
    <div class="hd">
      <Cover track={tr} size={compact ? 42 : 36} radius={5} />
      <div class="ht">
        <span class="t1 trunc">{tr.title}</span>
        <span class="t2 trunc">{tr.artist}</span>
      </div>
    </div>
  {/if}
{/snippet}

{#snippet rootItems()}
  {#if m.save && tr}
    {@render saveBody(tr)}
  {:else if artist}
    {#if aSongs.length}
      <MenuItem icon="list-plus" label={t("ctx.queue")} onenter={closesSub} onclick={run(() => queueAll(aSongs))} />
      <MenuItem icon="list-start" label={t("ctx.play_next")} onenter={closesSub} onclick={run(() => playNextAll(aSongs))} />
    {/if}
    <MenuItem icon="radio" label={t("ctx.artist_radio")} onenter={closesSub} onclick={run(() => artistRadio(artist))} />
    <MenuItem icon={app.follows(artist) ? "check" : "user-round"} label={t(app.follows(artist) ? "artist.unfollow" : "artist.follow")} onenter={closesSub} onclick={run(() => app.toggleFollow(artist))} />
    <MenuItem icon={app.compact ? "share-2" : "link-2"} label={t(app.compact ? "share.title" : "ctx.copy_link")} onenter={closesSub} onclick={run(() => copyArtistLink(artist))} />
    <Sep />
    {#if isHiddenArtist(artist)}
      <MenuItem icon="eye" label={t("ctx.show_artist")} onenter={closesSub} onclick={run(() => toggleHiddenArtist(artist))} />
    {:else}
      <MenuItem icon="eye-off" label={t("ctx.hide_artist")} danger onenter={closesSub} onclick={run(() => toggleHiddenArtist(artist))} />
    {/if}
  {:else if fold}
    {@const fid = fold.id}
    {#if has}
      <MenuItem icon="play" label={t("ctx.play")} onenter={closesSub} onclick={run(() => app.play(songs, 0, fold.name))} />
      <MenuItem icon="shuffle" label={t("ctx.shuffle")} onenter={closesSub} onclick={run(() => playShuffled(songs, fold.name))} />
      <MenuItem icon="list-plus" label={t("ctx.queue")} onenter={closesSub} onclick={run(() => queueAll(songs))} />
      <Sep />
    {/if}
    {#if !(app.view === "library" && app.libraryFolder === fid)}
      <MenuItem icon="folder-open" label={t("ctx.open")} onenter={closesSub} onclick={run(() => { if (app.view !== "library") app.navigate("library"); app.libraryFolder = fid; })} />
    {/if}
    <MenuItem icon="pencil" label={t("library.folder.rename")} onenter={closesSub} onclick={run(() => app.renameFolderDialog(fid))} />
    <Sep />
    <MenuItem icon="trash-2" label={t("library.folder.delete")} danger onenter={closesSub} onclick={run(() => app.deleteFolder(fid))} />
  {:else if list}
    {#if has}
      <MenuItem icon="play" label={t("ctx.play")} onenter={closesSub} onclick={run(() => app.play(songs, 0, contextOf(list)))} />
      <MenuItem icon="shuffle" label={t("ctx.shuffle")} onenter={closesSub} onclick={run(() => playShuffled(songs, contextOf(list)))} />
      <MenuItem icon="list-plus" label={t("ctx.queue")} onenter={closesSub} onclick={run(() => queueAll(songs))} />
      <Sep />
    {/if}
    {#if album}
      {@const saved = app.isSavedAlbum(list)}
      <MenuItem icon={saved ? "heart-fill" : "heart"} color={saved ? "var(--accent)" : undefined} label={t(saved ? "ctx.unsave" : "ctx.save")} onenter={closesSub} onclick={run(() => app.toggleSaveAlbum(list))} />
    {/if}
    {#if has}{@render playlistSub()}{/if}
    {#if own}
      {@const id = own}
      <MenuItem icon="circle-plus" label={t("add.title")} onenter={closesSub} onclick={run(() => openAddSongs(id))} />
      <MenuItem icon="pencil" label={t("list.edit")} onenter={closesSub} onclick={() => openNameDialog([], id)} />
      <MenuItem icon="folder" label={t("ctx.move_to_folder")} sub open={!compact && subOpen("folder")}
        onenter={() => { if (!compact) sub = "folder"; }} onclick={compact ? () => setSub("folder") : undefined}>
        {#if !compact && subOpen("folder")}<Flyout {flip}>{@render folderBody(id)}</Flyout>{/if}
      </MenuItem>
    {/if}
    {#if album || has || own}<Sep />{/if}
    {#if !onWeb() && anyDlBusy}
      <MenuItem icon="x" label={t("ctx.cancel_downloads")} danger onenter={closesSub} onclick={run(() => app.cancelDownloads(songs))} />
    {/if}
    <MenuItem icon="ellipsis" label={t("ctx.more")} sub open={!compact && subOpen("more")}
      onenter={() => { if (!compact) { sub = "more"; moreSub = null; } }} onclick={compact ? () => setSub("more") : undefined}>
      {#if !compact && subOpen("more")}
        {@const listArtists = list.kind === "album" || list.kind === "artist_liked" ? splitArtists(list.artist) : []}
        <Flyout {flip}>
          {@render listMoreBody(list)}
          {#snippet aside()}
            {#if moreSub === "artists" && listArtists.length > 1}
              <Flyout flip={nestedFlip} top={moreSubTop}>{@render artistsBody(listArtists, 24)}</Flyout>
            {:else if moreSub === "storage" && own}
              <Flyout flip={nestedFlip} top={moreSubTop}>{@render storageBody(own)}</Flyout>
            {:else if moreSub === "versions" && versions.length}
              <Flyout flip={nestedFlip} top={moreSubTop}>{@render versionsBody()}</Flyout>
            {/if}
          {/snippet}
        </Flyout>
      {/if}
    </MenuItem>
    {#if own}
      {@const id = own}
      <Sep />
      <MenuItem icon="trash-2" label={t("list.delete")} danger onenter={closesSub} onclick={run(() => deletePlaylist(id))} />
    {/if}
  {:else if tr}
    {@const song = tr}
    {@const liked = app.isLiked(song)}
    {@const songArtists = splitArtists(song.artist)}
    <!-- Play first, then keep (like, playlist), then go (artist, album,
         radio); the rarely used ones live under "More". -->
    <MenuItem icon="play" label={t("ctx.play")} onenter={closesSub} onclick={run(() => app.play([song], 0))} />
    <MenuItem icon="list-start" label={t("ctx.play_next")} onenter={closesSub} onclick={run(() => app.queueAdd([song], true))} />
    <MenuItem icon="list-plus" label={t("ctx.queue")} onenter={closesSub} onclick={run(() => app.queueAdd([song], false))} />
    <Sep />
    <MenuItem icon={liked ? "heart-fill" : "heart"} color={liked ? "var(--accent)" : undefined} label={t(liked ? "ctx.unlike" : "ctx.like")} onenter={closesSub} onclick={run(() => app.toggleLike(song))} />
    {@render playlistSub()}
    {#if removable && removable.list.kind === "playlist"}
      {@const from = removable}
      <MenuItem icon="circle-minus" label={t("ctx.remove")} danger onenter={closesSub} onclick={run(() => removeShown(from.list, from.index))} />
    {/if}
    <Sep />
    {#if songArtists.length > 1}
      <MenuItem icon="user-round" label={t("ctx.artist")} sub open={!compact && subOpen("artists")}
        onenter={() => { if (!compact) sub = "artists"; }} onclick={compact ? () => setSub("artists") : undefined}>
        {#if !compact && subOpen("artists")}<Flyout {flip}>{@render artistsBody(songArtists, 24)}</Flyout>{/if}
      </MenuItem>
    {:else}
      <MenuItem icon="user-round" label={t("ctx.artist")} onenter={closesSub} onclick={() => openArtist(songArtists[0] ?? song.artist)} />
    {/if}
    {#if song.album?.trim()}
      <MenuItem icon="disc-3" label={t("ctx.album")} onenter={closesSub} onclick={() => openAlbum(song.album!, song.artist, song)} />
    {/if}
    <MenuItem icon="radio" label={t("ctx.radio")} onenter={closesSub} onclick={run(() => songRadio(song))} />
    {#if compact && m.nowPlaying && phone.videoMode}
      <MenuItem icon="search" label={t("video.switch")} onenter={closesSub} onclick={run(() => (phone.videoPickerOpen = true))} />
      <MenuItem icon="music-2" label={t("video.back_to_song")} onenter={closesSub} onclick={run(() => { videoState.leave(true); phone.videoMode = false; phone.videoResume = false; phone.videoPickerOpen = false; })} />
    {/if}
    {#if !onWeb() && isDownloading(song)}
      <MenuItem icon="x" label={t("ctx.cancel_download")} danger onenter={closesSub} onclick={run(() => app.cancelDownloads([song]))} />
    {/if}
    <Sep />
    <MenuItem icon="sliders-horizontal" label={t("ctx.recommend")} sub open={!compact && subOpen("recommend")}
      onenter={() => { if (!compact) sub = "recommend"; }} onclick={compact ? () => setSub("recommend") : undefined}>
      {#if !compact && subOpen("recommend")}<Flyout {flip}>{@render recommendBody(song)}</Flyout>{/if}
    </MenuItem>
    <MenuItem icon="ellipsis" label={t("ctx.more")} sub open={!compact && subOpen("more")}
      onenter={() => { if (!compact) { sub = "more"; moreSub = null; } }} onclick={compact ? () => setSub("more") : undefined}>
      {#if !compact && subOpen("more")}<Flyout {flip}>{@render moreBody(song)}</Flyout>{/if}
    </MenuItem>
  {/if}

  {#if compact && m.nowPlaying && !m.save}
    {@const lyricLine = app.config.ui.np_lyric_line !== false}
    <Sep />
    <button
      class="sw-row"
      role="switch"
      aria-checked={lyricLine}
      onclick={() => app.setConfig((c) => { c.ui.np_lyric_line = !(c.ui.np_lyric_line !== false); })}
    >
      <Icon name="mic-vocal" size={18} color="rgba(255,255,255,.7)" />
      <span class="sw-lb trunc">{t("ctx.lyric_line")}</span>
      <span class="sw-tog" class:on={lyricLine} aria-hidden="true"><span class="sw-knob"></span></span>
    </button>
  {/if}

  {#if m.sidebar && !m.save}
    {@const sid = m.sidebar}
    {#if artist || list || tr}<Sep />{/if}
    <MenuItem icon="eye-off" label={t("ctx.hide_sidebar")} onenter={closesSub} onclick={run(() => hideSidebar(sid))} />
  {/if}
{/snippet}

<svelte:window bind:innerHeight={vh} />

{#if compact}
  <div class="bs-dim" transition:fade|global={{ duration: 180 }} onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")} onpointerdown={() => { eatTap(); close(); }} oncontextmenu={(e) => e.preventDefault()} role="presentation">
    <div class="bs-panel" data-part="menu" style:--bs-body-max="{bodyMax}px" style:transform="translateY({pull}px)" class:settle={pull === 0} class:dragging={pull > 0} in:sheetIn|global={{ duration: 340 }} out:sheetOut|global={{ duration: 210 }} onpointerdown={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
      <div class="bs-grab" bind:this={grabEl} use:drag={{ axis: "y", move: (_, dy) => (pull = Math.max(0, dy)), end: (_, dy, _vx, vy) => { if (shouldClose(dy, vy)) { close(); return; } pull = 0; } }}>
        <span class="hdl"></span>
        <div class="bs-stage">
          {#key sub ?? "root"}
            <div
              class="bs-slot"
              in:fly|local={{ x: subDir * 36, duration: subDir ? 220 : 0, easing: cubicOut }}
              out:fly|local={{ x: -subDir * 24, duration: subDir ? 150 : 0, easing: cubicOut }}
              onoutrostart={(e) => (e.currentTarget as HTMLElement).classList.add("leaving")}
            >
              {#if sub === "artists" && directArtists}
                <div class="bs-title">{t("library.artists")}</div>
              {:else if sub === "playlist" && directArtists}
                <div class="bs-title">{t("pick.playlist")} · {tf("pick.n", { n: songs.length })}</div>
              {:else if sub}
                <button class="bs-back" onclick={() => setSub((sub === "artists" || sub === "storage" || sub === "versions") && list ? "more" : null, -1)}>
                  <span class="bs-bk-ic"><Icon name="chevron-left" size={20} color="rgba(255,255,255,.85)" /></span>
                  <span class="ht">
                    <span class="t1 trunc">{subTitle}</span>
                    {#if subContext}<span class="t2 trunc">{subContext}</span>{/if}
                  </span>
                </button>
              {:else if artist || list || tr || fold}
                <div class="bs-hd">{@render header()}</div>
              {/if}
            </div>
          {/key}
        </div>
      </div>
      <div
        class="bs-stage bs-body"
        bind:this={bodyEl}
        style:height={bodyH}
        ontransitionend={(e) => { if (e.target === bodyEl && e.propertyName === "height") bodyH = undefined; }}
      >
        {#key sub ?? "root"}
          <div
            class="bs-slot bs-list scroll"
            use:fadeEdges={{ top: 12, bottom: 12, minOverflow: 8 }}
            use:drag={{ axis: "y", onlyAtTop: () => (bodyEl?.querySelector<HTMLElement>(".bs-slot:not(.leaving)")?.scrollTop ?? 0) <= 0, move: (_, dy) => (pull = Math.max(0, dy)), end: (_, dy, _vx, vy) => { if (shouldClose(dy, vy)) { close(); return; } pull = 0; } }}
            in:fly|local={{ x: subDir * 40, duration: subDir ? 220 : 0, easing: cubicOut }}
            out:fly|local={{ x: -subDir * 28, duration: subDir ? 150 : 0, easing: cubicOut }}
            onoutrostart={(e) => (e.currentTarget as HTMLElement).classList.add("leaving")}
          >
            {#if sub === "playlist"}{@render playlistBody()}
            {:else if sub === "recommend" && tr}{@render recommendBody(tr)}
            {:else if sub === "more" && list}{@render listMoreBody(list)}
            {:else if sub === "more" && tr}{@render moreBody(tr)}
            {:else if sub === "storage" && own}{@render storageBody(own)}
            {:else if sub === "folder" && own}{@render folderBody(own)}
            {:else if sub === "versions"}{@render versionsBody()}
            {:else if sub === "artists"}{@render artistsBody(sheetArtists, 44)}
            {:else}{@render rootItems()}{/if}
          </div>
        {/key}
      </div>
    </div>
  </div>
{:else}
  <div class="ctx" data-part="menu" bind:this={el} bind:clientWidth={w} bind:clientHeight={h} style:left="{left}px" style:top="{top}px" use:outside={close}
    oncontextmenu={(e) => e.preventDefault()} role="menu" tabindex="-1"
    in:fly|global={{ y: -6, duration: 140, easing: cubicOut }} out:fly|global={{ y: -6, duration: 110, easing: cubicOut }}>
    {#if artist || list || tr || fold}{@render header()}<Sep />{/if}
    {@render rootItems()}
  </div>
{/if}

<style>
  .ctx { position: fixed; z-index: 80; width: 244px; padding: 6px; border-radius: 10px; background: #181818; display: flex; flex-direction: column; font-family: var(--font); }
  .ah { padding: 6px 8px; font-size: 13px; font-weight: 600; color: rgba(255, 255, 255, .95); }
  .hd { display: flex; align-items: center; gap: 10px; padding: 4px 6px; }
  .ht { min-width: 0; display: flex; flex-direction: column; }
  .t1 { font-size: 13px; font-weight: 500; color: rgba(255, 255, 255, .95); }
  .t2 { font-size: 12px; color: rgba(255, 255, 255, .55); }
  .save-lbl { padding: 6px 10px 4px; font-size: 11.5px; font-weight: 600; color: rgba(255, 255, 255, .5); }
  .bs-list .save-lbl { padding: 6px 16px 6px; font-size: 12.5px; }
  .save-list { display: flex; flex-direction: column; }
  .save-scroll { max-height: min(60vh, 420px); overflow-y: auto; }
  .mark { flex: none; width: 18px; height: 18px; border-radius: 50%; box-shadow: inset 0 0 0 1.5px rgba(255, 255, 255, .35); display: grid; place-items: center; color: #0d0d0d; transition: background-color .14s ease-out, box-shadow .14s ease-out, transform .16s var(--ease); }
  .mark.on { background: var(--accent); box-shadow: none; animation: mark-pop .24s var(--ease); }
  .bs-list .mark { width: 22px; height: 22px; }
  .pl-row:hover .mark:not(.on) { box-shadow: inset 0 0 0 1.5px rgba(255, 255, 255, .6); }
  @keyframes mark-pop { 0% { transform: scale(.7); } 60% { transform: scale(1.15); } 100% { transform: scale(1); } }
  .none { display: flex; flex-direction: column; gap: 2px; padding: 8px 10px; cursor: default; }
  .n1 { font-size: 13px; color: rgba(255, 255, 255, .75); }
  .n2 { font-size: 12px; color: rgba(255, 255, 255, .45); }

  .bs-dim { position: fixed; inset: 0; z-index: 80; background: rgba(0, 0, 0, .55); }
  .bs-panel { position: absolute; left: 0; right: 0; bottom: 0; max-height: min(82vh, calc(100vh - var(--safe-top, env(safe-area-inset-top)) - 16px)); border-radius: 16px 16px 0 0; border-top: 1px solid rgba(255, 255, 255, .08); background: hsl(0 0% 8%); box-shadow: 0 -12px 40px rgba(0, 0, 0, .6); padding-bottom: max(10px, var(--safe-bottom, env(safe-area-inset-bottom)), var(--ime-bottom, 0px)); display: flex; flex-direction: column; overflow: hidden; --mi-h: 42px; --mi-px: 14px; --mi-r: 10px; --mi-fs: 15.5px; --mi-gap: 13px; }
  .bs-panel.settle { transition: transform .24s var(--ease); }
  .bs-panel.dragging { will-change: transform; }
  .bs-grab { flex: none; touch-action: none; }
  .hdl { display: block; width: 36px; height: 4px; margin: 10px auto 6px; border-radius: 2px; background: rgba(255, 255, 255, .25); }
  .bs-hd { padding: 4px 10px 10px; border-bottom: 1px solid rgba(255, 255, 255, .07); }
  .bs-hd .t1, .bs-back .t1 { font-size: 15px; font-weight: 600; }
  .bs-hd .t2, .bs-back .t2 { font-size: 12.5px; }
  .bs-hd .ah { padding: 6px 8px; font-size: 16px; font-weight: 700; }
  .bs-title { padding: 8px 16px 14px; text-align: center; font-size: 16px; font-weight: 700; color: rgba(255, 255, 255, .95); border-bottom: 1px solid rgba(255, 255, 255, .07); }
  .bs-back { width: 100%; padding: 8px 16px 14px; display: flex; align-items: center; gap: 10px; text-align: left; border-bottom: 1px solid rgba(255, 255, 255, .07); }
  .bs-bk-ic { flex: none; width: 36px; height: 36px; border-radius: 50%; background: rgba(255, 255, 255, .06); display: grid; place-items: center; transition: background-color .14s ease-out, transform .16s var(--ease); }
  .bs-back:active .bs-bk-ic { background: rgba(255, 255, 255, .12); transform: scale(.92); }
  .bs-stage { position: relative; display: grid; grid-template-columns: minmax(0, 1fr); align-items: start; overflow: hidden; }
  .bs-body { flex: 0 1 auto; min-height: 0; transition: height .22s var(--ease); }
  .bs-slot { grid-area: 1 / 1; min-width: 0; }
  .bs-slot:global(.leaving) { position: absolute; top: 0; left: 0; right: 0; pointer-events: none; }
  .bs-list { flex: 0 1 auto; min-height: 0; max-height: var(--bs-body-max, 65vh); padding: 6px 8px; display: flex; flex-direction: column; }

  .pl-row { flex: none; width: 100%; min-height: var(--mi-h, 38px); padding: 5px var(--mi-px, 10px); border-radius: var(--mi-r, 6px); display: flex; align-items: center; gap: var(--mi-gap, 10px); text-align: left; cursor: pointer; transition: background-color .12s ease-out, transform .16s var(--ease); }
  .bs-list .pl-row { padding-top: 7px; padding-bottom: 7px; }
  .pl-row:hover { background: rgba(255, 255, 255, .07); }
  .pl-row:active { background: rgba(255, 255, 255, .1); transform: scale(.975); }
  .pl-tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .pl-nm { font-size: var(--mi-fs, 13px); color: rgba(255, 255, 255, .9); font-weight: 500; }
  .pl-ct { font-size: 11.5px; color: rgba(255, 255, 255, .5); }
  .bs-list .pl-ct { font-size: 12.5px; }

  .art-row { flex: none; width: 100%; min-height: var(--mi-h, 36px); padding: 5px var(--mi-px, 10px); border-radius: var(--mi-r, 6px); display: flex; align-items: center; gap: var(--mi-gap, 10px); text-align: left; cursor: pointer; transition: background-color .12s ease-out, transform .16s var(--ease); }
  .bs-list .art-row { min-height: 58px; padding-top: 7px; padding-bottom: 7px; }
  .art-row:hover { background: rgba(255, 255, 255, .07); }
  .art-row:active { background: rgba(255, 255, 255, .1); transform: scale(.975); }
  .art-nm { flex: 1; min-width: 0; font-size: var(--mi-fs, 13px); font-weight: 500; color: rgba(255, 255, 255, .95); }
  .bs-list .art-nm { font-size: 15.5px; }

  .sw-row { flex: none; width: 100%; height: var(--mi-h, 42px); padding: 0 var(--mi-px, 14px); border-radius: var(--mi-r, 10px); display: flex; align-items: center; gap: var(--mi-gap, 13px); text-align: left; font-size: var(--mi-fs, 15.5px); color: rgba(255, 255, 255, .88); cursor: pointer; transition: background-color .12s ease-out, transform .16s var(--ease); }
  .sw-row:hover { background: rgba(255, 255, 255, .07); }
  .sw-row:active { background: rgba(255, 255, 255, .1); transform: scale(.975); }
  .sw-lb { flex: 1; min-width: 0; }
  .sw-tog { flex: none; width: 36px; height: 20px; padding: 2px; border-radius: 10px; display: flex; background: rgba(255, 255, 255, .14); transition: background-color .15s ease-out; }
  .sw-knob { width: 16px; height: 16px; border-radius: 50%; background: rgba(255, 255, 255, .75); transition: transform .18s var(--ease), background-color .15s; }
  .sw-tog.on { background: #fff; }
  .sw-tog.on .sw-knob { background: #000; transform: translateX(16px); }
</style>
