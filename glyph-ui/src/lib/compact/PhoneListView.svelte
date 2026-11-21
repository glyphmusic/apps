<script lang="ts">
  // compact.rs list_view: big cover (62 % of the width, 140..280) centred,
  // title 24 bold, "n songs" 13 dim, actions (download, shuffle, 52 px
  // accent play), then the song rows (60 px pitch). Above the default resting
  // scroll threshold sits a pull-to-reveal bar with inline search + sort.
  import { tick, untrack } from "svelte";
  import ListCover from "../components/ListCover.svelte";
  import PlayGlyph from "../components/PlayGlyph.svelte";
  import IconAction from "./IconAction.svelte";
  import Icon from "../components/Icon.svelte";
  import DownloadBadge from "../components/DownloadBadge.svelte";
  import PhoneSongRow from "./PhoneSongRow.svelte";
  import VirtualRows from "../components/VirtualRows.svelte";
  import SkeletonSwap from "../components/SkeletonSwap.svelte";
  import Skeletons from "../pages/search/Skeletons.svelte";
  import { app } from "../state.svelte";
  import { onWeb } from "../web";
  import { t, tf } from "../i18n.svelte";
  import { listId, trackKey, type Track } from "../api/types";
  import { albumSkeletonCount, downloadsView, listSort, openNameDialog, removeDownloads, shownTracks, sortId } from "../pages/lists/actions";
  import { cachedAlbumKind } from "../pages/lists/albumMeta.svelte";
  import { phone, scrollFx } from "./cstate.svelte";
  import { REVEAL_BAR_H, REVEAL_TOP_BAR, revealHeight, revealOpacity, revealSettleScroll, revealSnapTarget } from "./list-reveal";

  const readMainH = () => (typeof document !== "undefined" ? document.getElementById("main-scroll")?.clientHeight || 0 : 0);

  let w = $state(390);
  let mainH = $state(readMainH() || 760);
  let filterQ = $state("");
  let searchFocused = $state(false);
  let searchInput: HTMLInputElement | undefined = $state();
  let lvEl: HTMLDivElement | undefined = $state();

  const l = $derived(app.openList!);
  const lKey = $derived(listId(l));
  const tracks = $derived(shownTracks(l));
  const q = $derived(filterQ.trim().toLowerCase());
  const visibleTracks = $derived<Track[]>(
    q
      ? tracks.filter((tr) => tr.title.toLowerCase().includes(q) || tr.artist.toLowerCase().includes(q) || (tr.album ?? "").toLowerCase().includes(q))
      : tracks,
  );
  const albumLoading = $derived(l.kind === "album" && !!l.loading && !tracks.length);
  const skCount = $derived(albumSkeletonCount(l, l.kind === "album" ? cachedAlbumKind(l.title, l.artist) : undefined));
  const art = $derived(Math.min(280, Math.max(140, w * 0.62)));

  // Select mode (Liked Songs, own playlists): tap rows, then remove them together.
  const pickable = $derived(app.canPick(l));
  const pickMode = $derived(app.isPicking(l));
  const pickedN = $derived(pickMode ? tracks.filter((x) => app.isPicked(x)).length : 0);
  const allPicked = $derived(pickMode && pickedN === tracks.length && tracks.length > 0);
  const noArt = $derived(l.kind === "liked" || l.kind === "artist_liked" || l.kind === "recent" || l.kind === "downloads");

  // Pull-to-reveal search + sort bar above the resting threshold.
  const hasRevealBar = $derived(!pickMode && !albumLoading && (tracks.length > 0 || q.length > 0));
  const revealH = $derived(revealHeight(hasRevealBar, noArt));
  const canSort = $derived(!!sortId(l));
  const sort = $derived(listSort(l));
  const keepOpen = $derived(searchFocused || q.length > 0 || app.controlMenu?.menu === "sort");
  let settledKey = $state("");
  let settledRevealH = $state(0);
  const revealSettled = $derived(settledKey === lKey && settledRevealH === revealH);
  const barOpacity = $derived(keepOpen ? 1 : !revealSettled ? 0 : revealOpacity(phone.scroll, revealH));

  /** Cover shrink starts only once scrolled past the resting threshold (`revealH`). */
  const effScroll = $derived(Math.max(0, phone.scroll - revealH));
  const sk = $derived(Math.min(1, effScroll / Math.max(1, art)));
  const coverFx = $derived.by(() => {
    if (sk <= 0) return { transform: undefined as string | undefined, opacity: 1 };
    const s = 1 - 0.5 * sk;
    return { transform: `translateY(${(art * 0.5 * sk).toFixed(1)}px) scale(${s.toFixed(4)})`, opacity: Math.min(1, Math.max(0, (1 - sk) * 2)) };
  });

  $effect.pre(() => {
    phone.listRevealH = revealH;
  });
  $effect(() => () => {
    phone.listRevealH = 0;
  });

  // Reset filter & place the scroll container at the resting threshold on list
  // open or whenever the reveal bar appears/changes height (e.g. async track load).
  $effect(() => {
    const key = lKey;
    const targetH = revealH;
    const prevKey = untrack(() => settledKey);
    const prevH = untrack(() => settledRevealH);
    const isNewList = key !== prevKey;
    if (isNewList) {
      untrack(() => {
        filterQ = "";
        searchFocused = false;
      });
    }
    const sc = document.getElementById("main-scroll");
    if (!sc) {
      settledKey = key;
      settledRevealH = targetH;
      return;
    }
    const nextScroll = revealSettleScroll({
      isNewList,
      prevRevealH: prevH,
      nextRevealH: targetH,
      currentScrollTop: sc.scrollTop,
    });
    if (nextScroll == null) return;

    let cancelled = false;
    let raf = 0;
    const finish = () => {
      settledKey = key;
      settledRevealH = targetH;
    };
    const apply = (): boolean => {
      if (cancelled) return true;
      const ch = sc.clientHeight;
      if (ch && ch !== untrack(() => mainH)) {
        mainH = ch;
        if (lvEl && targetH > 0) {
          lvEl.style.minHeight = `${Math.max(0, Math.ceil(ch) - untrack(() => tailOffset) + targetH + 1)}px`;
        }
      }
      sc.scrollTop = nextScroll;
      phone.scroll = sc.scrollTop;
      return targetH <= 0 || sc.scrollTop >= targetH - 1;
    };

    if (apply()) {
      finish();
      return;
    }

    const onUserScroll = () => {
      cancelled = true;
      cancelAnimationFrame(raf);
      raf = 0;
      finish();
      detach();
    };
    const detach = () => {
      sc.removeEventListener("touchstart", onUserScroll);
      sc.removeEventListener("wheel", onUserScroll);
      sc.removeEventListener("pointerdown", onUserScroll);
    };
    sc.addEventListener("touchstart", onUserScroll, { passive: true });
    sc.addEventListener("wheel", onUserScroll, { passive: true });
    sc.addEventListener("pointerdown", onUserScroll, { passive: true });

    void tick().then(() => {
      if (cancelled) return;
      if (apply()) {
        detach();
        finish();
        return;
      }
      raf = requestAnimationFrame(() => {
        raf = 0;
        if (cancelled) return;
        apply();
        detach();
        finish();
      });
    });

    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
      detach();
    };
  });

  $effect(() => {
    void lKey;
    return () => app.stopPicking();
  });

  // Threshold snap ("Schwellenpunkt") on touch release or scroll settle.
  $effect(() => {
    const sc = document.getElementById("main-scroll");
    if (!sc) return;
    const syncH = () => { mainH = sc.clientHeight || mainH; };
    syncH();
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(syncH) : null;
    ro?.observe(sc);

    let touching = false;
    let startedDeep = false;
    let settleTimer = 0;
    let deepResetTimer = 0;

    const snap = () => {
      clearTimeout(settleTimer);
      settleTimer = 0;
      if (touching || !revealSettled) return;
      const target = revealSnapTarget({
        scrollTop: sc.scrollTop,
        revealH,
        keepOpen,
        startedDeep,
      });
      startedDeep = false;
      if (target != null) {
        sc.scrollTo({ top: target, behavior: "smooth" });
      }
    };

    const onStart = () => {
      touching = true;
      clearTimeout(settleTimer);
      clearTimeout(deepResetTimer);
      startedDeep = revealH > 0 && sc.scrollTop > revealH + 80;
    };
    const onEnd = () => {
      if (!touching) return;
      touching = false;
      clearTimeout(settleTimer);
      settleTimer = window.setTimeout(snap, 90);
    };
    const onScroll = () => {
      if (touching) return;
      if (!startedDeep && revealH > 0 && sc.scrollTop > revealH + 80) {
        startedDeep = true;
      }
      clearTimeout(deepResetTimer);
      deepResetTimer = window.setTimeout(() => { startedDeep = false; }, 240);
      clearTimeout(settleTimer);
      settleTimer = window.setTimeout(snap, 140);
    };

    sc.addEventListener("touchstart", onStart, { passive: true });
    sc.addEventListener("touchend", onEnd, { passive: true });
    sc.addEventListener("touchcancel", onEnd, { passive: true });
    sc.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      clearTimeout(settleTimer);
      clearTimeout(deepResetTimer);
      ro?.disconnect();
      sc.removeEventListener("touchstart", onStart);
      sc.removeEventListener("touchend", onEnd);
      sc.removeEventListener("touchcancel", onEnd);
      sc.removeEventListener("scroll", onScroll);
    };
  });

  function openSortMenu(e: MouseEvent) {
    e.stopPropagation();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    app.ctx = null;
    app.controlMenu = { menu: "sort", x: r.right, y: r.bottom + 6 };
  }

  function clearSearch() {
    filterQ = "";
    searchInput?.focus();
  }

  const here = $derived(app.player.current ? visibleTracks.some((x) => trackKey(x) === app.currentKey) : false);
  const dl = $derived(downloadsView(tracks));
  async function play() {
    if (here) return app.togglePlay();
    const i = visibleTracks.findIndex((x) => app.allowed(x));
    if (i >= 0) {
      if (app.player.shuffle) await app.setShuffle(false);
      app.play(visibleTracks, i, app.listTitle(l));
    }
  }
  function shuffle() {
    if (!visibleTracks.length) return;
    app.setShuffle(true);
    app.play(visibleTracks, Math.floor(Math.random() * visibleTracks.length), app.listTitle(l));
  }
  const emptySub = $derived(
    l.kind === "liked" || l.kind === "artist_liked" ? t("library.liked.empty.sub")
    : l.kind === "recent" ? t("library.recent.empty.sub")
    : l.kind === "downloads" ? t("library.downloads.empty.sub")
    : t("mobile.list.empty"),
  );
  const tailOffset = $derived(app.player.current ? 150 : 78);
