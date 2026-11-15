<script lang="ts">
  // views.rs home: page column gap 24 (greeting heading, home cards, then
  // sections with gap 10): Recently played (songs and artists, newest
  // first, 12), New from artists you follow, Recommended, Popular. "Show
  // all" opens the full list (Recent) or a Section page. Nothing played
  // yet: empty state. While the feeds load: ASCII skeleton cards.
  import SectionTitle from "../components/SectionTitle.svelte";
  import EmptyState from "../components/EmptyState.svelte";
  import HomeCards from "./home/HomeCards.svelte";
  import HomeSection from "./home/HomeSection.svelte";
  import HScroll from "./home/HScroll.svelte";
  import SongCard from "./home/SongCard.svelte";
  import ArtistCard from "./home/ArtistCard.svelte";
  import ReleaseCard from "./home/ReleaseCard.svelte";
  import CardSkeleton from "./home/CardSkeleton.svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { trackKey } from "../api/types";

  const recent = $derived(app.library.recent.slice(0, 12));
  /** Recently played songs only (what a click on a song plays from). */
  const recentSongs = $derived(app.library.recent.flatMap((r) => (r.kind === "song" ? [r.track] : [])));
  const recs = $derived(app.home.recs.filter((x) => app.allowed(x)));
  const trending = $derived(app.home.trending.filter((x) => app.allowed(x)));
  const releases = $derived(app.home.new_releases);
  const loading = $derived(app.home.loading && !recs.length && !trending.length);

  function playRecent(i: number) {
    const it = recent[i];
    if (it?.kind === "artist") return app.openArtistPage(it.name);
    if (it?.kind !== "song") return;
    const k = trackKey(it.track);
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
        {#each recent as it, i (i + (it.kind === "song" ? trackKey(it.track) : it.kind === "artist" ? it.name : "l"))}
          {#if it.kind === "song"}
            <SongCard track={it.track} onplay={() => playRecent(i)} from={{ list: { kind: "recent" }, index: i }} />
          {:else if it.kind === "artist"}
            <ArtistCard name={it.name} track={it.track ?? null} />
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

  {#if loading}
    <HomeSection icon="heart" title={t("home.recs")}>
      <HScroll>{#each Array(7) as _, i}<CardSkeleton id="home-recs" {i} />{/each}</HScroll>
    </HomeSection>
    <HomeSection icon="headphones" title={t("home.trending")}>
      <HScroll>{#each Array(7) as _, i}<CardSkeleton id="home-trend" {i} />{/each}</HScroll>
    </HomeSection>
  {:else}
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
  {/if}
</div>

<style>
  .home { display: flex; flex-direction: column; gap: 24px; min-width: 0; }
  :global(html.compact) .home { gap: 20px; }
</style>
