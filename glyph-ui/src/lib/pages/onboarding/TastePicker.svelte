<script lang="ts">
  // Reusable interactive taste calibration picker: lets users pick 3+ favorite
  // artists and/or songs from genre-diverse starters or live Glyph search.
  // Used inside Onboarding (TastePage.svelte) and standalone TastePickerModal.
  import { onMount } from "svelte";
  import { backend } from "../../api/backend";
  import { t, tf } from "../../i18n.svelte";
  import { primaryArtist, trackKey, type OnboardingArtist, type OnboardingCatalogResponse, type SearchResults, type Track } from "../../api/types";
  import Icon from "../../components/Icon.svelte";
  import Cover from "../../components/Cover.svelte";
  import ArtistAvatar from "../../components/ArtistAvatar.svelte";

  let {
    selectedArtists = $bindable<string[]>([]),
    selectedTracks = $bindable<Track[]>([]),
    selectedGenres = $bindable<string[]>([]),
    compact = false,
  }: {
    selectedArtists: string[];
    selectedTracks: Track[];
    selectedGenres?: string[];
    compact?: boolean;
  } = $props();

  let mode = $state<"artists" | "songs">("artists");
  let query = $state("");
  let genre = $state<string>("all");
  let catalog = $state<OnboardingCatalogResponse | null>(null);
  let loadingCatalog = $state(true);
  let searchLoading = $state(false);
  let searchArtists = $state<{ name: string; track?: Track | null }[]>([]);
  let searchTracks = $state<Track[]>([]);
  let searchTimer = 0;
  let activeQuery = "";

  const norm = (s: string) => s.trim().toLowerCase();
  const totalPicked = $derived(selectedArtists.length + selectedTracks.length);

  onMount(() => {
    let alive = true;
    backend()
      .call<OnboardingCatalogResponse>("taste_onboarding_catalog")
      .then((res) => {
        if (alive && res) catalog = res;
      })
      .catch(() => {})
      .finally(() => {
        if (alive) loadingCatalog = false;
      });

    const off = backend().on("search", (res: SearchResults) => {
      if (!alive || !activeQuery || norm(res.query) !== norm(activeQuery)) return;
      searchArtists = (res.artists ?? []).slice(0, 14);
      searchTracks = (res.tracks ?? []).slice(0, 18);
      if (!res.loading) searchLoading = false;
    });

    return () => {
      alive = false;
      clearTimeout(searchTimer);
      off();
    };
  });

  function onQueryInput() {
    const q = query.trim();
    clearTimeout(searchTimer);
    if (q.length < 2) {
      activeQuery = "";
      searchArtists = [];
      searchTracks = [];
      searchLoading = false;
      return;
    }
    searchLoading = true;
    searchTimer = window.setTimeout(() => {
      activeQuery = q;
      backend()
        .call<SearchResults>("search", { query: q, page: 0 })
        .then((res) => {
          if (norm(activeQuery) !== norm(q)) return;
          if (res?.artists) searchArtists = res.artists.slice(0, 14);
          if (res?.tracks) searchTracks = res.tracks.slice(0, 18);
          searchLoading = false;
        })
        .catch(() => {
          if (norm(activeQuery) === norm(q)) searchLoading = false;
        });
    }, 280);
  }

  function hasArtist(name: string): boolean {
    const k = norm(name);
    return selectedArtists.some((a) => norm(a) === k);
  }

  function toggleArtist(name: string, artistGenre?: string) {
    const k = norm(name);
    if (!k) return;
    if (hasArtist(name)) {
      const removedGenre = artistGenre || catalog?.artists.find((a) => norm(a.name) === k)?.genre || "";
      selectedArtists = selectedArtists.filter((a) => norm(a) !== k);
      if (removedGenre) {
        const stillUsed = selectedArtists.some((a) => {
          const ag = catalog?.artists.find((c) => norm(c.name) === norm(a))?.genre;
          return ag && norm(ag) === norm(removedGenre);
        });
        if (!stillUsed) {
          selectedGenres = selectedGenres.filter((g) => norm(g) !== norm(removedGenre));
        }
      }
    } else {
      selectedArtists = [...selectedArtists, name];
      if (artistGenre && artistGenre !== "Trending" && !selectedGenres.some((g) => norm(g) === norm(artistGenre))) {
        selectedGenres = [...selectedGenres, artistGenre];
      }
    }
  }

  function hasTrack(tr: Track): boolean {
    const k = trackKey(tr);
    return selectedTracks.some((x) => trackKey(x) === k);
  }

  function toggleTrack(tr: Track) {
    const k = trackKey(tr);
    if (hasTrack(tr)) {
      selectedTracks = selectedTracks.filter((x) => trackKey(x) !== k);
    } else {
      selectedTracks = [...selectedTracks, tr];
    }
  }

  const genres = $derived((catalog?.genres ?? []).slice(0, 10));

  const visibleArtists = $derived.by<OnboardingArtist[]>(() => {
    const q = norm(query);
    const out: OnboardingArtist[] = [];
    const seen = new Set<string>();
    const push = (a: OnboardingArtist) => {
      const k = norm(a.name);
      if (!k || seen.has(k)) return;
      seen.add(k);
      out.push(a);
    };

    if (q.length >= 2) {
      for (const sa of searchArtists) {
        push({
          name: sa.name,
          genre: "",
          photo: sa.track?.cover_url ?? null,
          sample_track: sa.track ?? null,
        });
      }
      for (const st of searchTracks) {
        const name = primaryArtist(st.artist);
        if (norm(name).includes(q)) {
          push({ name, genre: "", photo: st.cover_url ?? null, sample_track: st });
        }
      }
      for (const a of catalog?.artists ?? []) {
        if (norm(a.name).includes(q) || norm(a.genre).includes(q)) push(a);
      }
      // Allow adding the typed name directly if nothing matched yet.
      if (!out.length && !searchLoading) {
        push({ name: query.trim(), genre: "", photo: null, sample_track: null });
      }
      return out.slice(0, 24);
    }

    for (const a of catalog?.artists ?? []) {
      if (genre !== "all" && norm(a.genre) !== norm(genre)) continue;
      push(a);
    }
    return out;
  });

  const visibleTracks = $derived.by<Track[]>(() => {
    const q = norm(query);
    const out: Track[] = [];
    const seen = new Set<string>();
    const push = (tr: Track) => {
      const k = trackKey(tr);
      if (seen.has(k)) return;
      seen.add(k);
      out.push(tr);
    };
    if (q.length >= 2) {
      for (const tr of searchTracks) push(tr);
      for (const tr of catalog?.tracks ?? []) {
        if (norm(tr.title).includes(q) || norm(tr.artist).includes(q)) push(tr);
      }
      return out.slice(0, 24);
    }
    for (const tr of catalog?.tracks ?? []) push(tr);
    for (const a of catalog?.artists ?? []) {
      if (a.sample_track) push(a.sample_track);
    }
    return out.slice(0, 24);
  });

  function stopKeys(e: KeyboardEvent) {
    // Keep Arrow/Enter/Escape inside the search field from flipping Onboarding pages.
    if (e.key === "ArrowLeft" || e.key === "ArrowRight" || e.key === "Enter") {
      e.stopPropagation();
    } else if (e.key === "Escape" && query) {
      e.stopPropagation();
      query = "";
      onQueryInput();
    }
  }
