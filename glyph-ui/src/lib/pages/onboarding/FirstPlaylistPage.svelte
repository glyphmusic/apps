<script lang="ts">
  // The "first playlist" step of the onboarding: the user names a playlist
  // and ticks songs for it. Suggestions come from the step before (picked
  // songs, songs of picked artists) and the starter catalog; a search adds
  // anything else. The playlist is made when the tour ends (Onboarding.svelte),
  // so going back and forth never makes two.
  import { onMount } from "svelte";
  import { backend } from "../../api/backend";
  import { t, tf } from "../../i18n.svelte";
  import { primaryArtist, trackKey, type OnboardingCatalogResponse, type SearchResults, type Track } from "../../api/types";
  import Icon from "../../components/Icon.svelte";
  import Cover from "../../components/Cover.svelte";

  let {
    name = $bindable(""),
    picked = $bindable<Track[]>([]),
    artists = [],
    tracks = [],
    touch = false,
  }: {
    name: string;
    picked: Track[];
    artists?: string[];
    tracks?: Track[];
    touch?: boolean;
  } = $props();

  let catalog = $state<OnboardingCatalogResponse | null>(null);
  let query = $state("");
  let found = $state<Track[]>([]);
  let searching = $state(false);
  let timer = 0;
  let active = "";

  const norm = (s: string) => s.trim().toLowerCase();
  const pickedKeys = $derived(new Set(picked.map(trackKey)));

  /** Picked songs first, then songs of picked artists, then the rest of the catalog. */
  const suggestions = $derived.by(() => {
    const out: Track[] = [];
    const seen = new Set<string>();
    const add = (tr: Track | null | undefined) => {
      if (!tr) return;
      const k = trackKey(tr);
      if (seen.has(k)) return;
      seen.add(k);
      out.push(tr);
    };
    const liked = new Set(artists.map(norm));
    tracks.forEach(add);
    catalog?.artists.filter((a) => liked.has(norm(a.name))).forEach((a) => add(a.sample_track));
    catalog?.tracks.filter((tr) => liked.has(norm(primaryArtist(tr.artist)))).forEach(add);
    catalog?.tracks.forEach(add);
    return out.slice(0, 24);
  });
  const rows = $derived(query.trim().length >= 2 ? found : suggestions);

  onMount(() => {
    let alive = true;
    backend().call<OnboardingCatalogResponse>("taste_onboarding_catalog").then((c) => { if (alive && c) catalog = c; }).catch(() => {});
    // Songs picked on the taste step start out ticked.
    if (!picked.length && tracks.length) picked = tracks.slice(0, 12);
    const off = backend().on("search", (res: SearchResults) => {
      if (!alive || !active || norm(res.query) !== norm(active)) return;
      found = (res.tracks ?? []).slice(0, 24);
      if (!res.loading) searching = false;
    });
    return () => { alive = false; clearTimeout(timer); off(); };
  });

  function onInput() {
    const q = query.trim();
    clearTimeout(timer);
    if (q.length < 2) { active = ""; found = []; searching = false; return; }
    searching = true;
    timer = window.setTimeout(() => {
      active = q;
      backend().call<SearchResults>("search", { query: q, page: 0 })
        .then((res) => {
          if (norm(active) !== norm(q)) return;
          if (res?.tracks) found = res.tracks.slice(0, 24);
          if (!res?.loading) searching = false;
        })
        .catch(() => { if (norm(active) === norm(q)) searching = false; });
    }, 280);
  }

  function toggle(tr: Track) {
    const k = trackKey(tr);
    picked = pickedKeys.has(k) ? picked.filter((x) => trackKey(x) !== k) : [...picked, tr];
  }
</script>

