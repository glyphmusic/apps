<script lang="ts">
  // chrome.rs search_field + search_dropdown: flat dark field (white 8 %),
  // a border only when focused, shortcut chip; empty and focused it drops
  // down recent searches (or recently played).
  import Icon from "../components/Icon.svelte";
  import Cover from "../components/Cover.svelte";
  import ArtistAvatar from "../components/ArtistAvatar.svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { shortcut } from "../window";
  import { fadeEdges } from "../ui.svelte";
  import type { SearchItem } from "../api/types";
  let { width = 420 as number | null }: { width?: number | null } = $props();
  let input: HTMLInputElement;
  let focused = $state(false);

  export function focus() { input?.focus(); input?.select(); }

  const history = $derived(app.library.search_history.length > 0);
  const items = $derived<SearchItem[]>(
    history
      ? app.library.search_history.slice(0, 8)
      : app.library.recent.flatMap((r): SearchItem[] => (r.kind === "song" ? [{ kind: "song", track: r.track }] : r.kind === "artist" ? [{ kind: "artist", name: r.name, track: r.track }] : [])).slice(0, 8),
  );
  const open = $derived(width != null && focused && !app.searchQuery.trim() && items.length > 0);

  function pick(item: SearchItem) {
    input.blur();
    if (item.kind === "song") { app.playOne(item.track); app.pickSearch(item); }
    else if (item.kind === "artist") { app.openArtistPage(item.name); app.pickSearch(item); }
    else app.searchFor(item.text);
  }
  function key(e: KeyboardEvent) {
    if (e.key === "Enter") { app.runSearch(); }
    else if (e.key === "Escape") { if (app.searchQuery) app.setQuery(""); else input.blur(); }
  }
  $effect(() => { app.searchFocused = focused; });
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="sf" class:focused class:wide={width == null} style:width={width ? `${width}px` : undefined} data-no-drag
  onclick={(e) => { if (!(e.target as HTMLElement)?.closest("button, .dd")) input?.focus(); }}>
  <Icon name="search" size={width == null ? 16 : 15} color="rgba(255,255,255,.55)" />
  <input bind:this={input} id="search-input" value={app.searchQuery} placeholder={t("search.placeholder")} spellcheck="false" autocomplete="off"
    oninput={(e) => app.setQuery(e.currentTarget.value)} onfocus={() => (focused = true)} onblur={() => setTimeout(() => (focused = false), 120)} onkeydown={key} />
  {#if app.searchQuery}
    <button class="clr" type="button" aria-label={t("tip.x")}
      onpointerdown={(e) => e.preventDefault()}
      onclick={() => { app.setQuery(""); input?.focus(); }}>
      <Icon name="x" size={width == null ? 13 : 12} />
    </button>
  {/if}
  {#if width && !app.searchQuery}<span class="kbd mono">{shortcut()}</span>{/if}

  {#if open}
    <div class="dd" role="listbox">
      <div class="dd-in scroll" use:fadeEdges={{ top: 10, bottom: 10, minOverflow: 8 }}>
        <div class="dh">{history ? t("search.history") : t("search.recently_played")}</div>
        {#each items as item, i}
          <div class="it" role="option" aria-selected="false" tabindex="-1" onpointerdown={(e) => { e.preventDefault(); pick(item); }}>
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
  .dd { position: absolute; top: 38px; left: 0; right: 0; border-radius: 8px; background: #1c1c1c; overflow: hidden; z-index: 50; cursor: default; animation: rise-in .14s ease-out; }
  .dd-in { max-height: 440px; padding: 8px; display: flex; flex-direction: column; gap: 2px; }
  .dh { padding: 4px 8px 8px; font-size: var(--md); font-weight: 700; color: #fff; }
  .it { display: flex; align-items: center; gap: 12px; padding: 6px 8px; border-radius: 6px; cursor: pointer; transition: background-color .12s; }
  .it:hover { background: rgba(255, 255, 255, .08); }
  .art { position: relative; flex: none; }
  .po { position: absolute; inset: 0; border-radius: 4px; display: grid; place-items: center; background: rgba(0, 0, 0, .45); opacity: 0; transition: opacity .12s; }
  .it:hover .po { opacity: 1; }
  .ph { width: 44px; height: 44px; border-radius: 4px; background: rgba(255, 255, 255, .08); display: grid; place-items: center; }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .t { font-size: var(--md); font-weight: 500; color: #fff; }
  .s { font-size: var(--sm); color: rgba(255, 255, 255, .6); }
  .x { flex: none; width: 28px; height: 28px; border-radius: 6px; display: grid; place-items: center; opacity: 0; visibility: hidden; transition: opacity .12s; }
  .it:hover .x { opacity: 1; visibility: visible; }
  .x:hover { background: rgba(255, 255, 255, .1); }
</style>