</script>

<div class="pk" class:compact>
  <div class="bar">
    <div class="search">
      <Icon name="search" size={14} color="var(--dim)" />
      <input
        type="text"
        bind:value={query}
        oninput={onQueryInput}
        onkeydown={stopKeys}
        placeholder={t(mode === "artists" ? "taste.picker.search_artists" : "taste.picker.search_songs")}
        spellcheck="false"
      />
      {#if query}
        <button type="button" class="clr" aria-label={t("tip.x")} onclick={() => { query = ""; onQueryInput(); }}>
          <Icon name="x" size={13} color="var(--dim)" />
        </button>
      {/if}
    </div>
    <div class="modes" role="tablist">
      <button type="button" class="mtab" class:on={mode === "artists"} role="tab" aria-selected={mode === "artists"} onclick={() => (mode = "artists")}>
        {t("taste.picker.artists")}
      </button>
      <button type="button" class="mtab" class:on={mode === "songs"} role="tab" aria-selected={mode === "songs"} onclick={() => (mode = "songs")}>
        {t("taste.picker.songs")}
      </button>
    </div>
  </div>

  {#if mode === "artists" && !query.trim() && genres.length}
    <div class="genres">
      <button type="button" class="gchip" class:on={genre === "all"} onclick={() => (genre = "all")}>{t("taste.picker.all")}</button>
      {#each genres as g (g)}
        <button type="button" class="gchip" class:on={genre === g} onclick={() => (genre = genre === g ? "all" : g)}>{g}</button>
      {/each}
    </div>
  {/if}

  <div class="grid-scroll">
    {#if loadingCatalog && !visibleArtists.length && !visibleTracks.length}
      <div class="empty">{t("common.loading")}</div>
    {:else if mode === "artists"}
      <div class="agrid">
        {#each visibleArtists as a (a.name.toLowerCase())}
          {@const on = hasArtist(a.name)}
          <button type="button" class="acard" class:on onclick={() => toggleArtist(a.name, a.genre)}>
            <span class="av">
              {#if a.photo}
                <img src={a.photo} alt="" loading="lazy" />
              {:else}
                <ArtistAvatar name={a.name} track={a.sample_track ?? null} size={compact ? 30 : 36} iconSize={14} />
              {/if}
              {#if on}
                <span class="chk"><Icon name="check" size={12} color="#000" /></span>
              {/if}
            </span>
            <span class="atx">
              <span class="an trunc">{a.name}</span>
              {#if a.genre}<span class="ag trunc">{a.genre}</span>{/if}
            </span>
          </button>
        {/each}
      </div>
    {:else}
      {#if !visibleTracks.length}
        <div class="empty">
          {searchLoading ? t("common.loading") : t("taste.picker.songs_hint")}
        </div>
      {:else}
        <div class="sgrid">
          {#each visibleTracks as tr (trackKey(tr))}
            {@const on = hasTrack(tr)}
            <button type="button" class="scard" class:on onclick={() => toggleTrack(tr)}>
              <span class="cov">
                <Cover track={tr} size={compact ? 34 : 40} radius={6} />
                {#if on}
                  <span class="chk"><Icon name="check" size={12} color="#000" /></span>
                {/if}
              </span>
              <span class="atx">
                <span class="an trunc">{tr.title}</span>
                <span class="ag trunc">{tr.artist}</span>
              </span>
            </button>
          {/each}
        </div>
      {/if}
    {/if}
  </div>

  <div class="selbar">
    <span class="status" class:ready={totalPicked >= 3}>
      <Icon name={totalPicked >= 3 ? "circle-check" : "sparkles"} size={13} color={totalPicked >= 3 ? "var(--accent)" : "var(--dim)"} />
      {#if totalPicked === 0}
        {t("taste.picker.pick_hint")}
      {:else if totalPicked < 3}
        {tf("taste.picker.picked_more", { n: totalPicked, left: 3 - totalPicked })}
      {:else}
        {tf("taste.picker.picked_ready", { n: totalPicked })}
      {/if}
    </span>
    {#if totalPicked > 0}
      <div class="picked-pills">
        {#each selectedArtists as name (name)}
          <button type="button" class="ppill" onclick={() => toggleArtist(name)}>
            <span class="trunc">{name}</span>
            <Icon name="x" size={10} color="var(--dim)" />
          </button>
        {/each}
        {#each selectedTracks as tr (trackKey(tr))}
          <button type="button" class="ppill" onclick={() => toggleTrack(tr)}>
            <span class="trunc">{tr.title}</span>
            <Icon name="x" size={10} color="var(--dim)" />
          </button>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .pk { display: flex; flex-direction: column; gap: 10px; width: 100%; height: 100%; padding: 12px; min-height: 0; }
  .pk.compact { gap: 8px; padding: 10px; }
  .bar { display: flex; align-items: center; gap: 8px; flex: none; }
  .search {
    flex: 1;
    min-width: 0;
    height: 34px;
    padding: 0 10px;
    border-radius: 9px;
    background: rgba(255, 255, 255, .07);
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .search input {
    flex: 1;
    min-width: 0;
    background: none;
    border: 0;
    outline: none;
    color: var(--text);
    font-size: 13px;
    font-family: var(--font);
  }
  .search input::placeholder { color: var(--faint); }
  .clr { width: 20px; height: 20px; border-radius: 50%; display: grid; place-items: center; flex: none; }
  .clr:hover { background: rgba(255, 255, 255, .1); }
  .modes {
    flex: none;
    display: flex;
    padding: 3px;
    border-radius: 9px;
    background: rgba(255, 255, 255, .06);
    gap: 2px;
  }
  .mtab {
    height: 28px;
    padding: 0 11px;
    border-radius: 7px;
    font-size: 12.5px;
    font-weight: 500;
    color: var(--dim);
    transition: background-color .14s ease-out, color .14s ease-out;
  }
  .mtab.on { background: rgba(255, 255, 255, .12); color: var(--text); }
  .genres {
    flex: none;
    display: flex;
    gap: 6px;
    overflow-x: auto;
    scrollbar-width: none;
    padding-bottom: 1px;
  }
  .genres::-webkit-scrollbar { display: none; }
  .gchip {
    flex: none;
    height: 25px;
    padding: 0 10px;
    border-radius: 999px;
    background: rgba(255, 255, 255, .06);
    color: var(--dim);
    font-size: 12px;
    font-weight: 500;
    transition: background-color .14s ease-out, color .14s ease-out;
  }
  .gchip:hover { background: rgba(255, 255, 255, .1); color: var(--text); }
  .gchip.on { background: var(--accent); color: #000; font-weight: 600; }
  .grid-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    scrollbar-width: thin;
  }
  .agrid, .sgrid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 6px;
  }
  .compact .agrid, .compact .sgrid {
    grid-template-columns: repeat(auto-fill, minmax(134px, 1fr));
    gap: 5px;
  }
  .acard, .scard {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 6px 9px 6px 6px;
    border-radius: 10px;
    background: rgba(255, 255, 255, .045);
    text-align: left;
    min-width: 0;
    transition: background-color .14s ease-out, transform .14s var(--ease);
  }
  .acard:hover, .scard:hover { background: rgba(255, 255, 255, .09); }
  .acard:active, .scard:active { transform: scale(.97); }
  .acard.on, .scard.on { background: color-mix(in srgb, var(--accent) 22%, rgba(255, 255, 255, .06)); }
  .av, .cov { position: relative; flex: none; display: grid; place-items: center; }
  .av img { width: 36px; height: 36px; border-radius: 50%; object-fit: cover; }
  .compact .av img { width: 30px; height: 30px; }
  .chk {
    position: absolute;
    right: -2px;
    bottom: -2px;
    width: 17px;
    height: 17px;
    border-radius: 50%;
    background: var(--accent);
    display: grid;
    place-items: center;
  }
  .atx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .an { font-size: 12.5px; font-weight: 500; color: var(--text); }
  .ag { font-size: 11px; color: var(--dim); }
  .empty { height: 100%; min-height: 80px; display: grid; place-items: center; font-size: 13px; color: var(--dim); text-align: center; padding: 12px; }
  .selbar {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding-top: 4px;
    min-height: 26px;
  }
  .status { display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--dim); flex: none; }
  .status.ready { color: var(--text); font-weight: 500; }
  .picked-pills { display: flex; align-items: center; gap: 5px; overflow-x: auto; scrollbar-width: none; min-width: 0; }
  .picked-pills::-webkit-scrollbar { display: none; }
  .ppill {
    flex: none;
    max-width: 110px;
    height: 22px;
    padding: 0 7px;
    border-radius: 999px;
    background: rgba(255, 255, 255, .09);
    color: var(--text);
    font-size: 11.5px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .ppill:hover { background: rgba(255, 255, 255, .15); }
</style>
