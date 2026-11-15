<script lang="ts">
  // views.rs add_songs_panel: "Add songs" next to an own playlist (320 px
  // panel like the queue). Header (pl 16, pr 10, pt 20, pb 12: title lg
  // semibold + close), search pill (h 36, mx 16, mb 10, px 12, round, card
  // colour, 14 px search icon), tab chips Suggested / Liked / Recent (gap 6,
  // px 16, pb 8; hidden while searching), then the rows (h 52, px 6, gap 10,
  // radius 8, hover card colour; cover 40 radius 4; title sm, artist xs dim;
  // a + that turns into a pink check once the song is in the playlist).
  // Click plays the song. Another tab fades in. Search: first page, 300 ms
  // after typing, up to 50 songs. Suggestions: songs similar to the
  // playlist's own (backend `add_suggestions`), liked songs / Home feeds
  // when the playlist is empty.
  import { fade } from "svelte/transition";
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import ToggleButton from "../components/ToggleButton.svelte";
  import Button from "../components/Button.svelte";
  import Cover from "../components/Cover.svelte";
  import SongTitle from "../components/SongTitle.svelte";
  import EmptyState from "../components/EmptyState.svelte";
  import Segmented from "../settings/Segmented.svelte";
  import FadeScroll from "./panel/FadeScroll.svelte";
  import Skeletons from "../pages/search/Skeletons.svelte";
  import { app } from "../state.svelte";
  import { onWeb } from "../web";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import { rating, type SearchResults, type Track } from "../api/types";

  const a = $derived(app.addSongs);
  const id = $derived(a?.id ?? "");
  const tab = $derived(a?.tab ?? 0);
  let query = $state("");
  let results = $state<Track[]>([]);
  let searching = $state(false);
  let sugg = $state<Track[]>([]);
  let suggLoading = $state(false);
  let scrollEl: HTMLDivElement | undefined = $state();
  let seq = 0, timer = 0;

  let linkInput = $state("");
  let linkFound = $state<string | null>(null);
  let pastedCounts = $state<[number, number]>([0, 0]);
  let importing = $state(false);
  let importError = $state<string | null>(null);
  let spin = $state(0);
  let prog = $state({ done: 0, total: 0 });
  let linkSeq = 0;

  const q = $derived(query.trim());
  const pl = $derived(app.playlist(id));
  const have = $derived(new Set((pl?.tracks ?? []).map((x) => `${x.source}:${x.id}`)));

  $effect(() => backend().on("import_progress", (p) => (prog = p)));
  $effect(() => {
    if (!importing) return;
    const iv = setInterval(() => spin++, 110);
    return () => clearInterval(iv);
  });

  $effect(() => {
    const v = linkInput.trim();
    linkFound = null;
    pastedCounts = [0, 0];
    if (!v) return;
    const n = ++linkSeq;
    backend().call<string | null>("import_parse_url", { url: v }).then((k) => { if (n === linkSeq) linkFound = k ?? null; });
    backend().call<[number, number]>("migrate_pasted", { text: v }).then((c) => { if (n === linkSeq) pastedCounts = c ?? [0, 0]; });
  });

  // Suggestions for the playlist that is open.
  $effect(() => {
    const pid = id;
    query = ""; results = []; sugg = [];
    linkInput = ""; linkFound = null; pastedCounts = [0, 0]; importError = null; importing = false; prog = { done: 0, total: 0 };
    if (!pid) return;
    let alive = true;
    suggLoading = true;
    backend().call<Track[]>("add_suggestions", { id: pid }).then((v) => { if (alive) { sugg = (v ?? []).filter((x) => app.allowed(x)); suggLoading = false; } });
    return () => { alive = false; };
  });

  function onInput() {
    clearTimeout(timer);
    const mine = ++seq, text = q;
    results = [];
    if (!text) { searching = false; return; }
    searching = true;
    timer = window.setTimeout(async () => {
      const r = await backend().call<SearchResults>("search", { query: text, page: 0 });
      if (mine !== seq) return;
      const seen = new Set<string>();
      results = (r?.tracks ?? []).filter((x) => app.allowed(x) && !seen.has(`${x.source}:${x.id}`) && seen.add(`${x.source}:${x.id}`)).slice(0, 50);
      searching = false;
    }, 300);
  }
  const songs = $derived<Track[]>(q ? results : tab === 1 ? app.library.liked.slice(0, 100) : tab === 2 ? app.library.recent.flatMap((r) => (r.kind === "song" ? [r.track] : [])) : sugg);
  const loading = $derived(q ? searching : tab === 0 && suggLoading);

  function toggle(tr: Track) {
    const i = (pl?.tracks ?? []).findIndex((x) => x.id === tr.id && x.source === tr.source);
    if (i >= 0) app.removeFromList({ kind: "playlist", id }, i);
    else app.addToPlaylist(id, [tr]);
  }
  async function doImportLink() {
    const v = linkInput.trim();
    if (!v || !id || importing) return;
    importError = null;
    importing = true;
    const r = await backend().call<{ error?: string | null } | null>("import_link", { url: v, playlist_id: id }).catch((e) => ({ error: String(e) }));
    importing = false;
    if (r?.error) { importError = r.error; return; }
    linkInput = "";
  }
  function onLinkKey(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey && !linkInput.includes("\n")) {
      e.preventDefault();
      doImportLink();
    }
  }
  function setTab(n: number) { if (app.addSongs) app.addSongs.tab = n; scrollEl?.scrollTo(0, 0); }
  const TABS: [string, number][] = [["add.tab.suggested", 0], ["add.tab.liked", 1], ["add.tab.recent", 2], ["add.tab.import_short", 3]];
  const tabOptions = $derived(TABS.map(([k, v]) => ({ value: v, label: t(k) })));
