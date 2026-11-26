<script lang="ts">
  // views.rs home_cards: flat dark cards (h 56, gap 14, card colour, radius
  // 8, hover white 4 %) as a grid: as many columns as fit at 260 px, each
  // stretched up to 420; rows are made even (6 cards = 3 + 3). Picks Liked
  // Songs plus the playlists, albums, artists, and songs with the most
  // listening time & activity (falling back to recent items when stats empty).
  // The listening time itself is not shown. Artist cards are round on the
  // left, like their picture.
  import Icon from "../../components/Icon.svelte";
  import ListCover from "../../components/ListCover.svelte";
  import Cover from "../../components/Cover.svelte";
  import SongTitle from "../../components/SongTitle.svelte";
  import ArtistName from "../lists/ArtistName.svelte";
  import { app } from "../../state.svelte";
  import { t, tf } from "../../i18n.svelte";
  import ArtistAvatar from "../../components/ArtistAvatar.svelte";
  import { listId, primaryArtist, rating, trackKey, type ListKey, type Playlist, type Track } from "../../api/types";

  const MAX = 420;
  // Phones get two columns (150 px) instead of a tall stack.
  const MIN = $derived(app.compact ? 150 : 260), GAP = $derived(app.compact ? 8 : 10);
  let w = $state(1000);

  type CardItem =
    | { kind: "liked"; key: string }
    | { kind: "playlist"; list: ListKey; playlist: Playlist; ms: number; key: string }
    | { kind: "album"; list: Extract<ListKey, { kind: "album" }>; ms: number; key: string }
    | { kind: "artist"; name: string; track: Track | null; ms: number; key: string }
    | { kind: "song"; track: Track; ms: number; key: string };

  const cards = $derived.by<CardItem[]>(() => {
    const top = app.library.home_top;
    const out: CardItem[] = [];
    const seen = new Set<string>();
    const add = (c: CardItem): boolean => {
      if (out.length >= 6 || seen.has(c.key)) return false;
      seen.add(c.key);
      out.push(c);
      return true;
    };

    if (app.library.liked.length > 0 || !app.library.playlists.length) {
      add({ kind: "liked", key: "liked" });
    }

    // Build ordered candidate pools per category (listening time + activity first, then recent fallback)
    const playlistPool: CardItem[] = [];
    const plSeen = new Set<string>();
    for (const item of top?.playlists ?? []) {
      if (item.list.kind !== "playlist") continue;
      const p = app.playlist(item.list.id);
      if (!p || plSeen.has(p.id)) continue;
      plSeen.add(p.id);
      playlistPool.push({ kind: "playlist", list: item.list, playlist: p, ms: item.ms, key: `pl:${p.id}` });
    }
    for (const r of app.library.recent) {
      const l = r.kind === "list" ? r.list : r.kind === "song" ? r.from : undefined;
      if (l?.kind === "playlist" && !plSeen.has(l.id)) {
        const p = app.playlist(l.id);
        if (p) { plSeen.add(p.id); playlistPool.push({ kind: "playlist", list: l, playlist: p, ms: 0, key: `pl:${p.id}` }); }
      }
    }
    for (const p of app.library.playlists) {
      if (!plSeen.has(p.id)) {
        plSeen.add(p.id);
        playlistPool.push({ kind: "playlist", list: { kind: "playlist", id: p.id }, playlist: p, ms: 0, key: `pl:${p.id}` });
      }
    }

    const albumPool: CardItem[] = [];
    const alSeen = new Set<string>();
    for (const item of top?.albums ?? []) {
      if (item.list.kind !== "album") continue;
      const id = listId(item.list);
      if (alSeen.has(id)) continue;
      alSeen.add(id);
      albumPool.push({ kind: "album", list: item.list, ms: item.ms, key: id });
    }
    for (const l of app.library.saved_albums) {
      if (l.kind !== "album") continue;
      const id = listId(l);
      if (!alSeen.has(id)) {
        alSeen.add(id);
        albumPool.push({ kind: "album", list: l, ms: 0, key: id });
      }
    }

    const artistPool: CardItem[] = [];
    const arSeen = new Set<string>();
    for (const a of top?.artists ?? []) {
      const ak = a.name.trim().toLowerCase();
      if (!ak || arSeen.has(ak)) continue;
      arSeen.add(ak);
      artistPool.push({ kind: "artist", name: a.name, track: a.track ?? null, ms: a.ms, key: `ar:${ak}` });
    }
    for (const r of app.library.recent) {
      if (r.kind !== "artist") continue;
      const ak = r.name.trim().toLowerCase();
      if (ak && !arSeen.has(ak)) {
        arSeen.add(ak);
        artistPool.push({ kind: "artist", name: r.name, track: r.track ?? null, ms: 0, key: `ar:${ak}` });
      }
    }

    const songPool: CardItem[] = [];
    const sSeen = new Set<string>();
    for (const s of top?.songs ?? []) {
      if (!app.allowed(s.track)) continue;
      const sk = trackKey(s.track);
      if (sSeen.has(sk)) continue;
      sSeen.add(sk);
      songPool.push({ kind: "song", track: s.track, ms: s.ms, key: `s:${sk}` });
    }
    for (const r of app.library.recent) {
      if (r.kind !== "song" || !app.allowed(r.track)) continue;
      const sk = trackKey(r.track);
      if (!sSeen.has(sk)) {
        sSeen.add(sk);
        songPool.push({ kind: "song", track: r.track, ms: 0, key: `s:${sk}` });
      }
    }

    // Take up to 2 playlists/albums, up to 2 artists, up to 2 songs in balanced rounds, then fill up to 6.
    const takeSome = (pool: CardItem[], limit: number) => {
      let n = 0;
      for (const item of pool) {
        if (n >= limit || out.length >= 6) break;
        if (add(item)) n++;
      }
    };
    takeSome(playlistPool, 2);
    takeSome(albumPool, 1);
    takeSome(artistPool, 1);
    takeSome(songPool, 2);
    for (const pool of [songPool, playlistPool, albumPool, artistPool]) {
      for (const item of pool) {
        if (out.length >= 6) break;
        add(item);
      }
    }
    return out;
  });

  const topSongs = $derived(cards.flatMap((c) => (c.kind === "song" ? [c.track] : [])));
  const count = $derived(cards.length);
  const fit = $derived(Math.min(Math.max(1, Math.floor((w + GAP) / (MIN + GAP))), Math.max(1, count)));
  const rows = $derived(Math.ceil(count / fit));
  const cols = $derived(Math.ceil(count / rows));

  const open = (l: ListKey) => app.openListView(l);
  const openAlbum = (l: Extract<ListKey, { kind: "album" }>) => app.openAlbum(l.title, l.artist, l.tracks ?? [], l.cover ?? null);
  function listMenu(l: ListKey) { return (e: MouseEvent) => { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: null, list: l }); }; }
  function songMenu(tr: Track) { return (e: MouseEvent) => { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: tr }); }; }
  function artistMenu(name: string) { return (e: MouseEvent) => { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: null, artist: primaryArtist(name) }); }; }
