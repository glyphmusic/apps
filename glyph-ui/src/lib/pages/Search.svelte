<script lang="ts">
  // views.rs search: column gap 18: heading (“query” / "Search"), filter
  // chips (gap 8), query completions as chips, the body, footer. Body: All /
  // Tracks = matching artists on top (Spotify-style, All only, max 3),
  // songs found through a line of their lyrics, then the song list (track
  // rows, "Song" chip, results decode in); Albums / Artists = one row per
  // album / artist (44 px cover). States: offline, search unavailable, type
  // to search, skeleton rows while loading, nothing found. Footer: spinner
  // line while more sources answer and a retry for failing sources. Near the
  // end of the page the next page is requested (app.searchMore).
  import { fade } from "svelte/transition";
  import SectionTitle from "../components/SectionTitle.svelte";
  import Chip from "../components/Chip.svelte";
  import Icon from "../components/Icon.svelte";
  import Cover from "../components/Cover.svelte";
  import Button from "../components/Button.svelte";
  import EmptyState from "../components/EmptyState.svelte";
  import Decode from "../components/Decode.svelte";
  import VirtualRows from "../components/VirtualRows.svelte";
  import SkeletonSwap from "../components/SkeletonSwap.svelte";
  import TrackRow from "./search/TrackRow.svelte";
  import FollowButton from "./search/FollowButton.svelte";
  import ArtistName from "./search/ArtistName.svelte";
  import Skeletons from "./search/Skeletons.svelte";
  import Avatar from "./profile/Avatar.svelte";
  import { backend } from "../api/backend";
  import { filterSearchHistory } from "./search/history-filter";
  import { app, type SearchFilter } from "../state.svelte";
  import { registerListNav, scrollVirtualRowIntoView } from "../state/list-nav.svelte";
  import { t, tf } from "../i18n.svelte";
  import { uniqBy } from "../fmt";
  import { PROVIDER_NAME, primaryArtist, type Profile, type Release, type SearchItem, type Track } from "../api/types";

  const s = $derived(app.search);
  const filter = $derived(app.searchFilter);
  const query = $derived(s.query);
  const offline = $derived(app.backendState.kind === "offline");
  // People: a query that is one handle, with or without "@", is looked up as
  // a profile (exact handle, the server has no fuzzy people search). Found
  // profiles show on top of All and Artists.
  const handleQ = $derived.by(() => {
    const h = query.trim().replace(/^@/, "").toLowerCase();
    return /^[a-z0-9_]{3,20}$/.test(h) && !/^_+$/.test(h) ? h : null;
  });
  const atQ = $derived(query.trim().startsWith("@"));
  let prof = $state<Profile | null>(null);
  let profBusy = $state(false);
  let profSignedOut = $state(false);
  $effect(() => {
    const h = handleQ;
    prof = null; profSignedOut = false;
    if (!h) { profBusy = false; return; }
    profBusy = true;
    let alive = true;
    const timer = setTimeout(() => {
      backend().call<Profile | null>("profile", { handle: h }).then(
        (p) => { if (alive) { prof = p ?? null; profBusy = false; } },
        (e) => { if (alive) { prof = null; profBusy = false; profSignedOut = String((e as Error)?.message ?? e).includes("signed_out"); } },
      );
    }, 250);
    return () => { alive = false; clearTimeout(timer); };
  });
  const profRow = $derived(prof && (filter === "all" || filter === "artists") ? prof : null);
  const any = $derived(s.tracks.length > 0 || s.albums.length > 0 || s.artists.length > 0 || s.lyric_hits.length > 0 || !!profRow);
  function openProf(p: Profile) { app.openProfilePage(p.handle); }
  const failing = $derived(!offline && (s.errors.length > 0 || !!s.error));
  const FILTERS: [SearchFilter, string][] = [["all", "search.filter.all"], ["tracks", "search.filter.tracks"], ["albums", "search.filter.albums"], ["artists", "search.filter.artists"]];

  // Matching artists on top (search_artists).
  const artistRows = $derived(filter === "all" ? uniqBy(s.artists, (a) => a.name).slice(0, 3) : []);
  /** Query completions from history, suggestions, artists, and tracks. */
  const cleanCompletion = (t: string) =>
    t.replace(/[\(\[]\s*(?:official|lyrics?|audio|video|visualizer|explicit|hd|4k|hq|mv)[^\)\]]*[\)\]]/gi, "")
      .replace(/[\(\[]\s*([^\)\]]+)\s*[\)\]]/g, " $1")
      .replace(/\s*[-–—]\s*/g, " ")
      .replace(/\s+/g, " ")
      .trim();

  const completions = $derived.by(() => {
    const rawQ = query.trim();
    const q = rawQ.toLowerCase();
    if (q.length < 2) return [];
    const out: string[] = [];
    const seen = new Set<string>();
    const max = app.compact ? 5 : 8;
    const add = (cand: string) => {
      const clean = cand.replace(/\s+/g, " ").trim();
      const l = clean.toLowerCase();
      const norm = l.replace(/[-–—]/g, " ").replace(/\s+/g, " ").trim();
      if (!l || l === q || norm === q || l.length <= q.length || !l.includes(q) || [...clean].length > 48) return;
      if (seen.has(norm)) return;
      seen.add(norm);
      out.push(clean);
    };
    for (const h of app.library.search_history) {
      if (h.kind === "query") add(h.text);
      if (out.length >= max) return out;
    }
    for (const sg of s.suggestions) {
      add(cleanCompletion(sg));
      if (out.length >= max) return out;
    }
    for (const tr of s.tracks) {
      const base = tr.title.split(/[(\[]/)[0].trim();
      const tidy = cleanCompletion(tr.title);
      const artist = primaryArtist(tr.artist).trim();
      const artistMatch = artist && !q.includes(artist.toLowerCase()) && base.toLowerCase().includes(q) ? `${base} ${artist}` : "";
      for (const cand of [tidy, base, artist, artistMatch]) {
        if (cand) add(cand);
        if (out.length >= max) return out;
      }
    }
    for (const a of s.artists) {
      add(a.name);
      if (out.length >= max) return out;
    }
    for (const al of s.albums) {
      add(cleanCompletion(al.title));
      if (out.length >= max) return out;
    }
    return out;
  });

  const asTrack = (r: Release): Track => ({ id: r.id, source: "deezer", title: r.title, artist: r.artist, album: r.title, cover_url: r.cover_url ?? undefined });
  const songsOf = (name: string) => s.tracks.filter((tr) => primaryArtist(tr.artist).toLowerCase() === name.toLowerCase()).length;
  function artistTrack(a: { name: string; track?: Track | null }): Track | null { return a.track ?? s.tracks.find((tr) => primaryArtist(tr.artist).toLowerCase() === a.name.toLowerCase()) ?? null; }

  function pickArtist(a: { name: string; track?: Track | null }) {
    app.pickSearch({ kind: "artist", name: a.name, track: artistTrack(a) });
    app.openArtistPage(a.name);
  }
  function artistMenu(name: string) { return (e: MouseEvent) => { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: null, artist: primaryArtist(name) }); }; }
  function albumMenu(r: Release) { return (e: MouseEvent) => { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: null, list: { kind: "album", title: r.title, artist: r.artist, tracks: [asTrack(r)] } }); }; }

  function pickHistory(it: SearchItem) {
    if (it.kind === "song") {
      app.playOne(it.track);
      app.pickSearch(it);
    } else if (it.kind === "artist") {
      app.openArtistPage(it.name);
      app.pickSearch(it);
    } else {
      app.searchFor(it.text);
    }
  }
  function historyMenu(e: MouseEvent, it: SearchItem) {
    if (it.kind === "song") {
      e.preventDefault();
      e.stopPropagation();
      app.openCtx({ x: e.clientX, y: e.clientY, track: it.track });
    } else if (it.kind === "artist") {
      e.preventDefault();
      e.stopPropagation();
      app.openCtx({ x: e.clientX, y: e.clientY, track: it.track ?? null, artist: it.name });
    }
  }

  // Near the end of the list: ask for the next page.
  let scroller: HTMLElement | null = null;
  let trackListEl: HTMLDivElement | undefined = $state();
  function near() {
    if (!scroller || !s.tracks.length || (filter !== "all" && filter !== "tracks")) return;
    if (scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 900) app.searchMore();
  }
  $effect(() => {
    scroller = document.getElementById("main-scroll");
    scroller?.addEventListener("scroll", near, { passive: true });
    return () => scroller?.removeEventListener("scroll", near);
  });
  $effect(() => { void s.tracks.length; void s.loading; void filter; queueMicrotask(near); });

  $effect(() => {
    if (filter !== "all" && filter !== "tracks") return;
    return registerListNav({
      count: () => s.tracks.length,
      play: (i) => {
        const tr = s.tracks[i];
        if (!tr) return;
        if (!app.allowed(tr)) { app.toast(t("toast.explicit_blocked"), "info"); return; }
        app.play(s.tracks, i);
      },
      scrollTo: (i) => scrollVirtualRowIntoView(trackListEl, i, 58, 48, 32),
    });
  });

  const filteredHistory = $derived(filterSearchHistory(app.library.search_history, app.library.recent_lists, filter));
  const hasAnyHistory = $derived(app.library.search_history.length > 0 || app.library.recent_lists.some((l) => l.kind === "album"));

  const SPIN = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";
  const spin = $derived(SPIN[Math.floor(app.clock / 80) % SPIN.length]);
  const curly = (q: string) => `“${q}”`;