<div class="fp" class:touch>
  <label class="name">
    <span class="cover" aria-hidden="true">
      {#if picked[0]}<Cover track={picked[0]} size={touch ? 44 : 40} radius={6} />{:else}<Icon name="list-music" size={18} color="rgba(255,255,255,.6)" />{/if}
    </span>
    <span class="nm">
      <input bind:value={name} placeholder={t("onboarding.playlist.name_ph")} maxlength="80" aria-label={t("onboarding.playlist.name_ph")} />
      <span class="ct mono" class:on={picked.length > 0}>{tf("common.n_songs", { n: picked.length })}</span>
    </span>
  </label>

  <div class="search">
    <Icon name="search" size={15} color="rgba(255,255,255,.5)" />
    <input bind:value={query} oninput={onInput} placeholder={t("onboarding.playlist.search")} aria-label={t("onboarding.playlist.search")} />
    {#if searching}<span class="spin" aria-hidden="true"></span>{/if}
  </div>

  <div class="list">
    {#each rows as tr (trackKey(tr))}
      {@const on = pickedKeys.has(trackKey(tr))}
      <button class="row" class:on aria-pressed={on} onclick={() => toggle(tr)}>
        <Cover track={tr} size={touch ? 40 : 34} radius={5} />
        <span class="tx">
          <span class="t trunc">{tr.title}</span>
          <span class="a trunc">{tr.artist}</span>
        </span>
        <span class="mark" class:on aria-hidden="true">{#if on}<Icon name="check" size={13} color="#000" />{:else}<Icon name="plus" size={14} color="rgba(255,255,255,.7)" />{/if}</span>
      </button>
    {:else}
      <div class="empty">{searching ? "" : t("onboarding.playlist.empty")}</div>
    {/each}
  </div>
</div>

<style>
  .fp { width: 100%; height: 100%; padding: 12px; display: flex; flex-direction: column; gap: 8px; font-family: var(--font); }
  .name { flex: none; display: flex; align-items: center; gap: 10px; padding: 6px 8px; border-radius: 12px; background: rgba(255, 255, 255, .05); cursor: text; }
  .cover { flex: none; width: 40px; height: 40px; border-radius: 6px; display: grid; place-items: center; background: rgba(255, 255, 255, .06); overflow: hidden; }
  .touch .cover { width: 44px; height: 44px; }
  .nm { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .nm input { width: 100%; font: 600 16px var(--font); color: #fff; background: none; border: 0; outline: 0; padding: 0; }
  .nm input::placeholder { color: rgba(255, 255, 255, .45); }
  .ct { font-size: 11px; color: rgba(255, 255, 255, .45); transition: color .15s; }
  .ct.on { color: var(--accent); }
  .search { flex: none; height: 36px; padding: 0 10px; display: flex; align-items: center; gap: 8px; border-radius: 10px; background: rgba(255, 255, 255, .05); }
  .touch .search { height: 42px; }
  .search input { flex: 1; min-width: 0; font: 400 13.5px var(--font); color: #fff; background: none; border: 0; outline: 0; }
  .search input::placeholder { color: rgba(255, 255, 255, .45); }
  .spin { width: 12px; height: 12px; border-radius: 50%; border: 2px solid rgba(255, 255, 255, .2); border-top-color: var(--accent); animation: spin .8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .list { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; margin: 0 -4px; padding: 0 4px; }
  .row { flex: none; display: flex; align-items: center; gap: 10px; padding: 5px 8px 5px 5px; border-radius: 9px; text-align: left; transition: background-color .12s; }
  .row:hover { background: rgba(255, 255, 255, .05); }
  .row.on { background: color-mix(in srgb, var(--accent) 12%, transparent); }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .t { font-size: 13.5px; font-weight: 500; color: #fff; }
  .a { font-size: 12px; color: rgba(255, 255, 255, .55); }
  .touch .t { font-size: 15px; }
  .touch .a { font-size: 13px; }
  .mark { flex: none; width: 24px; height: 24px; border-radius: 12px; display: grid; place-items: center; border: 1.5px solid rgba(255, 255, 255, .3); transition: background-color .15s, border-color .15s; }
  .mark.on { background: var(--accent); border-color: var(--accent); animation: pop .22s var(--ease); }
  @keyframes pop { from { transform: scale(.7); } 60% { transform: scale(1.15); } to { transform: none; } }
  .empty { padding: 18px 8px; font-size: 13px; color: rgba(255, 255, 255, .5); text-align: center; }
</style>
