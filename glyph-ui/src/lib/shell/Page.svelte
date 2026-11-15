<script lang="ts">
  // The page for `app.view` (ui/mod.rs main match). Every page switch fades
  // the new page in; the scroll frame fades out at its edges (fade_edges).
  import { fade } from "svelte/transition";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { fadeEdges, pageChrome } from "../ui.svelte";
  import ErrorCard from "../components/ErrorCard.svelte";
  import Home from "../pages/Home.svelte";
  import Search from "../pages/Search.svelte";
  import Library from "../pages/Library.svelte";
  import ListView from "../pages/ListView.svelte";
  import Artist from "../pages/Artist.svelte";
  import Profile from "../pages/Profile.svelte";
  import Section from "../pages/Section.svelte";
  import Releases from "../pages/Releases.svelte";
  import Similar from "../pages/Similar.svelte";
  import Stats from "../pages/Stats.svelte";
  import Local from "../pages/Local.svelte";
  import NowPlaying from "../pages/NowPlaying.svelte";

  const pages = { home: Home, search: Search, library: Library, list: ListView, artist: Artist, profile: Profile, section: Section, releases: Releases, similar: Similar, stats: Stats, local: Local, now_playing: NowPlaying };
  const key = $derived(JSON.stringify([app.view, app.openList && { ...app.openList, tracks: undefined }, app.openArtist, app.openProfile, app.section?.title]));
  const Current = $derived(pages[app.view]);
  let scroller: HTMLDivElement | undefined = $state();
  $effect(() => { void key; void app.libraryTab; scroller?.scrollTo(0, 0); });
  // A page that throws while drawing shows a card; the rest of the app keeps working.
  function report(e: unknown) { console.error("[page]", e); }
  function errText(e: unknown) { return e instanceof Error ? `${e.name}: ${e.message}
${e.stack ?? ""}`.trim() : String(e); }
</script>

{#snippet broken(e: unknown, reset: () => void)}
  <div class="broken">
    <ErrorCard compact title={t("error.page.title")} body={t("error.page.body")} detail={errText(e)} onretry={reset} onback={app.history.length ? () => app.back() : undefined} />
  </div>
{/snippet}

{#if app.view === "now_playing"}
  {#key key}<div class="fill" in:fade={{ duration: 200 }}><svelte:boundary onerror={report}><Current />{#snippet failed(e, reset)}{@render broken(e, reset)}{/snippet}</svelte:boundary></div>{/key}
{:else}
  <div class="frame">
    <div class="scroll page-scroll" bind:this={scroller} id="main-scroll" use:fadeEdges={{ top: pageChrome.stickyTop ? 0 : 28, bottom: 52, active: !app.compact }}>
      {#key key}<div class="pg" in:fade={{ duration: 200 }}><svelte:boundary onerror={report}><Current />{#snippet failed(e, reset)}{@render broken(e, reset)}{/snippet}</svelte:boundary></div>{/key}
    </div>
  </div>
{/if}

<style>
  .broken { min-height: 60vh; display: grid; place-items: center; padding: 24px; }
  .fill, .frame { position: relative; width: 100%; height: 100%; }
  .page-scroll { position: absolute; inset: 0; }
  .pg { min-height: 100%; padding: var(--page-top) var(--page-x) calc(var(--gap) * 3); }
</style>
