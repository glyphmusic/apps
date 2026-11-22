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
  import MixCard from "./home/MixCard.svelte";
  import CardSkeleton from "./home/CardSkeleton.svelte";
  import Icon from "../components/Icon.svelte";
  import { app } from "../state.svelte";
  import { onWeb } from "../web";
  import { dismissInstallBanner, openInstallSheet, pwa, triggerNativeInstall } from "../pwa.svelte";
  import { t } from "../i18n.svelte";
  import { uniqBy } from "../fmt";
  import { listId, trackKey, type ListKey, type Track } from "../api/types";
  import { slide } from "svelte/transition";

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
  const tasteOn = $derived(app.config.taste?.enabled !== false);
  const recs = $derived(showRecs ? uniqBy(app.home.recs.filter((x) => app.allowed(x)), trackKey) : []);
  const mixes = $derived(showRecs && tasteOn ? (app.home.mixes ?? []).filter((m) => m.tracks.length > 0) : []);
  const trending = $derived(uniqBy(app.home.trending.filter((x) => app.allowed(x)), trackKey));
  const releases = $derived(uniqBy(app.home.new_releases, (r) => r.id));
  const loading = $derived(app.home.loading && !recs.length && !trending.length && !mixes.length);

  function playRecent(tr: Track) {
    const k = trackKey(tr);
    app.play(recentSongs, Math.max(0, recentSongs.findIndex((x) => trackKey(x) === k)));
  }
  const showAll = (title: string, icon: string, tracks: typeof recs) => () => app.openSection({ title, icon, tracks });
  const showInstallBanner = $derived(onWeb() && app.compact && !pwa.installed && !pwa.bannerDismissed);
  async function installFromBanner() {
    if (pwa.canPrompt) {
      const res = await triggerNativeInstall();
      if (res === "accepted") { app.toast(t("web.pwa.installed_toast")); return; }
      if (res === "dismissed") return;
    }
    openInstallSheet();
  }
</script>

<div class="home">
  <!-- Phones: the greeting sits in the compact header (views.rs TOUCH). -->
  {#if !app.compact}<SectionTitle heading title={t("home.greeting")} />{/if}
  {#if showInstallBanner}
    <div class="pwa-banner" transition:slide={{ duration: 200 }}>
      <img class="pwa-ic" src="{import.meta.env.BASE_URL}apple-touch-icon.png" alt="Glyph" width="40" height="40" />
      <div class="pwa-tx">
        <b class="trunc">{t(pwa.ios ? "web.pwa.banner.title_ios" : "web.pwa.banner.title")}</b>
        <span class="trunc">{t(pwa.ios ? "web.pwa.banner.sub_ios" : "web.pwa.banner.sub")}</span>
      </div>
      <button class="pwa-go" onclick={installFromBanner}>{t(pwa.ios ? "web.pwa.banner.add" : "web.pwa.banner.install")}</button>
      <button class="pwa-x" onclick={dismissInstallBanner} aria-label={t("tip.x")}><Icon name="x" size={15} /></button>
    </div>
  {/if}
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
      {#if mixes.length}
        <HomeSection icon="sparkles" title={t("taste.home.mixes")} more={() => app.navigate("taste")} moreLabel={t("taste.title")}>
          <HScroll>{#each mixes as mix (mix.id)}<MixCard {mix} />{/each}</HScroll>
        </HomeSection>
      {/if}
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
  .pwa-banner {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 10px 10px 10px 12px;
    border-radius: 12px;
    background: rgba(255, 255, 255, .07);
  }
  .pwa-ic { flex: none; width: 40px; height: 40px; border-radius: 10px; background: #050505; }
  .pwa-tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .pwa-tx b { font-size: 13.5px; font-weight: 600; color: #fff; }
  .pwa-tx span { font-size: 12px; color: var(--dim); }
  .pwa-go {
    flex: none;
    height: 32px;
    padding: 0 13px;
    border-radius: 999px;
    background: var(--accent);
    color: #000;
    font-size: 12.5px;
    font-weight: 600;
    white-space: nowrap;
    transition: opacity .12s, transform .14s var(--ease);
  }
  .pwa-go:active { opacity: .85; transform: scale(.95); }
  .pwa-x {
    flex: none;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    color: var(--faint);
  }
  .pwa-x:active { background: rgba(255, 255, 255, .1); color: #fff; }
</style>
