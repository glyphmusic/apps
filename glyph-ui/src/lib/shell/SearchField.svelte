<script lang="ts">
  // chrome.rs search_field + search_dropdown: flat dark field with Ctrl+K
  // quick-jump command palette while typing (playlists, artists, albums, local
  // songs, app pages) and recent searches when empty, navigable with ↑ / ↓ / Enter.
  import Icon from "../components/Icon.svelte";
  import Cover from "../components/Cover.svelte";
  import ListCover from "../components/ListCover.svelte";
  import ArtistAvatar from "../components/ArtistAvatar.svelte";
  import { app } from "../state.svelte";
  import { onWeb } from "../web";
  import { t, tf } from "../i18n.svelte";
  import { shortcut } from "../window";
  import { fadeEdges } from "../ui.svelte";
  import type { SearchItem } from "../api/types";
  import { buildQuickJump, type QuickJumpItem, type QuickJumpPage } from "./quick-jump";

  let { width = 420 as number | null }: { width?: number | null } = $props();
  let input: HTMLInputElement;
  let ddEl: HTMLDivElement | undefined = $state();
  let focused = $state(false);
  let dismissed = $state(false);
  let activeIdx = $state(-1);

  export function focus() {
    dismissed = false;
    input?.focus();
    input?.select();
  }

  const trimmed = $derived(app.searchQuery.trim());
  const typingMode = $derived(trimmed.length > 0);
  const history = $derived(app.library.search_history.length > 0);
  const idleItems = $derived<SearchItem[]>(
    history
      ? app.library.search_history.slice(0, 8)
      : app.library.recent
          .flatMap((r): SearchItem[] =>
            r.kind === "song" ? [{ kind: "song", track: r.track }] : r.kind === "artist" ? [{ kind: "artist", name: r.name, track: r.track }] : [],
          )
          .slice(0, 8),
  );

  const pages = $derived.by<QuickJumpPage[]>(() => {
    const out: QuickJumpPage[] = [
      { id: "home", label: t("nav.home"), keywords: ["home", "start"], icon: "house" },
      { id: "library", label: t("nav.library"), keywords: ["library", "bibliothek"], icon: "library" },
      { id: "now_playing", label: t("nav.now_playing"), keywords: ["now playing", "lyrics", "player"], icon: "disc-3" },
      { id: "stats", label: t("stats.title"), keywords: ["stats", "statistiken", "recap"], icon: "chart-column" },
      { id: "settings", label: t("nav.settings"), keywords: ["settings", "einstellungen", "config"], icon: "settings" },
    ];
    if (!onWeb()) {
      out.push({ id: "migrate", label: t("library.migrate"), keywords: ["transfer", "import", "spotify", "übertragen"], icon: "arrow-right-left" });
      if (app.platform !== "android" && app.platform !== "ios") {
        out.push({ id: "local", label: t("local.title"), keywords: ["local", "files", "lokal", "dateien"], icon: "hard-drive" });
      }
    }
    return out;
  });

  const builtInLists = $derived.by(() => {
    const out = [
      { list: { kind: "liked" } as const, title: t("library.liked"), count: app.library.liked.length, sub: tf("common.n_songs", { n: app.library.liked.length }) },
    ] as { list: import("../api/types").ListKey; title: string; count: number; sub: string }[];
    if (app.library.recent.length) {
      out.push({ list: { kind: "recent" }, title: t("library.recent"), count: app.library.recent.length, sub: t("list.kind.playlist") });
    }
    if (app.library.downloads.length) {
      out.push({ list: { kind: "downloads" }, title: t("library.downloads"), count: app.library.downloads.length, sub: tf("common.n_songs", { n: app.library.downloads.length }) });
    }
    return out;
  });

  const quickItems = $derived<QuickJumpItem[]>(
    width != null && focused && typingMode
      ? buildQuickJump({
          query: trimmed,
          playlists: app.library.playlists,
          liked: app.library.liked,
          savedAlbums: app.library.saved_albums,
          followed: app.library.followed,
          builtInLists,
          pages,
          limit: 7,
        })
      : [],
  );

  // Total navigable rows in the dropdown (quick items + "Search for …" when typing, or idle history).
  const rowCount = $derived(typingMode ? quickItems.length + 1 : idleItems.length);
  const open = $derived(width != null && focused && !dismissed && (typingMode ? quickItems.length > 0 : idleItems.length > 0));

  $effect(() => {
    void trimmed;
    activeIdx = -1;
    dismissed = false;
  });

  $effect(() => {
    if (activeIdx >= rowCount) activeIdx = rowCount - 1;
  });

  $effect(() => { app.searchFocused = focused; });

  function scrollActiveIntoView(idx: number) {
    if (!ddEl || idx < 0) return;
    const row = ddEl.querySelector<HTMLElement>(`[data-dd-idx="${idx}"]`);
    row?.scrollIntoView({ block: "nearest" });
  }

  function pickIdle(item: SearchItem) {
    input.blur();
    if (item.kind === "song") { app.playOne(item.track); app.pickSearch(item); }
    else if (item.kind === "artist") { app.openArtistPage(item.name); app.pickSearch(item); }
    else app.searchFor(item.text);
  }

  function pickQuick(item: QuickJumpItem) {
    dismissed = true;
    input.blur();
    if (item.kind === "list") {
      app.openListView(item.list);
    } else if (item.kind === "artist") {
      app.openArtistPage(item.name);
    } else if (item.kind === "song") {
      app.playOne(item.track);
    } else if (item.kind === "page") {
      if (item.id === "migrate") app.openSettings("migrate");
      else if (item.id === "settings") app.navigate("settings");
      else app.navigate(item.id as import("../state.svelte").View);
    }
  }

  function runFullSearch() {
    dismissed = true;
    app.runSearch();
  }

  function key(e: KeyboardEvent) {
    if (open && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
      e.preventDefault();
      const delta = e.key === "ArrowDown" ? 1 : -1;
      if (activeIdx < 0) activeIdx = delta > 0 ? 0 : rowCount - 1;
      else activeIdx = (activeIdx + delta + rowCount) % rowCount;
      scrollActiveIntoView(activeIdx);
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      if (open && activeIdx >= 0) {
        if (typingMode) {
          if (activeIdx < quickItems.length) pickQuick(quickItems[activeIdx]);
          else runFullSearch();
        } else if (activeIdx < idleItems.length) {
          pickIdle(idleItems[activeIdx]);
        }
        return;
      }
      runFullSearch();
    } else if (e.key === "Escape") {
      if (open && activeIdx >= 0) {
        activeIdx = -1;
        return;
      }
      if (app.searchQuery) app.setQuery("");
      else input.blur();
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="sf" class:focused class:wide={width == null} style:width={width ? `${width}px` : undefined} data-no-drag
  onclick={(e) => { if (!(e.target as HTMLElement)?.closest("button, .dd")) input?.focus(); }}>
  <Icon name="search" size={width == null ? 16 : 15} color="rgba(255,255,255,.55)" />
  <input bind:this={input} id="search-input" value={app.searchQuery} placeholder={t("search.placeholder")} spellcheck="false" autocomplete="off"
    aria-expanded={open} aria-autocomplete="list"
    oninput={(e) => app.setQuery(e.currentTarget.value)}
    onfocus={() => { focused = true; dismissed = false; }}
    onblur={() => setTimeout(() => (focused = false), 120)}
    onkeydown={key} />
  {#if app.searchQuery}
    <button class="clr" type="button" aria-label={t("tip.x")}
      onpointerdown={(e) => e.preventDefault()}
      onclick={() => { app.setQuery(""); input?.focus(); }}>
      <Icon name="x" size={width == null ? 13 : 12} />
    </button>
  {/if}
  {#if width && !app.searchQuery}<span class="kbd mono">{shortcut()}</span>{/if}

  {#if open}
    <div class="dd" role="listbox" bind:this={ddEl}>
      <div class="dd-in scroll" use:fadeEdges={{ top: 10, bottom: 10, minOverflow: 8 }}>
        {#if typingMode}
          <div class="dh">{t("search.quick_jump")}</div>
          {#each quickItems as item, i}
            <div
              class="it"
              class:hi={activeIdx === i}
              data-dd-idx={i}
              role="option"
              aria-selected={activeIdx === i}
              tabindex="-1"
              onpointerenter={() => (activeIdx = i)}
              onpointerdown={(e) => { e.preventDefault(); pickQuick(item); }}
            >
              <span class="art">
                {#if item.kind === "list"}
                  <ListCover list={item.list} size={36} />
                {:else if item.kind === "artist"}
                  <ArtistAvatar name={item.name} track={item.track} size={36} iconSize={16} />
                {:else if item.kind === "song"}
                  <Cover track={item.track} size={36} radius={4} />
                  <span class="po"><Icon name="play" size={14} color="#fff" /></span>
                {:else}
                  <span class="ph sm"><Icon name={item.icon} size={16} color="rgba(255,255,255,.72)" /></span>
                {/if}
              </span>
              <span class="tx">
                <span class="t trunc">
                  {item.kind === "list" ? item.title : item.kind === "artist" ? item.name : item.kind === "song" ? item.track.title : item.label}
                </span>
                <span class="s trunc">
                  {#if item.kind === "list"}
                    {item.list.kind === "album" ? `${t("list.kind.album")} · ${item.sub}` : `${t("list.kind.playlist")} · ${item.list.kind === "playlist" ? tf("common.n_songs", { n: Number(item.sub) }) : item.sub}`}
                  {:else if item.kind === "artist"}
                    {t("search.kind.artist")}
                  {:else if item.kind === "song"}
                    {t("search.kind.song")} · {item.track.artist}
                  {:else}
                    {t("search.kind.page")}
                  {/if}
                </span>
              </span>
              <span class="tag">{item.kind === "list" ? (item.list.kind === "album" ? t("list.kind.album") : t("list.kind.playlist")) : item.kind === "artist" ? t("search.kind.artist") : item.kind === "song" ? t("search.kind.song") : t("search.kind.page")}</span>
            </div>
          {/each}
          <div class="dsep"></div>
          <div
            class="it qrun"
            class:hi={activeIdx === quickItems.length}
            data-dd-idx={quickItems.length}
            role="option"
            aria-selected={activeIdx === quickItems.length}
            tabindex="-1"
            onpointerenter={() => (activeIdx = quickItems.length)}
            onpointerdown={(e) => { e.preventDefault(); runFullSearch(); }}
          >
            <span class="ph sm"><Icon name="search" size={15} color="var(--accent)" /></span>
            <span class="tx">
              <span class="t trunc">{tf("search.search_for", { q: trimmed })}</span>
            </span>
            <span class="kbd mono">Enter</span>
          </div>
        {:else}
          <div class="dh">{history ? t("search.history") : t("search.recently_played")}</div>
          {#each idleItems as item, i}
            <div
              class="it"
              class:hi={activeIdx === i}
              data-dd-idx={i}
              role="option"
              aria-selected={activeIdx === i}
              tabindex="-1"
              onpointerenter={() => (activeIdx = i)}
              onpointerdown={(e) => { e.preventDefault(); pickIdle(item); }}
            >
              <span class="art">
                {#if item.kind === "song"}
                  <Cover track={item.track} size={44} radius={4} />
                  <span class="po"><Icon name="play" size={16} color="#fff" /></span>
                {:else if item.kind === "artist"}
                  <ArtistAvatar name={item.name} track={item.track} size={44} iconSize={18} />
                {:else}
                  <span class="ph"><Icon name="search" size={18} color="rgba(255,255,255,.6)" /></span>
                {/if}
              </span>
              <span class="tx">
                <span class="t trunc">{item.kind === "song" ? item.track.title : item.kind === "artist" ? item.name : item.text}</span>
                <span class="s trunc">{item.kind === "song" ? `${t("search.kind.song")} • ${item.track.artist}` : item.kind === "artist" ? t("search.kind.artist") : t("search.kind.query")}</span>
              </span>
              {#if history}
                <button class="x" aria-label={t("tip.x")} onpointerdown={(e) => { e.preventDefault(); e.stopPropagation(); app.removeSearchHistory(i); }}><Icon name="x" size={14} color="rgba(255,255,255,.7)" /></button>
              {/if}
            </div>
          {/each}
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .sf { position: relative; display: flex; align-items: center; gap: 10px; max-width: 100%; height: 32px; padding: 0 6px 0 12px; border-radius: 8px; background: #141414; border: 1px solid transparent; font-size: var(--sm); cursor: text; }
  .sf.wide { width: 100%; height: 40px; padding: 0 12px 0 14px; }
  .sf.focused { border-color: rgba(255, 255, 255, .22); }
  input { flex: 1; min-width: 0; height: 100%; background: none; border: 0; outline: none; color: rgba(255, 255, 255, .95); font-size: var(--sm); line-height: normal; padding: 0; }
  input::placeholder { color: rgba(255, 255, 255, .55); }
  .clr { flex: none; width: 22px; height: 22px; border-radius: 6px; display: grid; place-items: center; background: rgba(255, 255, 255, .14); color: rgba(255, 255, 255, .88); transition: background-color .12s, transform .12s; }
  .clr:hover { background: rgba(255, 255, 255, .24); }
  .clr:active { transform: scale(.92); }
  .sf.wide .clr { width: 26px; height: 26px; }
  .kbd { flex: none; height: 20px; padding: 0 6px; display: flex; align-items: center; border-radius: 4px; background: rgba(255, 255, 255, .08); font-size: 11px; color: rgba(255, 255, 255, .55); }
  .dd { position: absolute; top: 38px; left: 0; right: 0; border-radius: 10px; background: #18181b; box-shadow: 0 14px 36px rgba(0, 0, 0, .55); overflow: hidden; z-index: 50; cursor: default; animation: rise-in .14s ease-out; }
  .dd-in { max-height: 440px; padding: 8px; display: flex; flex-direction: column; gap: 2px; }
  .dh { padding: 4px 8px 6px; font-size: 11.5px; font-weight: 600; text-transform: uppercase; letter-spacing: .03em; color: rgba(255, 255, 255, .45); }
  .it { display: flex; align-items: center; gap: 11px; padding: 6px 8px; border-radius: 7px; cursor: pointer; transition: background-color .1s; }
  .it:hover, .it.hi { background: rgba(255, 255, 255, .09); }
  .art { position: relative; flex: none; display: grid; place-items: center; }
  .po { position: absolute; inset: 0; border-radius: 4px; display: grid; place-items: center; background: rgba(0, 0, 0, .45); opacity: 0; transition: opacity .12s; }
  .it:hover .po, .it.hi .po { opacity: 1; }
  .ph { width: 44px; height: 44px; border-radius: 6px; background: rgba(255, 255, 255, .07); display: grid; place-items: center; }
  .ph.sm { width: 36px; height: 36px; }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .t { font-size: var(--md); font-weight: 500; color: #fff; }
  .s { font-size: var(--xs); color: rgba(255, 255, 255, .58); }
  .tag { flex: none; padding: 2px 7px; border-radius: 5px; background: rgba(255, 255, 255, .06); font-size: 11px; color: rgba(255, 255, 255, .52); }
  .dsep { height: 1px; margin: 4px 6px; background: rgba(255, 255, 255, .07); }
  .x { flex: none; width: 28px; height: 28px; border-radius: 6px; display: grid; place-items: center; opacity: 0; visibility: hidden; transition: opacity .12s; }
  .it:hover .x, .it.hi .x { opacity: 1; visibility: visible; }
  .x:hover { background: rgba(255, 255, 255, .1); }
</style>
