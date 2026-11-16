<script lang="ts">
  // compact.rs search_bar + search_recent: idle, the "Search" header with the
  // full-width field under it; while typing a back arrow + the field. Without
  // a query: recent searches (or recently played), 64 px rows, 52 px art.
  import Icon from "../components/Icon.svelte";
  import Cover from "../components/Cover.svelte";
  import SearchField from "../shell/SearchField.svelte";
  import Header from "./Header.svelte";
  import IconAction from "./IconAction.svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import type { SearchItem } from "../api/types";
  let { part, bg = 0 }: { part: "bar" | "recent"; bg?: number } = $props();
  const active = $derived(app.searchFocused || app.searchQuery.trim() !== "");
  const history = $derived(app.library.search_history.length > 0);
  const items = $derived<SearchItem[]>(history ? app.library.search_history : app.library.recent.flatMap((r): SearchItem[] => (r.kind === "song" ? [{ kind: "song", track: r.track }] : r.kind === "artist" ? [{ kind: "artist", name: r.name, track: r.track }] : [])).slice(0, 12));
  function pick(it: SearchItem) {
    if (it.kind === "song") { app.playOne(it.track); app.pickSearch(it); }
    else if (it.kind === "artist") { app.openArtistPage(it.name); app.pickSearch(it); }
    else app.searchFor(it.text);
  }
  function itemMenu(e: MouseEvent, it: SearchItem) {
    if (it.kind === "song") { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: it.track }); }
    else if (it.kind === "artist") { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: it.track ?? null, artist: it.name }); }
  }
</script>

{#if part === "bar"}
  <div style:background="rgba(10, 10, 10, {bg})">
    {#if !active}
      <Header title={t("nav.search")} />
    {/if}
    <!-- Keep one SearchField mounted across active transitions so focusing it on tap never unmounts the input. -->
    <div class:bar={active} class:field={!active}>
      {#if active}
        <IconAction name="arrow-left" onclick={() => { app.setQuery(""); app.searchFocused = false; (document.activeElement as HTMLElement)?.blur(); }} />
      {/if}
      <div class:flex1={active}><SearchField width={null} /></div>
    </div>
  </div>
{:else if !items.length}
  <p class="hint">{t("search.empty.sub")}</p>
{:else}
  <h2>{history ? t("search.history") : t("search.recently_played")}</h2>
  {#each items as it, i}
    <div class="it" onclick={() => pick(it)} oncontextmenu={(e) => itemMenu(e, it)} onkeydown={() => {}} role="button" tabindex="-1">
      {#if it.kind === "song"}<Cover track={it.track} size={52} radius={4} />
      {:else if it.kind === "artist"}{#if it.track}<Cover track={it.track} size={52} round />{:else}<span class="ph r"><Icon name="user-round" size={22} color="rgba(255,255,255,.6)" /></span>{/if}
      {:else}<span class="ph"><Icon name="search" size={22} color="rgba(255,255,255,.6)" /></span>{/if}
      <span class="tx">
        <span class="t trunc">{it.kind === "song" ? it.track.title : it.kind === "artist" ? it.name : it.text}</span>
        <span class="s trunc">{it.kind === "song" ? `${t("search.kind.song")} • ${it.track.artist}` : it.kind === "artist" ? t("search.kind.artist") : t("search.kind.query")}</span>
      </span>
      {#if history}<button class="x" aria-label={t("tip.x")} onclick={(e) => { e.stopPropagation(); app.removeSearchHistory(i); }}><Icon name="x" size={20} color="var(--dim)" /></button>{/if}
    </div>
  {/each}
{/if}

<style>
  .bar { flex: none; height: 60px; padding: 0 16px 0 4px; display: flex; align-items: center; gap: 4px; }
  .field { padding: 0 16px 8px; }
  .hint { margin: 0; padding-top: 24px; font-size: 14px; color: var(--dim); }
  h2 { margin: 0; padding-bottom: 6px; font-size: 18px; font-weight: 700; color: var(--text); }
  .it { height: 64px; display: flex; align-items: center; gap: 12px; border-radius: 8px; transition: background-color .12s var(--ease), transform .16s var(--ease); }
  .it:active { background: rgba(255, 255, 255, .05); transform: scale(.98); }
  .ph { flex: none; width: 52px; height: 52px; border-radius: 4px; background: rgba(255, 255, 255, .08); display: grid; place-items: center; }
  .ph.r { border-radius: 50%; }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .t { font-size: 15px; color: var(--text); }
  .s { font-size: 12.5px; color: var(--dim); }
  .x { flex: none; width: 40px; height: 40px; border-radius: 50%; display: grid; place-items: center; }
</style>