</script>

<div class="ap">
  <div class="head">
    <div class="ttl">{t("add.title")}</div>
    <IconButton name="x" size={16} onclick={() => (app.addSongs = null)} />
  </div>
  {#if !q}
    <div class="tabs"><Segmented fill value={tab} options={tabOptions} onpick={setTab} /></div>
  {/if}
  {#if tab !== 3}
    <label class="search">
      <Icon name="search" size={14} color="var(--dim)" />
      <input bind:value={query} oninput={onInput} placeholder={t("add.search")} spellcheck="false" />
    </label>
  {/if}
  <FadeScroll bind:el={scrollEl} pad="0 10px 10px">
    <div class="stack">
      {#key `${tab}:${!!q}`}
        <div class="pane" in:fade={{ duration: 160, delay: 25 }} out:fade={{ duration: 120 }}>
          {#if tab === 3 && !q}
            <div class="imp">
              <textarea class="imp-in" bind:value={linkInput} onkeydown={onLinkKey} placeholder={t("add.import.placeholder")} rows={3} spellcheck="false"></textarea>
              {#if importError}
                <div class="st err"><span class="ii"><Icon name="info" size={13} color="rgba(255,255,255,.6)" /></span><span>{importError}</span></div>
              {:else if importing}
                <div class="st spin"><span class="mono sp">{"|/-\\"[spin % 4]}</span>{t("dialog.playlist.reading")}{#if prog.total > 0} {tf("import.progress", { done: prog.done, total: prog.total })}{/if}</div>
              {:else if linkFound}
                <div class="st ok"><Icon name="check" size={13} color="rgba(255,255,255,.65)" />{tf("dialog.playlist.link.found", { kind: linkFound })}</div>
              {:else if pastedCounts[0] + pastedCounts[1] > 0}
                <div class="st ok"><Icon name="check" size={13} color="rgba(255,255,255,.65)" />{tf("migrate.pasted", { songs: pastedCounts[0], lists: pastedCounts[1] })}</div>
              {:else if linkInput.trim()}
                <div class="st bad">{t("import.unknown_link")}</div>
              {/if}
              <div class="imp-act">
                <Button kind="primary" disabled={importing || !linkInput.trim()} onclick={doImportLink}>{importing ? t("dialog.importing") : t("dialog.import")}</Button>
                {#if !onWeb()}
                  <Button icon="folder-open" disabled={importing} onclick={() => backend().call("import_playlist_file", { playlist_id: id })}>{t("dialog.playlist.file")}</Button>
                {/if}
              </div>
            </div>
          {:else if !songs.length && loading}
            <div class="px"><Skeletons kind="rows" n={8} prefix="add" /></div>
          {:else if !songs.length}
            <div class="px"><EmptyState title={t(q ? "search.none.title" : "add.empty")} /></div>
          {:else}
            {#each songs as tr, i (`${tr.source}:${tr.id}:${i}`)}
              {@const added = have.has(`${tr.source}:${tr.id}`)}
              <div class="row" onclick={() => app.play([tr], 0)} onkeydown={(e) => e.key === "Enter" && app.play([tr], 0)} role="row" tabindex="-1">
                <Cover track={tr} size={40} radius={4} />
                <span class="tx">
                  <span class="t"><SongTitle rating={rating(tr)}>{tr.title}</SongTitle></span>
                  <span class="ar trunc">{tr.artist}</span>
                </span>
                <ToggleButton name="circle-plus" onName="circle-check" size={18} on={added} accent="var(--accent)" notip onclick={() => toggle(tr)} />
              </div>
            {/each}
          {/if}
        </div>
      {/key}
    </div>
  </FadeScroll>
</div>

<style>
  .ap { width: 100%; height: 100%; display: flex; flex-direction: column; background: rgba(12, 12, 12, .94); border-radius: 12px; overflow: hidden; }
  .head { flex: none; display: flex; align-items: center; padding: 20px 10px 12px 16px; }
  .ttl { flex: 1; font-size: var(--lg); font-weight: 600; }
  .search { flex: none; margin: 0 12px 10px; height: 34px; padding: 0 10px; display: flex; align-items: center; gap: 8px; border-radius: 8px; background: rgba(255, 255, 255, .06); transition: background-color .15s ease-out; cursor: text; }
  .search:focus-within { background: rgba(255, 255, 255, .1); }
  .search input { flex: 1; min-width: 0; background: none; border: 0; font-size: var(--sm); color: var(--text); }
  .search input::placeholder { color: rgba(255, 255, 255, .4); }
  .tabs { flex: none; padding: 0 12px 10px; }
  .stack { display: grid; }
  .pane { grid-area: 1 / 1; min-width: 0; }
  .px { padding: 0 6px; }
  .imp { padding: 4px 6px 12px; display: flex; flex-direction: column; gap: 10px; }
  .imp-in { width: 100%; padding: 10px 12px; border-radius: 8px; background: rgba(255, 255, 255, .05); border: 0; outline: none; resize: vertical; font: inherit; font-size: var(--sm); color: var(--text); }
  .imp-in::placeholder { color: var(--faint); }
  .st { font-size: var(--xs); line-height: 1.45; }
  .err { display: flex; align-items: flex-start; gap: 6px; color: rgba(255, 255, 255, .8); }
  .ii { flex: none; padding-top: 2px; }
  .spin { display: flex; align-items: center; gap: 8px; color: var(--dim); }
  .sp { color: var(--text); }
  .ok { display: flex; align-items: center; gap: 6px; color: rgba(255, 255, 255, .75); }
  .bad { color: var(--dim); }
  .imp-act { display: flex; flex-wrap: wrap; gap: 8px; }
  .row { display: flex; align-items: center; gap: 10px; height: 52px; padding: 0 6px; border-radius: 8px; cursor: pointer; transition: background-color .24s ease-out; }
  .row:hover { background: var(--card); transition-duration: .12s; }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .t { font-size: var(--sm); min-width: 0; }
  .ar { font-size: var(--xs); color: var(--dim); }
</style>