</script>

<div
  class="lv"
  bind:this={lvEl}
  bind:clientWidth={w}
  style:min-height={revealH > 0 ? `${Math.max(0, Math.ceil(mainH) - tailOffset + revealH + 1)}px` : undefined}
>
  {#if hasRevealBar}
    <div
      class="reveal-slot"
      style:height="{REVEAL_TOP_BAR + REVEAL_BAR_H}px"
      style:padding-top="{REVEAL_TOP_BAR}px"
    >
      <div
        class="reveal-bar"
        style:opacity={barOpacity}
        style:pointer-events={barOpacity > 0.15 ? "auto" : "none"}
      >
        <div class="msearch" class:focused={searchFocused}>
          <Icon name="search" size={15} color="var(--dim)" />
          <input
            bind:this={searchInput}
            bind:value={filterQ}
            placeholder={t("add.search")}
            spellcheck="false"
            onfocus={() => {
              searchFocused = true;
              const sc = document.getElementById("main-scroll");
              if (sc && sc.scrollTop > 0) sc.scrollTo({ top: 0, behavior: "smooth" });
            }}
            onblur={() => (searchFocused = false)}
          />
          {#if filterQ}
            <button type="button" class="mclr" aria-label={t("tip.x")} onpointerdown={(e) => e.preventDefault()} onclick={clearSearch}>
              <Icon name="x" size={13} color="var(--dim)" />
            </button>
          {/if}
        </div>
        {#if canSort}
          <button
            type="button"
            class="msort"
            class:on={sort !== "custom"}
            aria-label={t("sort.title")}
            onclick={openSortMenu}
          >
            <Icon name="arrow-up-down" size={15} />
            <span>{t("sort.title")}</span>
          </button>
        {/if}
      </div>
    </div>
  {/if}

  <div class="head" class:no-art={noArt} class:has-reveal={hasRevealBar}>
    {#if !noArt}
      {#snippet cover()}<span class="cv" class:sd={scrollFx} style:--art={art} style:--rev={revealH} style:transform={scrollFx ? undefined : coverFx.transform} style:opacity={scrollFx ? undefined : coverFx.opacity}><ListCover list={l} size={art} /></span>{/snippet}
      {#if l.kind === "playlist"}
        <button class="art edit" onclick={() => openNameDialog([], l.id)} aria-label={t("list.edit")}>{@render cover()}</button>
      {:else}
        <div class="art">{@render cover()}</div>
      {/if}
    {/if}
    <h1>{app.listTitle(l)}</h1>
    <div class="n">
      {#if albumLoading && l.kind === "album"}
        {l.artist}
      {:else}
        {tf("common.n_songs", { n: tracks.length })}{#if l.kind === "artist_liked"} · <button class="link" onclick={() => app.openArtistPage(l.artist)}>{l.artist}</button>{/if}
      {/if}
    </div>
    {#if !pickMode}
    <div class="acts">
      {#if tracks.length && !onWeb()}
        {#if dl.state === "busy"}
          <button type="button" class="dl-act" aria-label={t("ctx.cancel_downloads")} onclick={() => app.cancelDownloads(tracks)}>
            <DownloadBadge state="busy" progress={dl.progress} size={22} arrow="x" />
          </button>
        {:else if l.kind !== "downloads"}
          {#if dl.state === "done"}
            <button type="button" class="dl-act" aria-label={t("ctx.remove_downloads")} onclick={() => removeDownloads(tracks)}>
              <DownloadBadge state="done" size={20} />
            </button>
          {:else}
            <IconAction name="arrow-down-to-line" label={t("tip.download")} onclick={() => app.download(tracks)} />
          {/if}
        {/if}
      {/if}
      {#if l.kind === "playlist"}<IconAction name="pencil" label={t("list.edit")} onclick={() => openNameDialog([], l.id)} />{/if}
      {#if pickable && tracks.length}<IconAction name="circle-check" label={t("pick.select")} onclick={() => app.startPicking(l)} />{/if}
      <span class="flex1"></span>
      {#if visibleTracks.length}
        <IconAction name="shuffle" label={t("tip.shuffle")} onclick={shuffle} />
        <button class="play" onclick={play} aria-label={t("common.play")}><PlayGlyph loading={here && app.loading} playing={here && app.playing} size={20} button={52} /></button>
      {/if}
    </div>
    {/if}
  </div>
  {#if pickMode}
    <div class="picks">
      <div class="pickbar" role="toolbar">
        <button class="pk" onclick={() => app.stopPicking()}>{t("pick.done")}</button>
        <span class="pick-n">{tf("pick.n", { n: pickedN })}</span>
        <button class="pk danger" disabled={!pickedN} onclick={() => app.removePicked(l)} aria-label={t("pick.remove")}><Icon name="trash-2" size={18} />{t("pick.remove")}</button>
      </div>
    </div>
    <div class="pick-more">
      <button class="chip" onclick={() => app.pickAll(tracks)}><Icon name={allPicked ? "circle" : "circle-check"} size={15} />{t(allPicked ? "pick.none" : "pick.all")}</button>
      {#if l.kind === "liked"}<button class="chip" onclick={() => app.clearLiked()}><Icon name="trash-2" size={15} />{t("liked.clear")}</button>{/if}
    </div>
  {/if}
  <SkeletonSwap loading={albumLoading}>
    {#snippet skeleton()}
      <div class="rows"><Skeletons kind="rows" n={skCount} prefix="album" /></div>
    {/snippet}
    {#if !tracks.length}
      <p class="empty">{emptySub}</p>
    {:else if !visibleTracks.length && q}
      <div class="fempty">
        <p class="empty">{t("search.none.title")}</p>
        <button type="button" class="chip" onclick={() => (filterQ = "")}>{t("lyrics.timing.reset")}</button>
      </div>
    {:else}
      <div class="rows">
        <VirtualRows count={visibleTracks.length} pitch={60} overscan={12}>
          {#snippet row(i)}
            <PhoneSongRow tracks={visibleTracks} index={i} from={l.kind === "album" ? null : l} context={app.listTitle(l)} />
          {/snippet}
        </VirtualRows>
      </div>
    {/if}
  </SkeletonSwap>
  <div style:height="24px"></div>
</div>

<style>
  /* Starts under the status bar (the bar above is see-through until scrolled). */
  .lv { display: flex; flex-direction: column; gap: 8px; padding-top: var(--safe-top, env(safe-area-inset-top)); overflow-anchor: none; }
  .reveal-slot { flex: none; margin-bottom: -8px; display: flex; align-items: flex-end; }
  .reveal-bar { width: 100%; height: 46px; padding: 4px 16px 6px; display: flex; align-items: center; gap: 8px; transition: opacity .08s linear; }
  .msearch {
    flex: 1; min-width: 0; height: 36px; padding: 0 8px 0 11px; border-radius: 10px;
    background: rgba(255, 255, 255, .08); display: flex; align-items: center; gap: 8px;
    transition: background-color .14s ease-out;
  }
  .msearch.focused { background: rgba(255, 255, 255, .13); }
  .msearch input { flex: 1; min-width: 0; height: 100%; background: none; border: 0; padding: 0; outline: none; font-size: 14px; color: var(--text); }
  .msearch input::placeholder { color: var(--faint); }
  .mclr { flex: none; width: 24px; height: 24px; border-radius: 6px; display: grid; place-items: center; }
  .mclr:active { background: rgba(255, 255, 255, .12); }
  .msort {
    flex: none; height: 36px; padding: 0 12px; border-radius: 10px;
    background: rgba(255, 255, 255, .08); display: inline-flex; align-items: center; gap: 6px;
    font-size: 13px; font-weight: 500; color: var(--text); white-space: nowrap;
    transition: background-color .14s ease-out, color .14s ease-out;
  }
  .msort:active { background: rgba(255, 255, 255, .14); }
  .msort.on { color: var(--accent); background: rgba(var(--accent-rgb), .14); }
  .head { display: flex; flex-direction: column; gap: 4px; padding: 0 16px; }
  .head.no-art { padding-top: 56px; }
  .head.no-art.has-reveal { padding-top: 8px; }
  .art { display: flex; justify-content: center; padding: 14px 0 16px; }
  .art.edit { align-self: center; transition: transform .14s var(--ease); }
  .art.edit:active { transform: scale(.98); }
  .cv { display: block; transform-origin: 50% 0; will-change: transform, opacity; }
  /* Same effect as coverFx, run by the browser along the page scroll past --rev. */
  .cv.sd { animation: cv-sink linear both; animation-timeline: --page; animation-range: calc(var(--rev, 0) * 1px) calc((var(--rev, 0) + var(--art)) * 1px); }
  @keyframes cv-sink {
    0% { transform: none; opacity: 1; }
    50% { opacity: 1; }
    100% { transform: translateY(calc(var(--art) * .5px)) scale(.5); opacity: 0; }
  }
  h1 { margin: 0; font-size: 24px; font-weight: 700; color: var(--text); }
  .n { font-size: 13px; color: var(--dim); }
  .link:hover { color: var(--text); text-decoration: underline; }
  .acts { margin-top: 8px; display: flex; align-items: center; gap: 4px; }
  .dl-act { flex: none; width: 40px; height: 40px; display: grid; place-items: center; border-radius: 50%; transition: background-color .12s; }
  .dl-act:active { background: rgba(255, 255, 255, .08); }
  .play { flex: none; width: 52px; height: 52px; border-radius: 50%; background: var(--accent); display: grid; place-items: center; }
  /* Stays under the top bar while the rows scroll. */
  .picks { position: sticky; top: calc(52px + var(--safe-top, env(safe-area-inset-top))); z-index: 3; padding: 0 16px; display: flex; flex-direction: column; }
  .pickbar { margin-top: 4px; height: 52px; padding: 0 6px; display: flex; align-items: center; gap: 4px; border-radius: 14px; background: #161618; }
  .pick-n { flex: 1; min-width: 0; padding: 0 6px; font-size: 15px; font-weight: 600; color: var(--text); white-space: nowrap; }
  .pk { flex: none; height: 40px; padding: 0 12px; display: inline-flex; align-items: center; gap: 6px; border-radius: 20px; font-size: 14px; font-weight: 500; color: rgba(255, 255, 255, .85); }
  .pk:active { background: rgba(255, 255, 255, .08); }
  .pk.danger { color: #fff; background: var(--danger); }
  .pk.danger:disabled { opacity: .4; }
  .pick-more { padding: 8px 16px 2px; display: flex; flex-wrap: wrap; gap: 8px; }
  .chip { height: 36px; padding: 0 12px 0 10px; display: inline-flex; align-items: center; gap: 6px; border-radius: 18px; font-size: 13.5px; color: rgba(255, 255, 255, .8); background: rgba(255, 255, 255, .07); }
  .chip:active { background: rgba(255, 255, 255, .13); }
  .empty { margin: 0; padding: 8px 16px 0; font-size: 14px; color: var(--dim); }
  .fempty { display: flex; flex-direction: column; align-items: flex-start; gap: 10px; padding: 4px 16px 0; }
  .fempty .empty { padding: 0; }
  .rows { padding: 0 16px; }
  .rows :global(.row) { height: 60px; }
</style>
