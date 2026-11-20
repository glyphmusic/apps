<script lang="ts">
  // views.rs home: page column gap 24 (greeting heading, home cards, then
  // sections with gap 10): Recently played (songs and artists, newest
  // first, 12), New from artists you follow, Recommended, Popular. "Show
  // all" opens the full list (Recent) or a Section page. Nothing played
  // yet: empty state. While the feeds load: ASCII skeleton cards.
  import SectionTitle from "../components/SectionTitle.svelte";
  import EmptyState from "../components/EmptyState.svelte";
  import SkeletonSwap from "../components/SkeletonSwap.svelte";
  import HomeCards from "./home/HomeCards.svelte";
  import HomeSection from "./home/HomeSection.svelte";
  import HScroll from "./home/HScroll.svelte";
  import SongCard from "./home/SongCard.svelte";
  import ArtistCard from "./home/ArtistCard.svelte";
  import ListCard from "./home/ListCard.svelte";
  import ReleaseCard from "./home/ReleaseCard.svelte";
  import CardSkeleton from "./home/CardSkeleton.svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { uniqBy } from "../fmt";
  import { listId, trackKey, type ListKey, type Track } from "../api/types";

  type HomeRecentEntry =
    | { kind: "song"; track: Track; key: string }
    | { kind: "artist"; name: string; track: Track | null; key: string }
    | { kind: "list"; list: ListKey; key: string };

  function validList(l: ListKey): boolean {
    if (l.kind === "recent") return false;
    if (l.kind === "playlist") return !!app.playlist(l.id);
    return true;
  }

  // Collapse songs played from a list/playlist into that list's card while
  // keeping standalone songs (search, home, radio) and artist entries visible.
  const recent = $derived.by<HomeRecentEntry[]>(() => {
    const out: HomeRecentEntry[] = [];
    const seenLists = new Set<string>();
    const seenArtists = new Set<string>();
    const seenSongs = new Set<string>();
    for (const r of app.library.recent) {
      if (r.kind === "list") {
        if (!validList(r.list)) continue;
        const id = listId(r.list);
        if (seenLists.has(id)) continue;
        seenLists.add(id);
        out.push({ kind: "list", list: r.list, key: `l:${id}` });
      } else if (r.kind === "artist") {
        const ak = r.name.trim().toLowerCase();
        if (!ak || seenArtists.has(ak)) continue;
        seenArtists.add(ak);
        out.push({ kind: "artist", name: r.name, track: r.track ?? null, key: `a:${ak}` });
      } else if (r.kind === "song") {
        if (r.from && validList(r.from)) {
          const id = listId(r.from);
          if (seenLists.has(id)) continue;
          seenLists.add(id);
          out.push({ kind: "list", list: r.from, key: `l:${id}` });
        } else {
          const sk = trackKey(r.track);
          if (seenSongs.has(sk)) continue;
          seenSongs.add(sk);
          out.push({ kind: "song", track: r.track, key: `s:${sk}` });
        }
      }
      if (out.length >= 12) break;
    }
    return out;
  });
  /** Recently played songs only (what a click on a song plays from). */
  const recentSongs = $derived(app.library.recent.flatMap((r) => (r.kind === "song" ? [r.track] : [])));
  // Settings > Playback > Content "Song suggestions" off: no Recommended row.
  const showRecs = $derived(app.config.ui.recommendations !== false);
  const recs = $derived(showRecs ? uniqBy(app.home.recs.filter((x) => app.allowed(x)), trackKey) : []);
  const trending = $derived(uniqBy(app.home.trending.filter((x) => app.allowed(x)), trackKey));
  const releases = $derived(uniqBy(app.home.new_releases, (r) => r.id));
  const loading = $derived(app.home.loading && !recs.length && !trending.length);

  function playRecent(tr: Track) {
    const k = trackKey(tr);
    app.play(recentSongs, Math.max(0, recentSongs.findIndex((x) => trackKey(x) === k)));
  }
  const showAll = (title: string, icon: string, tracks: typeof recs) => () => app.openSection({ title, icon, tracks });
</script>

<div class="home">
  <!-- Phones: the greeting sits in the compact header (views.rs TOUCH). -->
  {#if !app.compact}<SectionTitle heading title={t("home.greeting")} />{/if}
  <HomeCards />

  <HomeSection icon="disc-3" title={t("home.recent")} more={recent.length ? () => app.openListView({ kind: "recent" }) : undefined}>
    {#if !recent.length}
      <EmptyState title={t("home.empty.title")} sub={t("home.empty.sub")} />
    {:else}
      <HScroll>
        {#each recent as it, i (it.key)}
          {#if it.kind === "song"}
            <SongCard track={it.track} onplay={() => playRecent(it.track)} from={{ list: { kind: "recent" }, index: i }} />
          {:else if it.kind === "artist"}
            <ArtistCard name={it.name} track={it.track} />
          {:else if it.kind === "list"}
            <ListCard list={it.list} />
          {/if}
        {/each}
      </HScroll>
    {/if}
  </HomeSection>

  {#if releases.length}
    <HomeSection icon="sparkles" title={t("home.new_releases")}>
      <HScroll>{#each releases as r (r.id)}<ReleaseCard release={r} />{/each}</HScroll>
    </HomeSection>
  {/if}

  <SkeletonSwap {loading}>
    {#snippet skeleton()}
      <div class="feeds">
        {#if showRecs}
          <HomeSection icon="heart" title={t("home.recs")}>
            <HScroll>{#each Array(7) as _, i}<CardSkeleton id="home-recs" {i} />{/each}</HScroll>
          </HomeSection>
        {/if}
        <HomeSection icon="headphones" title={t("home.trending")}>
          <HScroll>{#each Array(7) as _, i}<CardSkeleton id="home-trend" {i} />{/each}</HScroll>
        </HomeSection>
      </div>
    {/snippet}
    <div class="feeds">
      {#if recs.length}
        <HomeSection icon="heart" title={t("home.recs")} more={showAll(t("home.recs"), "heart", recs)}>
          <HScroll>{#each recs as tr, i (trackKey(tr))}<SongCard track={tr} onplay={() => app.play(recs, i)} />{/each}</HScroll>
        </HomeSection>
      {/if}
      {#if trending.length}
        <HomeSection icon="headphones" title={t("home.trending")} more={showAll(t("home.trending"), "headphones", trending)}>
          <HScroll>{#each trending as tr, i (trackKey(tr))}<SongCard track={tr} onplay={() => app.play(trending, i)} />{/each}</HScroll>
        </HomeSection>
      {/if}
    </div>
  </SkeletonSwap>
</div>

<style>
  .home, .feeds { display: flex; flex-direction: column; gap: 24px; min-width: 0; }
  :global(html.compact) .home, :global(html.compact) .feeds { gap: 20px; }
</style>
