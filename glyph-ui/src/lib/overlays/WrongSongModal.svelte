<script lang="ts">
  // "Not the right one?" candidate picker modal (desktop dialog + mobile bottom
  // sheet). Lets the listener audition 30-second previews of alternative matches,
  // records their wrong-song vote without switching everyone else globally until
  // consensus + server verification confirm it, and immediately downloads their
  // selected version locally.
  import { fade, scale, slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Cover from "../components/Cover.svelte";
  import Badges from "../components/Badges.svelte";
  import Button from "../components/Button.svelte";
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import PlayingBars from "../components/PlayingBars.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import ErrorCard from "../components/ErrorCard.svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { onWeb } from "../web";
  import { t } from "../i18n.svelte";
  import { fmtMs, fmtTime } from "../fmt";
  import { fadeEdges, press } from "../ui.svelte";
  import { drag, sheetIn, sheetOut, shouldClose } from "../compact/cstate.svelte";
  import type {
    MatchAlternative,
    MatchAlternativesResponse,
    ProviderId,
    Track,
    WrongSongPreviewState,
  } from "../api/types";

  const PREVIEW_MAX = 30;
  const RING_R = 19;
  const RING_C = 2 * Math.PI * RING_R;

  const track = $derived(app.wrongSong!);
  const compact = $derived(app.compact);

  let query = $state("");
  let loading = $state(true);
  let error = $state<string | null>(null);
  let currentSource = $state<ProviderId | null>(null);
  let currentId = $state<string | null>(null);
  let alternatives = $state<MatchAlternative[]>([]);
  let selectedKey = $state<string | null>(null);
  let saving = $state(false);
  let pull = $state(0);
  let loadSeq = 0;

  let preview = $state<WrongSongPreviewState>({
    key: null,
    status: "stopped",
    position: 0,
    duration: PREVIEW_MAX,
    error: null,
  });

  function altKey(tr: Track): string {
    return `${tr.source}:${tr.id}`;
  }

  const selected = $derived(
    selectedKey ? alternatives.find((a) => altKey(a.track) === selectedKey) ?? null : null,
  );

  async function fetchAlternatives(customQuery?: string) {
    const seq = ++loadSeq;
    loading = true;
    error = null;
    try {
      const q = customQuery?.trim() ? customQuery.trim() : null;
      const res = await backend().call<MatchAlternativesResponse>("wrong_song_alternatives", {
        track: $state.snapshot(track),
        query: q,
      });
      if (seq !== loadSeq) return;
      currentSource = res.current_source ?? null;
      currentId = res.current_id ?? null;
      alternatives = res.alternatives ?? [];
      // Pre-select the top non-current alternative when available so the action is one click away.
      const firstNonCurrent = alternatives.find((a) => a.tag !== "current") ?? alternatives[0] ?? null;
      selectedKey = firstNonCurrent ? altKey(firstNonCurrent.track) : null;
    } catch (e) {
      if (seq !== loadSeq) return;
      error = e instanceof Error ? e.message : String(e);
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }

  $effect(() => {
    void track;
    query = "";
    selectedKey = null;
    void fetchAlternatives();
  });

  $effect(() => {
    const off = backend().on("wrong_song_preview", (p) => {
      preview = p;
    });
    return () => {
      off();
      void backend().call("wrong_song_preview_stop").catch(() => {});
    };
  });

  function stopPreview() {
    preview = { key: null, status: "stopped", position: 0, duration: PREVIEW_MAX, error: null };
    void backend().call("wrong_song_preview_stop").catch(() => {});
  }

  async function togglePreview(alt: MatchAlternative, e: MouseEvent) {
    e.stopPropagation();
    const k = altKey(alt.track);
    if (preview.key === k && (preview.status === "playing" || preview.status === "loading")) {
      stopPreview();
      return;
    }
    selectedKey = k;
    preview = { key: k, status: "loading", position: 0, duration: PREVIEW_MAX, error: null };
    try {
      await backend().call("wrong_song_preview", {
        track: $state.snapshot(alt.track),
        preview_url: alt.preview_url ?? null,
      });
    } catch (err) {
      preview = {
        key: k,
        status: "error",
        position: 0,
        duration: PREVIEW_MAX,
        error: err instanceof Error ? err.message : String(err),
      };
    }
  }

  function close() {
    if (saving) return;
    stopPreview();
    app.wrongSong = null;
  }

  async function confirm() {
    if (!selected || saving) return;
    saving = true;
    stopPreview();
    try {
      await backend().call("wrong_song_select", {
        original: $state.snapshot(track),
        chosen: $state.snapshot(selected.track),
        wrong_provider: currentSource,
        wrong_id: currentId,
      });
      app.wrongSong = null;
    } catch (err) {
      app.toast(err instanceof Error ? err.message : String(err));
    } finally {
      saving = false;
    }
  }

  function onSearchSubmit(e: SubmitEvent) {
    e.preventDefault();
    stopPreview();
    void fetchAlternatives(query);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  }

  function tagLabel(tag: string | null | undefined): string | null {
    if (!tag) return null;
    if (tag === "current") return t("wrong_song.tag.current");
    if (tag === "topic") return t("wrong_song.tag.topic");
    if (tag === "official_audio") return t("wrong_song.tag.official_audio");
    return null;
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#snippet content()}
  <div class="head">
    <div class="head-tx">
      <div class="title">{t("wrong_song.title")}</div>
      <div class="sub">{t("wrong_song.sub")}</div>
    </div>
    {#if !compact}
      <IconButton name="x" size={16} box={30} label={t("common.close")} onclick={close} />
    {/if}
  </div>

  <!-- The song being fixed -->
  <div class="orig">
    <Cover {track} size={40} radius={6} />
    <div class="orig-tx">
      <div class="orig-t trunc">{track.title}</div>
      <div class="orig-a">
        <span class="trunc">{track.artist}</span>
        {#if currentSource}
          <Badges source={currentSource} />
        {:else}
          <Badges {track} />
        {/if}
        {#if track.duration_ms}
          <span class="dur mono">{fmtMs(track.duration_ms)}</span>
        {/if}
      </div>
    </div>
  </div>

  <!-- Refine search -->
  <form class="search" onsubmit={onSearchSubmit}>
    <span class="s-ic" aria-hidden="true"><Icon name="search" size={14} color="var(--faint)" /></span>
    <input
      type="text"
      bind:value={query}
      placeholder={t("wrong_song.search_placeholder")}
      aria-label={t("wrong_song.search_placeholder")}
    />
    <button type="submit" class="s-btn" use:press disabled={loading}>
      {t("wrong_song.search_btn")}
    </button>
  </form>

  <!-- Alternatives list -->
  <div class="list scroll" use:fadeEdges={{ top: 10, bottom: 10, minOverflow: 8 }}>
    {#if loading}
      <div class="skels" aria-label={t("wrong_song.loading")}>
        {#each Array(4) as _, i (i)}
          <div class="skel-row">
            <Skeleton w={44} h={44} radius={6} id="ws-c-{i}" />
            <div class="skel-tx">
              <Skeleton w="{58 + (i % 3) * 12}%" h={13} id="ws-t-{i}" />
              <Skeleton w="{36 + (i % 2) * 14}%" h={11} id="ws-a-{i}" />
            </div>
          </div>
        {/each}
      </div>
    {:else if error}
      <div class="state-pad">
        <ErrorCard title={t("wrong_song.error")} body={t("wrong_song.empty_sub")} detail={error} compact onretry={() => fetchAlternatives(query)} />
      </div>
    {:else if alternatives.length === 0}
      <div class="empty">
        <Icon name="search" size={22} color="var(--faint)" />
        <div class="em-t">{t("wrong_song.empty")}</div>
        <div class="em-s">{t("wrong_song.empty_sub")}</div>
      </div>
    {:else}
      {#each alternatives as alt (altKey(alt.track))}
        {@const k = altKey(alt.track)}
        {@const isSelected = selectedKey === k}
        {@const isThisPreview = preview.key === k}
        {@const isPlaying = isThisPreview && preview.status === "playing"}
        {@const isLoading = isThisPreview && preview.status === "loading"}
        {@const isErr = isThisPreview && preview.status === "error"}
        {@const dur = Math.max(1, Math.min(PREVIEW_MAX, preview.duration || PREVIEW_MAX))}
        {@const frac = isThisPreview ? Math.min(1, Math.max(0, preview.position / dur)) : 0}
        {@const badge = tagLabel(alt.tag)}
        <div
          class="row"
          class:sel={isSelected}
          class:pv-on={isPlaying || isLoading}
          role="option"
          aria-selected={isSelected}
          tabindex="0"
          onclick={() => (selectedKey = k)}
          onkeydown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              selectedKey = k;
            }
          }}
        >
          <div class="row-main">
            <!-- Cover with 30-second preview button & progress ring -->
            <button
              type="button"
              class="pv-btn"
              class:active={isPlaying || isLoading}
              aria-label={isPlaying ? t("wrong_song.preview_stop") : t("wrong_song.preview_play")}
              onclick={(e) => togglePreview(alt, e)}
              use:press
            >
              <Cover track={alt.track} size={44} radius={6} />
              <span class="pv-scrim" class:show={isPlaying || isLoading}>
                {#if isLoading}
                  <span class="spin" aria-hidden="true"></span>
                {:else if isPlaying}
                  <Icon name="square" size={12} color="#fff" />
                {:else}
                  <Icon name="play" size={14} color="#fff" />
                {/if}
              </span>
              {#if isPlaying}
                <svg class="ring" viewBox="0 0 44 44" aria-hidden="true">
                  <circle class="ring-bg" cx="22" cy="22" r={RING_R} />
                  <circle
                    class="ring-fg"
                    cx="22"
                    cy="22"
                    r={RING_R}
                    stroke-dasharray={RING_C}
                    stroke-dashoffset={RING_C * (1 - frac)}
                  />
                </svg>
              {/if}
            </button>

            <!-- Candidate metadata -->
            <div class="meta">
              <div class="m-top">
                <span class="m-t trunc">{alt.track.title}</span>
                {#if badge}
                  <span class="tag" class:cur={alt.tag === "current"} class:topic={alt.tag === "topic"}>
                    {badge}
                  </span>
                {/if}
              </div>
              <div class="m-sub">
                <span class="m-a trunc">{alt.track.artist}</span>
                <Badges source={alt.track.source} />
                {#if alt.track.duration_ms}
                  <span class="dur mono">{fmtMs(alt.track.duration_ms)}</span>
                {/if}
              </div>
            </div>

            <!-- Radio selection circle -->
            <span class="mark" class:on={isSelected} aria-hidden="true">
              {#if isSelected}<Icon name="check" size={12} />{/if}
            </span>
          </div>

          <!-- Active 30s preview strip -->
          {#if isPlaying || isLoading || isErr}
            <div class="pv-strip" transition:slide|local={{ duration: 180, easing: cubicOut }}>
              {#if isErr}
                <span class="pv-err trunc">{preview.error ?? t("wrong_song.error")}</span>
              {:else}
                <span class="pv-bars"><PlayingBars active={isPlaying} size={12} /></span>
                <span class="pv-badge">{t("wrong_song.preview_badge")}</span>
                <div class="pv-track" aria-hidden="true">
                  <div class="pv-fill" style:width="{(frac * 100).toFixed(1)}%"></div>
                </div>
                <span class="pv-tm mono">{fmtTime(preview.position)} / 0:30</span>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    {/if}
  </div>

  <!-- Consensus & privacy notice -->
  <div class="notice">
    <span class="n-ic" aria-hidden="true"><Icon name="info" size={14} color="var(--dim)" /></span>
    <span>{t("wrong_song.notice")}</span>
  </div>

  <!-- Footer buttons -->
  <div class="foot">
    <span class="flex1"></span>
    <Button onclick={close} disabled={saving}>{t("dialog.cancel")}</Button>
    <Button kind="primary" onclick={confirm} disabled={!selected || saving}>
      {#if saving}
        {t("wrong_song.saving")}
      {:else if onWeb()}
        {t("wrong_song.confirm_web")}
      {:else}
        <Icon name="download" size={14} />
        <span>{t("wrong_song.confirm")}</span>
      {/if}
    </Button>
  </div>
{/snippet}

{#if compact}
  <div
    class="bs-dim"
    transition:fade|global={{ duration: 180 }}
    onpointerdown={close}
    role="presentation"
  >
    <div
      class="bs-panel"
      data-part="dialog"
      style:transform="translateY({pull}px)"
      class:settle={pull === 0}
      in:sheetIn|global={{ duration: 340 }}
      out:sheetOut|global={{ duration: 210 }}
      onpointerdown={(e) => e.stopPropagation()}
      role="dialog"
      aria-modal="true"
      aria-label={t("wrong_song.title")}
      tabindex="-1"
    >
      <div
        class="bs-grab"
        use:drag={{
          axis: "y",
          move: (_, dy) => (pull = Math.max(0, dy)),
          end: (_, dy, _vx, vy) => {
            if (shouldClose(dy, vy)) {
              close();
              return;
            }
            pull = 0;
          },
        }}
      >
        <span class="hdl"></span>
      </div>
      <div class="bs-inner">{@render content()}</div>
    </div>
  </div>
{:else}
  <div class="layer" transition:fade|global={{ duration: 160 }}>
    <div class="tdim"></div>
    <div
      class="bd"
      onpointerdown={(e) => e.target === e.currentTarget && close()}
      role="presentation"
    >
      <div
        class="panel"
        data-part="dialog"
        transition:scale|global={{ start: 0.97, duration: 180, easing: cubicOut }}
        role="dialog"
        aria-modal="true"
        aria-label={t("wrong_song.title")}
        tabindex="-1"
      >
        {@render content()}
      </div>
    </div>
  </div>
{/if}

<style>
  .layer { position: fixed; inset: 0; z-index: 90; }
  .tdim { position: absolute; left: 0; right: 0; top: 0; height: var(--titlebar-h); background: rgba(0, 0, 0, .52); pointer-events: none; }
  .bd { position: absolute; top: var(--titlebar-h); left: 0; right: 0; bottom: 0; padding: 16px; padding-bottom: calc(16px + var(--titlebar-h)); background: rgba(0, 0, 0, .52); display: flex; align-items: center; justify-content: center; }
  .panel { width: 560px; max-width: calc(100vw - 32px); max-height: min(680px, calc(100vh - var(--titlebar-h) - 48px)); padding: 20px; border-radius: 12px; background: #161616; display: flex; flex-direction: column; gap: 12px; font-family: var(--font); }

  .bs-dim { position: fixed; inset: 0; z-index: 90; background: rgba(0, 0, 0, .55); }
  .bs-panel { position: absolute; left: 0; right: 0; bottom: 0; max-height: min(88vh, calc(100vh - var(--safe-top, env(safe-area-inset-top)) - 16px)); border-radius: 16px 16px 0 0; background: #141414; padding-bottom: max(12px, var(--safe-bottom, env(safe-area-inset-bottom)), var(--ime-bottom, 0px)); display: flex; flex-direction: column; overflow: hidden; }
  .bs-panel.settle { transition: transform .24s var(--ease); }
  .bs-grab { flex: none; touch-action: none; padding: 10px 0 4px; }
  .hdl { display: block; width: 36px; height: 4px; margin: 0 auto; border-radius: 2px; background: rgba(255, 255, 255, .25); }
  .bs-inner { flex: 1; min-height: 0; padding: 6px 16px 10px; display: flex; flex-direction: column; gap: 14px; }
  .bs-inner .search { margin: 2px 0 4px; }

  .head { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; }
  .head-tx { min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .title { font-size: 16px; font-weight: 600; color: rgba(255, 255, 255, .96); }
  .sub { font-size: 12.5px; line-height: 1.4; color: var(--dim); }

  .orig { display: flex; align-items: center; gap: 10px; padding: 8px 10px; border-radius: 8px; background: rgba(255, 255, 255, .045); }
  .orig-tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .orig-t { font-size: 13.5px; font-weight: 600; color: rgba(255, 255, 255, .94); }
  .orig-a { display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--dim); min-width: 0; }

  .search { display: flex; align-items: center; gap: 8px; height: 36px; padding: 0 6px 0 10px; border-radius: 8px; background: rgba(255, 255, 255, .055); }
  .search:focus-within { background: rgba(255, 255, 255, .08); }
  .s-ic { flex: none; display: grid; place-items: center; }
  .search input { flex: 1; min-width: 0; height: 100%; background: none; border: 0; outline: 0; color: rgba(255, 255, 255, .94); font-size: 13px; font-family: inherit; }
  .search input::placeholder { color: var(--faint); }
  .s-btn { flex: none; height: 26px; padding: 0 10px; border-radius: 6px; background: rgba(255, 255, 255, .08); color: rgba(255, 255, 255, .86); font-size: 12px; font-weight: 500; cursor: pointer; transition: background-color .14s ease-out; }
  .s-btn:hover:not(:disabled) { background: rgba(255, 255, 255, .14); }
  .s-btn:disabled { opacity: .45; cursor: default; }

  .list { flex: 1; min-height: 180px; max-height: 360px; overflow-y: auto; display: flex; flex-direction: column; gap: 4px; padding: 2px 0; }
  .skels { display: flex; flex-direction: column; gap: 8px; padding: 4px 2px; }
  .skel-row { display: flex; align-items: center; gap: 10px; padding: 6px 8px; }
  .skel-tx { flex: 1; display: flex; flex-direction: column; gap: 6px; }
  .state-pad { padding: 12px 0; }
  .empty { padding: 36px 16px; display: flex; flex-direction: column; align-items: center; text-align: center; gap: 6px; }
  .em-t { font-size: 13.5px; font-weight: 500; color: rgba(255, 255, 255, .85); }
  .em-s { font-size: 12px; color: var(--dim); max-width: 300px; }

  .row { flex: none; border-radius: 8px; padding: 7px 10px; background: transparent; cursor: pointer; display: flex; flex-direction: column; gap: 8px; transition: background-color .14s ease-out; outline: none; }
  .row:hover { background: rgba(255, 255, 255, .055); }
  .row:focus-visible { background: rgba(255, 255, 255, .08); }
  .row.sel { background: color-mix(in srgb, var(--accent) 13%, rgba(255, 255, 255, .03)); }
  .row.pv-on { background: color-mix(in srgb, var(--accent) 10%, rgba(255, 255, 255, .04)); }
  .row-main { display: flex; align-items: center; gap: 10px; min-width: 0; }

  .pv-btn { position: relative; flex: none; width: 44px; height: 44px; border-radius: 6px; padding: 0; overflow: hidden; cursor: pointer; display: grid; place-items: center; }
  .pv-scrim { position: absolute; inset: 0; background: rgba(0, 0, 0, .52); display: grid; place-items: center; opacity: 0; transition: opacity .15s ease-out; }
  .pv-btn:hover .pv-scrim, .row:hover .pv-scrim, .pv-scrim.show { opacity: 1; }
  .ring { position: absolute; inset: 0; width: 44px; height: 44px; transform: rotate(-90deg); pointer-events: none; }
  .ring-bg { fill: none; stroke: rgba(255, 255, 255, .18); stroke-width: 2.5; }
  .ring-fg { fill: none; stroke: var(--accent); stroke-width: 2.5; stroke-linecap: round; transition: stroke-dashoffset .14s linear; }
  .spin { width: 14px; height: 14px; border-radius: 50%; border: 2px solid rgba(255, 255, 255, .25); border-top-color: #fff; animation: spin .7s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }

  .meta { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .m-top { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .m-t { font-size: 13.5px; font-weight: 500; color: rgba(255, 255, 255, .94); }
  .m-sub { display: flex; align-items: center; gap: 6px; min-width: 0; font-size: 12px; color: var(--dim); }
  .m-a { min-width: 0; }
  .dur { flex: none; font-size: 11px; color: var(--faint); }

  .tag { flex: none; font-size: 11px; font-weight: 500; color: var(--dim); background: rgba(255, 255, 255, .07); padding: 2px 7px; border-radius: 999px; }
  .tag.topic { color: var(--accent); background: color-mix(in srgb, var(--accent) 14%, transparent); }
  .tag.cur { color: rgba(255, 255, 255, .65); background: rgba(255, 255, 255, .09); }

  .mark { flex: none; width: 18px; height: 18px; border-radius: 50%; box-shadow: inset 0 0 0 1.5px rgba(255, 255, 255, .3); display: grid; place-items: center; color: #0d0d0d; transition: background-color .14s ease-out, box-shadow .14s ease-out; }
  .mark.on { background: var(--accent); box-shadow: none; animation: mark-pop .22s var(--ease); }
  @keyframes mark-pop { 0% { transform: scale(.72); } 60% { transform: scale(1.14); } 100% { transform: scale(1); } }

  .pv-strip { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: 6px; background: rgba(0, 0, 0, .28); }
  .pv-bars { flex: none; display: flex; align-items: center; color: var(--accent); }
  .pv-badge { flex: none; font-size: 11.5px; font-weight: 500; color: var(--accent); }
  .pv-track { flex: 1; height: 3px; border-radius: 2px; background: rgba(255, 255, 255, .14); overflow: hidden; }
  .pv-fill { height: 100%; background: var(--accent); border-radius: 2px; transition: width .15s linear; }
  .pv-tm { flex: none; font-size: 11px; color: var(--dim); font-variant-numeric: tabular-nums; }
  .pv-err { font-size: 11.5px; color: var(--warn); }

  .notice { display: flex; align-items: flex-start; gap: 8px; padding: 8px 10px; border-radius: 8px; background: rgba(255, 255, 255, .035); font-size: 11.5px; line-height: 1.4; color: var(--dim); }
  .n-ic { flex: none; margin-top: 1px; display: grid; place-items: center; }

  .foot { display: flex; align-items: center; gap: 8px; padding-top: 2px; }
  .foot :global(button) { display: inline-flex; align-items: center; gap: 6px; }
</style>
