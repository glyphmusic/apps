<script lang="ts">
  // Phone layout (compact.rs, ui/mod.rs compact branches): header per tab
  // (picture + title), a back bar on inner pages (title + play fade in once
  // the big heading scrolled away), the page edge to edge, the mini player
  // and the tab bar at the bottom. Now Playing is a sheet; lyrics, queue,
  // Connect, the profile drawer and settings sit on top.
  import { fade } from "svelte/transition";
  import { onMount, untrack } from "svelte";
  import { app } from "../state.svelte";
  import { onWeb } from "../web";
  import { jam } from "../jam.svelte";
  import { press } from "../ui.svelte";
  import { t } from "../i18n.svelte";
  import { playingFrom } from "../pages/lists/actions";
  import PlayGlyph from "../components/PlayGlyph.svelte";
  import Header from "./Header.svelte";
  import IconAction from "./IconAction.svelte";
  import TabBar from "./TabBar.svelte";
  import MiniBar from "./MiniBar.svelte";
  import Drawer from "./Drawer.svelte";
  import PhoneLibrary from "./PhoneLibrary.svelte";
  import PhoneListView from "./PhoneListView.svelte";
  import PhoneSearch from "./PhoneSearch.svelte";
  import NowPlayingSheet from "./NowPlayingSheet.svelte";
  import LyricsView from "./LyricsView.svelte";
  import BottomSheet from "./BottomSheet.svelte";
  import SettingsScreen from "./SettingsScreen.svelte";
  import ShareSheet from "./ShareSheet.svelte";
  import Page from "../shell/Page.svelte";
  import { installCompactLongPress, phone } from "./cstate.svelte";

  onMount(() => installCompactLongPress());

  // Now Playing from shared code (shortcuts, menus) opens the sheet instead.
  $effect(() => {
    if (app.view === "now_playing") {
      app.back();
      if (app.player.current) phone.npOpen = true;
    }
  });
  $effect(() => { if (!app.player.current) { phone.npOpen = false; phone.lyricsOpen = false; } });
  $effect(() => { if (!app.settingsOpen) phone.settingsInPage = false; });
  // Leaving the phone layout (window made wide) closes the share menu.
  $effect(() => () => { app.shareSheet = null; });
  // Navigating away from the sheet ("Go to album" in its menu) closes it.
  let lastView = app.view;
  $effect(() => { const v = app.view; if (v !== lastView && v !== "now_playing") phone.npOpen = false; lastView = v; });

  const tabView = $derived(app.view === "home" || app.view === "search" || app.view === "library");
  const ownList = $derived(app.view === "list");
  let mainEl: HTMLElement | undefined = $state();
  let barEl: HTMLElement | undefined = $state();
  let scrollTop = $state(0);
  let barBgK = $state(0);
  let titleK = $state(0);
  let playK = $state(0);
  const viewKey = $derived(app.view + JSON.stringify(app.openList && { ...app.openList, tracks: undefined }) + app.openArtist + (app.section?.title ?? ""));
  const clamp01 = (v: number) => Math.min(1, Math.max(0, v));

  function syncBar(top = mainEl?.scrollTop ?? 0) {
    if (!mainEl || !barEl) {
      barBgK = 0; titleK = 0; playK = 0;
      return;
    }
    const cutoff = barEl.getBoundingClientRect().bottom;
    const titleEl = mainEl.querySelector<HTMLElement>(".lv .head h1, .ap .hero .name, .sv .head h1, .desk h1.hd");
    const playEl = mainEl.querySelector<HTMLElement>(".lv .head .play, .ap .mob-acts .rp, .sv .head .play");
    // Artist pages and lists with a big picture: the bar turns dark only when
    // the title comes up to it, so the photo / shrinking cover stays visible.
    barBgK = app.view === "artist" || ownList
      ? (titleEl ? clamp01((cutoff + 36 - titleEl.getBoundingClientRect().top) / 48) : clamp01((top - 180) / 48))
      : clamp01(top / 24);
    titleK = titleEl ? clamp01((cutoff - titleEl.getBoundingClientRect().bottom) / 24) : clamp01((top - 20) / 36);
    // Keep the bar play button hidden until the inline play button scrolls out of view.
    playK = playEl ? clamp01((cutoff - playEl.getBoundingClientRect().bottom) / 20) : 0;
  }

  $effect(() => {
    void viewKey;
    untrack(() => {
      scrollTop = 0;
      phone.scroll = 0;
      mainEl?.scrollTo(0, 0);
      syncBar(0);
    });
  });
  $effect(() => {
    void barTracks.length;
    void app.view;
    untrack(() => syncBar());
  });
  // One update per frame, read in one go: scroll events can come faster
  // than frames on Android, and each one used to restyle the page.
  let scrollRaf = 0;
  function onScroll() {
    if (scrollRaf) return;
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = 0;
      const top = mainEl?.scrollTop ?? 0;
      scrollTop = top;
      phone.scroll = top;
      syncBar(top);
    });
  }
  // The bar of the tab pages and plain pages turns dark as the page scrolls
  // under it; its status-bar part with it, so no lighter strip stays above.
  const topK = $derived(app.view === "home" || app.view === "search" || app.view === "library" ? clamp01(scrollTop / 24) : barBgK);
  onMount(() => {
    const onResize = () => syncBar();
    addEventListener("resize", onResize);
    return () => { removeEventListener("resize", onResize); cancelAnimationFrame(scrollRaf); };
  });
  function back() { if (!app.history.length) app.navigate("home"); else app.back(); }
  const barTitle = $derived(
    app.view === "list" && app.openList ? app.listTitle(app.openList)
    : app.view === "section" ? app.section?.title ?? ""
    : app.view === "artist" ? app.openArtist ?? ""
    : app.view === "releases" || app.view === "similar" ? `${app.openArtist} · ${t(app.view === "releases" ? "artist.albums" : "artist.similar")}`
    : "",
  );
  const artistMenuTrack = $derived(app.view === "artist" && app.openArtist ? (app.artists[app.openArtist.toLowerCase()]?.tracks[0] ?? null) : null);
  const barTracks = $derived.by(() => {
    if (app.view === "list" && app.openList) return app.listTracks(app.openList);
    if (app.view === "section" && app.section) return app.section.tracks;
    if (app.view === "artist" && app.openArtist) return app.artists[app.openArtist.toLowerCase()]?.tracks ?? [];
    return [];
  });
  const barPlayable = $derived(barTracks.some((x) => app.allowed(x)));
  const barHere = $derived.by(() => {
    if (!app.player.current) return false;
    if (app.view === "artist" && app.openArtist && app.player.current.artist.toLowerCase() === app.openArtist.toLowerCase()) return true;
    return playingFrom(barTracks);
  });
  const barPlaying = $derived(barHere && app.playing);
  const barLoading = $derived(barHere && app.loading);
  function playBar() {
    if (barHere) return app.togglePlay();
    const i = barTracks.findIndex((x) => app.allowed(x));
    if (i < 0) return;
    const ctx = app.view === "list" && app.openList ? app.listTitle(app.openList) : app.view === "section" ? app.section?.title : app.openArtist ?? undefined;
    app.play(barTracks, i, ctx);
  }
  const connectActive = $derived(app.devices.some((d) => d.active && d.kind !== "this") || app.jamOpen || jam.state != null);