</script>

<div class="search">
  <!-- Phones (app.compact): the title sits in the compact header, filters only
       with a query, no completion chips, and an empty query shows nothing here
       (the phone draws its own recent searches). -->
  {#if !app.compact}<SectionTitle heading title={query.trim() ? curly(query) : t("nav.search")} />{/if}

  {#if !app.compact || query.trim()}
    <div class="chips">
      {#each FILTERS as [f, key]}
        <Chip on={filter === f} onclick={() => (app.searchFilter = f)}>{t(key)}</Chip>
      {/each}
    </div>
  {/if}

  {#if completions.length}
    {@const q = query.trim().toLowerCase()}
    <div class="sugg-list">
      {#each completions as c (c)}
        <div class="sl-row" onclick={() => app.searchFor(c)} onkeydown={(e) => e.key === "Enter" && app.searchFor(c)} role="button" tabindex="-1">
          <Icon name="search" size={16} color="var(--dim)" />
          <span class="sl-tx trunc">
            {#if c.toLowerCase().startsWith(`${q} `)}
              <span class="sl-q">{c.slice(0, q.length)}</span><span class="sl-hi">{c.slice(q.length)}</span>
            {:else}
              <span class="sl-hi">{c}</span>
            {/if}
          </span>
          <button class="sl-fill" type="button" aria-label={c} onclick={(e) => { e.stopPropagation(); app.setQuery(c); }}>
            <Icon name="arrow-up" size={16} color="var(--dim)" style="transform: rotate(-45deg)" />
          </button>
        </div>
      {/each}
    </div>
  {/if}

  {#if offline && !any}
    <EmptyState icon="refresh-cw" title={t("offline.title")} sub={t("offline.search")} />
  {:else if s.error && !any}
    <EmptyState icon="refresh-cw" title={t("search.unavailable")} sub={t("search.retry_sources")} />
  {:else if !query.trim() && app.compact}
    <!-- phone: compact.rs search_recent is drawn by the phone layout -->
  {:else if !query.trim()}
    {#if hasAnyHistory}
      <div class="hist">
        <div class="lh"><Icon name="clock" size={15} color="var(--dim)" /><span>{t("search.history")}</span></div>
        <div class="stage">
          {#key filter}
            <div
              class="slot"
              in:fade={{ duration: 150, delay: 95 }}
              out:fade={{ duration: 95 }}
              onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")}
            >
              {#if filteredHistory.length > 0}
                <div class="list">
                  {#each filteredHistory as entry}
                    {#if entry.kind === "album"}
                      {@const al = entry.list}
                      <div
                        class="grow"
                        onclick={() => app.openAlbum(al.title, al.artist, al.tracks, entry.coverTrack ?? undefined)}
                        oncontextmenu={(e) => { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: entry.coverTrack, list: al }); }}
                        onkeydown={(e) => e.key === "Enter" && app.openAlbum(al.title, al.artist, al.tracks, entry.coverTrack ?? undefined)}
                        role="button"
                        tabindex="-1"
                      >
                        <Cover track={entry.coverTrack} size={44} radius={6} />
                        <div class="gt">
                          <div class="g1 trunc">{al.title}</div>
                          <div class="g2 trunc">{t("list.kind.album")} • {al.artist}</div>
                        </div>
                      </div>
                    {:else}
                      {@const it = entry.item}
                      <div
                        class="grow"
                        onclick={() => pickHistory(it)}
                        oncontextmenu={(e) => historyMenu(e, it)}
                        onkeydown={(e) => e.key === "Enter" && pickHistory(it)}
                        role="button"
                        tabindex="-1"
                      >
                        {#if it.kind === "song"}
                          <Cover track={it.track} size={44} radius={6} />
                        {:else if it.kind === "artist"}
                          {#if it.track}
                            <Cover track={it.track} size={44} round />
                          {:else}
                            <span class="ph r"><Icon name="user-round" size={20} color="rgba(255,255,255,.6)" /></span>
                          {/if}
                        {:else}
                          <span class="ph"><Icon name="search" size={20} color="rgba(255,255,255,.6)" /></span>
                        {/if}
                        <div class="gt">
                          <div class="g1 trunc">
                            {#if it.kind === "artist"}<ArtistName name={it.name} badge={14} />{:else}{it.kind === "song" ? it.track.title : it.text}{/if}
                          </div>
                          <div class="g2 trunc">{it.kind === "song" ? `${t("search.kind.song")} • ${it.track.artist}` : it.kind === "artist" ? t("search.kind.artist") : t("search.kind.query")}</div>
                        </div>
                        <button
                          class="x"
                          aria-label={t("tip.x")}
                          onclick={(e) => { e.stopPropagation(); app.removeSearchHistory(entry.origIndex); }}
                        >
                          <Icon name="x" size={16} color="var(--dim)" />
                        </button>
                      </div>
                    {/if}
                  {/each}
                </div>
              {:else}
                <EmptyState icon="search" title={t("search.none.title")}>
                  <Button onclick={() => (app.searchFilter = "all")}>{t("search.filter.all")}</Button>
                </EmptyState>
              {/if}
            </div>
          {/key}
        </div>
      </div>
    {:else}
      <EmptyState icon="search" title={t("search.empty.title")} sub={t("search.empty.sub")} />
    {/if}
  {:else}
    <SkeletonSwap loading={!any && (s.loading || (atQ && profBusy))}>
      {#snippet skeleton()}
        <div class="skel">
          {#if filter === "all"}
            <Skeletons kind="artists" n={2} prefix="search" />
            <Skeletons kind="rows" n={7} prefix="search" />
          {:else if filter === "artists"}
            <Skeletons kind="artists" n={6} prefix="search" />
          {:else}
            <Skeletons kind="rows" n={7} prefix="search" />
          {/if}
        </div>
      {/snippet}
      {#if !any && s.errors.length}
        <EmptyState icon="refresh-cw" title={t("search.unavailable")} sub={t("search.retry_sources")} />
      {:else if !any && atQ && profSignedOut}
        <EmptyState icon="user-round" title={t("profile.signed_out")} sub={t("profile.signed_out.sub")} />
      {:else if !any}
        <EmptyState title={t("search.none.title")} sub={atQ ? t("profile.not_found") : curly(query)} />
      {:else if filter === "all" || filter === "tracks"}
        <div class="list">
          {#if profRow}
            {@const pr = profRow}
            <div class="arow" onclick={() => openProf(pr)} onkeydown={(e) => e.key === "Enter" && openProf(pr)} role="button" tabindex="-1">
              <Avatar src={pr.avatar} size={48} flat />
              <span class="an pn"><span class="trunc">{pr.name || pr.handle}</span><span class="ph-h trunc">@{pr.handle}</span></span>
              <Chip kind="kind">{t("search.kind.profile")}</Chip>
            </div>
          {/if}
          {#each artistRows as a, i (a.name)}
            {@const tr = artistTrack(a)}
            <div class="arow" onclick={() => pickArtist(a)} oncontextmenu={artistMenu(a.name)} onkeydown={(e) => e.key === "Enter" && pickArtist(a)} role="button" tabindex="-1">
              <Cover track={tr} size={48} radius={24} />
              <span class="an"><ArtistName name={a.name} badge={15}><Decode text={a.name} key="search:artist:{a.name}" /></ArtistName></span>
              <Chip kind="kind">{t("search.kind.artist")}</Chip>
              <FollowButton name={a.name} />
            </div>
          {/each}

          {#if s.lyric_hits.length}
            {@const hits = s.lyric_hits.map((h) => h.track)}
            <div class="lyr">
              <div class="lh"><Icon name="mic-vocal" size={15} color="var(--dim)" /><span>{t("search.lyrics.title")}</span></div>
              {#each s.lyric_hits as h, i (i)}
                <div class="r"><TrackRow tracks={hits} index={i} kind="search" lyric={h.line} chip={t("search.lyrics.in")} /></div>
              {/each}
            </div>
          {/if}

          <div bind:this={trackListEl}>
            <VirtualRows count={s.tracks.length} pitch={58}>
              {#snippet row(i)}<TrackRow tracks={s.tracks} index={i} kind="search" />{/snippet}
            </VirtualRows>
          </div>
        </div>
      {:else if filter === "albums"}
        <div class="list">
          <VirtualRows count={s.albums.length} pitch={58}>
            {#snippet row(i)}
              {@const r = s.albums[i]}
              <div class="grow" onclick={() => app.openAlbum(r.title, r.artist, [], asTrack(r), { kind: r.kind, tracks: r.tracks })} oncontextmenu={albumMenu(r)} onkeydown={(e) => e.key === "Enter" && app.openAlbum(r.title, r.artist, [], asTrack(r), { kind: r.kind, tracks: r.tracks })} role="button" tabindex="-1">
                <Cover track={asTrack(r)} size={44} radius={6} />
                <div class="gt"><div class="g1 trunc">{r.title}</div><div class="g2 trunc">{r.artist}</div></div>
                {#if r.tracks}<div class="gn">{tf("common.n_songs", { n: r.tracks })}</div>{/if}
              </div>
            {/snippet}
          </VirtualRows>
        </div>
      {:else}
        <div class="list">
          {#if profRow}
            {@const pr = profRow}
            <div class="arow" onclick={() => openProf(pr)} onkeydown={(e) => e.key === "Enter" && openProf(pr)} role="button" tabindex="-1">
              <Avatar src={pr.avatar} size={44} flat />
              <span class="an pn"><span class="trunc">{pr.name || pr.handle}</span><span class="ph-h trunc">@{pr.handle}</span></span>
              <Chip kind="kind">{t("search.kind.profile")}</Chip>
            </div>
          {/if}
          <VirtualRows count={s.artists.length} pitch={58}>
            {#snippet row(i)}
              {@const a = s.artists[i]}
              {@const n = songsOf(a.name)}
              <div class="grow" onclick={() => pickArtist(a)} oncontextmenu={artistMenu(a.name)} onkeydown={(e) => e.key === "Enter" && pickArtist(a)} role="button" tabindex="-1">
                <Cover track={artistTrack(a)} size={44} radius={22} />
                <div class="gt"><div class="g1"><ArtistName name={a.name} badge={14} /></div></div>
                {#if n}<div class="gn">{tf("common.n_songs", { n })}</div>{/if}
              </div>
            {/snippet}
          </VirtualRows>
        </div>
      {/if}
    </SkeletonSwap>
  {/if}

  <div class="foot">
    {#if s.loading && any}
      <div class="spin"><span class="mono">{spin}</span><span>{t("search.more")}</span></div>
    {/if}
    {#if failing}
      <div class="part">
        <div class="pt">{s.errors.length ? tf("search.partial_sources", { names: s.errors.map((e) => PROVIDER_NAME[e.source]).join(", ") }) : t("search.retry_sources")}</div>
        <div><Button onclick={() => app.searchFor(query)}>{t("common.retry")}</Button></div>
      </div>
    {/if}
  </div>
</div>

<style>
  .search { display: flex; flex-direction: column; gap: 18px; min-width: 0; }
  .chips { display: flex; align-items: center; gap: 8px; }
  .sugg-list { display: flex; flex-direction: column; margin: -6px 0 -4px; }
  .sl-row { display: flex; align-items: center; gap: 14px; height: 42px; padding: 0 10px; border-radius: 8px; cursor: pointer; transition: background-color .16s ease-out; }
  .sl-row:hover { background: var(--card); }
  .sl-row:active:not(:has(.sl-fill:active)) { background: rgba(255, 255, 255, .08); }
  .sl-tx { flex: 1; min-width: 0; font-size: var(--md); }
  .sl-q { color: var(--dim); }
  .sl-hi { color: var(--text); font-weight: 500; }
  .sl-fill { flex: none; width: 34px; height: 34px; border-radius: 50%; display: grid; place-items: center; transition: background-color .12s; }
  .sl-fill:hover { background: rgba(255, 255, 255, .08); }
  .sl-fill:active { background: rgba(255, 255, 255, .14); }
  :global(html.compact) .sl-row { height: 44px; padding: 0 4px; }
  :global(html.compact) .sl-tx { font-size: 14.5px; }
  :global(html.compact) .sl-fill { width: 38px; height: 38px; }
  .list, .skel { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .skel { padding-top: 8px; }
  .r { content-visibility: auto; contain-intrinsic-size: auto 56px; }
  /* artist row: gap 14, px 10, py 6, radius 8, hover card colour */
  .arow, .grow { display: flex; align-items: center; gap: 14px; padding: 6px 10px; border-radius: 8px; cursor: pointer; transition: background-color .24s ease-out; }
  .arow:hover:not(:has(.fb:hover, .fb:active)), .grow:hover { background: var(--card); transition-duration: .12s; }
  .arow:active:not(:has(.fb:active)), .grow:active { background: rgba(255, 255, 255, .08); transition-duration: .08s; }
  :global(html.compact) .arow, :global(html.compact) .grow { padding: 6px 4px; }
  .an { flex: 1; min-width: 0; font-size: var(--md); font-weight: 500; color: var(--text); }
  .an.pn { display: flex; flex-direction: column; gap: 2px; }
  .ph-h { font-size: var(--sm); font-weight: 400; color: var(--dim); font-family: var(--mono); }
  .gt { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .g1 { font-size: var(--md); }
  .g2 { font-size: var(--sm); color: var(--dim); }
  .gn { font-size: var(--sm); color: var(--dim); }
  /* lyric hits */
  .lyr { display: flex; flex-direction: column; gap: 2px; padding-bottom: 10px; }
  .lh { display: flex; align-items: center; gap: 8px; padding: 2px 10px 6px; font-size: var(--md); font-weight: 600; }
  .hist { display: flex; flex-direction: column; gap: 4px; }
  .stage { display: grid; }
  .slot { grid-area: 1 / 1; min-width: 0; }
  .ph { flex: none; width: 44px; height: 44px; border-radius: 6px; background: rgba(255, 255, 255, .08); display: grid; place-items: center; }
  .ph.r { border-radius: 50%; }
  .x { flex: none; width: 32px; height: 32px; border-radius: 50%; display: grid; place-items: center; transition: background-color .12s; }
  .x:hover { background: rgba(255, 255, 255, .08); }
  .foot { display: flex; flex-direction: column; gap: 4px; }
  .spin { display: flex; align-items: center; gap: 8px; font-size: var(--sm); color: var(--dim); }
  .spin .mono { color: var(--text); }
  .part { display: flex; flex-direction: column; gap: 8px; }
  .pt { font-size: var(--sm); color: var(--dim); }
</style>