</script>

<div class="wrap" style:--cg="{app.compact ? 10 : 14}px" bind:clientWidth={w}>
  <div class="grid" style:grid-template-columns="repeat({cols}, minmax(0, 1fr))" style:gap="{GAP}px" style:max-width="{cols * MAX + (cols - 1) * GAP}px">
    {#each cards as c (c.key)}
      {#if c.kind === "liked"}
        <div class="c" onclick={() => open({ kind: "liked" })} oncontextmenu={listMenu({ kind: "liked" })} onkeydown={(e) => e.key === "Enter" && open({ kind: "liked" })} role="button" tabindex="-1">
          <div class="liked"><Icon name="heart" size={20} color="var(--text)" /></div>
          <div class="tx">
            <div class="tl trunc">{t("library.liked")}</div>
            <div class="sub trunc">{tf("common.n_songs", { n: app.library.liked.length })}</div>
          </div>
        </div>
      {:else if c.kind === "playlist"}
        <div class="c" onclick={() => open(c.list)} oncontextmenu={listMenu(c.list)} onkeydown={(e) => e.key === "Enter" && open(c.list)} role="button" tabindex="-1">
          <ListCover list={c.list} size={56} />
          <div class="tx">
            <div class="tl trunc">{c.playlist.name}</div>
            <div class="sub trunc">{tf("common.n_songs", { n: c.playlist.tracks.length })}</div>
          </div>
        </div>
      {:else if c.kind === "album"}
        <div class="c" onclick={() => openAlbum(c.list)} oncontextmenu={listMenu(c.list)} onkeydown={(e) => e.key === "Enter" && openAlbum(c.list)} role="button" tabindex="-1">
          <ListCover list={c.list} size={56} />
          <div class="tx">
            <div class="tl trunc">{c.list.title}</div>
            <div class="sub trunc">{c.list.artist}</div>
          </div>
        </div>
      {:else if c.kind === "artist"}
        <!-- Round on the left like the picture, so no card corners show around it. -->
        <div class="c artist" onclick={() => app.openArtistPage(c.name)} oncontextmenu={artistMenu(c.name)} onkeydown={(e) => e.key === "Enter" && app.openArtistPage(c.name)} role="button" tabindex="-1">
          <ArtistAvatar name={c.name} track={c.track} size={56} iconSize={22} />
          <div class="tx">
            <div class="tl"><ArtistName name={c.name} badge={14} /></div>
            <div class="sub trunc">{t("search.kind.artist")}</div>
          </div>
        </div>
      {:else if c.kind === "song"}
        {@const si = topSongs.findIndex((x) => trackKey(x) === trackKey(c.track))}
        <div class="c" class:off={!app.allowed(c.track)} onclick={() => app.allowed(c.track) && app.play(topSongs, Math.max(0, si))} oncontextmenu={songMenu(c.track)} onkeydown={(e) => e.key === "Enter" && app.allowed(c.track) && app.play(topSongs, Math.max(0, si))} role="button" tabindex="-1">
          <Cover track={c.track} size={56} radius={8} />
          <div class="tx">
            <div class="tl"><SongTitle rating={rating(c.track)}>{c.track.title}</SongTitle></div>
            <div class="sub">
              <button class="link trunc" onclick={(e) => { e.stopPropagation(); app.openArtistPage(c.track.artist); }} oncontextmenu={artistMenu(c.track.artist)}>{c.track.artist}</button>
            </div>
          </div>
        </div>
      {/if}
    {/each}
  </div>
</div>

<style>
  .wrap { width: 100%; position: relative; }
  .grid { display: grid; gap: 10px; width: 100%; }
  .c { position: relative; display: flex; align-items: center; gap: var(--cg, 14px); height: 56px; width: 100%; min-width: 0; overflow: hidden; border-radius: 8px; background: var(--card); cursor: pointer; --ink: 242, 242, 242; }
  .c::before { content: ""; position: absolute; inset: 0; background: rgba(255, 255, 255, .04); opacity: 0; transition: opacity .24s ease-out; pointer-events: none; }
  .c:hover::before { opacity: 1; transition-duration: .12s; }
  .c.off { opacity: .4; }
  .liked { flex: none; width: 56px; height: 56px; display: grid; place-items: center; background: var(--active); }
  .c.artist { border-radius: 28px 8px 8px 28px; }
  .tx { position: relative; min-width: 0; display: flex; flex-direction: column; gap: 2px; padding-right: 10px; }
  .tl { font-size: var(--md); font-weight: 500; min-width: 0; }
  .sub { display: flex; min-width: 0; font-size: var(--sm); color: var(--dim); }
  .link { min-width: 0; max-width: 100%; }
  .link:hover { color: var(--text); text-decoration: underline; }
</style>