</script>

{#snippet barPlayBtn()}
  {#if barPlayable}
    <button
      class="bar-play"
      class:nudge={!barPlaying && !barLoading}
      use:press
      style:opacity={playK}
      style:pointer-events={playK > 0.2 ? "auto" : "none"}
      tabindex={playK > 0.2 ? 0 : -1}
      onclick={playBar}
      aria-label={t(barPlaying ? "common.pause" : "common.play")}
    >
      <PlayGlyph loading={barLoading} playing={barPlaying} size={15} button={34} />
    </button>
  {/if}
{/snippet}

<div class="phone" data-skin class:under={phone.npOpen} class:fading={!phone.npOpen || phone.npPull === 0} style:opacity={phone.npOpen ? Math.min(1, phone.npPull / 140) : 1}>
  {#if app.view !== "artist" && !ownList}
  <div class="top" style:background="rgba(10, 10, 10, {topK})">
  {#if app.view === "home"}
    <Header title={t("home.greeting")}>
      {#snippet actions()}
        <IconAction name="monitor-speaker" label={t("connect.title")} color={connectActive ? "var(--accent)" : undefined} onclick={() => { app.showQueue = false; app.showDevices = true; }} />
      {/snippet}
    </Header>
  {:else if app.view === "search"}<PhoneSearch part="bar" />
  {:else if app.view === "library"}
    <Header title={t("nav.library")}>
      {#snippet actions()}
        <IconAction name="search" onclick={() => app.navigate("search")} />
        <IconAction name="plus" onclick={() => (app.nameDialog = { tracks: [] })} />
      {/snippet}
    </Header>
  {:else}
    <div class="backbar" bind:this={barEl}>
      <IconAction name="arrow-left" onclick={back} />
      <span class="bt trunc" style:opacity={titleK}>{barTitle}</span>
      {@render barPlayBtn()}
    </div>
  {/if}
  </div>
  {:else if app.view === "artist"}
    <div class="backbar over artist" bind:this={barEl} style:background="rgba(10, 10, 10, {barBgK})">
      <span class="pill-ic" style:--bar-k={barBgK}>
        <IconAction name="arrow-left" onclick={back} />
      </span>
      <span class="bt trunc" style:opacity={titleK}>{barTitle}</span>
      {@render barPlayBtn()}
      <span class="pill-ic" style:--bar-k={barBgK}>
        <IconAction name="ellipsis-vertical" onclick={(e) => app.openCtx({ x: e.clientX, y: e.clientY, track: artistMenuTrack, artist: app.openArtist })} />
      </span>
    </div>
  {:else}
    <div class="backbar over" bind:this={barEl} style:background="rgba(10, 10, 10, {barBgK})">
      <IconAction name="arrow-left" onclick={back} />
      <span class="bt trunc" style:opacity={titleK}>{barTitle}</span>
      {@render barPlayBtn()}
      {#if app.openList}
        <IconAction name="ellipsis-vertical" label={t("ctx.more")} onclick={(e) => app.openCtx({ x: e.clientX, y: e.clientY, track: barTracks[0] ?? null, list: app.openList! })} />
      {/if}
    </div>
  {/if}

  <main class="main scroll" id="main-scroll" bind:this={mainEl} onscroll={onScroll}>
    {#key viewKey}
      <div in:fade={{ duration: 180 }}>
        {#if app.view === "library"}<PhoneLibrary />
        {:else if ownList}<PhoneListView />
        {:else if app.view === "search" && !app.searchQuery.trim()}<div class="pad"><PhoneSearch part="recent" /></div>
        {:else}<div class="desk"><Page /></div>{/if}
      </div>
    {/key}
    <div class="tail" class:with-mini={!!app.player.current}></div>
  </main>

  <div class="bottom" class:has-mini={!!app.player.current}>
    {#if app.player.current}<MiniBar />{/if}
    <TabBar />
  </div>
</div>

{#if phone.npOpen && app.player.current}<NowPlayingSheet />{/if}
{#if phone.lyricsOpen && app.player.current}<LyricsView />{/if}
{#if app.showQueue || app.showDevices}<BottomSheet />{/if}
{#if phone.drawerOpen}<Drawer />{/if}
{#if app.settingsOpen}<SettingsScreen />{/if}
{#if app.shareSheet}<ShareSheet />{/if}

<style>
  /* Pages run up under the status bar: the top bar holds the status-bar space
     and takes its colour, artist photos and list covers start at the very top. */
  .phone { position: relative; z-index: 1; height: 100%; display: flex; flex-direction: column; }
  .top { flex: none; position: relative; z-index: 6; padding-top: var(--safe-top, env(safe-area-inset-top)); }
  .phone.fading { transition: opacity .22s var(--ease); }
  .phone.under { pointer-events: none; }
  .phone::before { content: ""; position: fixed; inset: 0; z-index: -1; background: rgba(0, 0, 0, .5); pointer-events: none; }
  .backbar { flex: none; height: 52px; padding: 0 16px 0 4px; display: flex; align-items: center; gap: 4px; }
  .backbar.over { position: absolute; top: 0; left: 0; right: 0; z-index: 6; height: calc(52px + var(--safe-top, env(safe-area-inset-top))); padding-top: var(--safe-top, env(safe-area-inset-top)); pointer-events: none; }
  .backbar.over > :global(*) { pointer-events: auto; }
  .backbar.over:not(.artist) { padding-left: 4px; padding-right: 4px; }
  /* Starts at the very top, so once it turns dark on scroll it covers the status bar too. */
  .backbar.over.artist { top: 0; height: calc(54px + var(--safe-top, env(safe-area-inset-top))); padding: var(--safe-top, env(safe-area-inset-top)) 12px 0; gap: 8px; }
  .pill-ic { display: grid; place-items: center; width: 38px; height: 38px; border-radius: 50%; background: color-mix(in srgb, #141414, transparent calc(var(--bar-k, 0) * 100%)); }
  .pill-ic > :global(*) { width: 38px; height: 38px; }
  .bt { flex: 1; font-size: 16px; font-weight: 700; color: var(--text); }
  .bar-play { flex: none; width: 34px; height: 34px; border-radius: 50%; background: var(--accent); display: grid; place-items: center; transition: transform .12s; }
  .bar-play.nudge :global(.ic) { margin-left: 1.5px; }
  .bar-play:global(.pressing) { animation: press .25s var(--ease); }
  /* The page runs on under the tab bar and mini player, so it stays visible
     there; the tail lets the last row scroll up above them. */
  /* --page: the scroll timeline the cover shrink and photo zoom follow. */
  .main { flex: 1; min-height: 0; scroll-timeline: --page block; }
  .pad { padding: 0 16px; }
  /* Desktop pages inside the phone frame: the frame scrolls, not the page. */
  .desk :global(.frame), .desk :global(.page-scroll) { position: static; height: auto; overflow: visible; }
  .desk :global(.page-scroll) { -webkit-mask-image: none !important; mask-image: none !important; }
  .tail { height: calc(78px + var(--safe-bottom, env(safe-area-inset-bottom))); }
  .tail.with-mini { height: calc(150px + var(--safe-bottom, env(safe-area-inset-bottom))); }
  .bottom { position: absolute; left: 0; right: 0; bottom: 0; z-index: 5; padding: 6px 10px calc(6px + var(--safe-bottom, env(safe-area-inset-bottom))); display: flex; flex-direction: column; gap: 8px; pointer-events: none; background: linear-gradient(to bottom, rgba(10, 10, 10, 0), rgba(10, 10, 10, .55) 60%, rgba(10, 10, 10, .75)); }
  .bottom > :global(*) { pointer-events: auto; }
</style>
