<script lang="ts">
  // Mobile bottom sheet for searching and switching music videos for the playing song.
  import { fade } from "svelte/transition";
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { fmtTime } from "../fmt";
  import type { VideoCandidate } from "../api/types";
  import { videoState, type VideoFilter } from "../pages/nowplaying/video-state.svelte";
  import { drag, phone, sheetIn, sheetOut, shouldClose } from "./cstate.svelte";

  let pull = $state(0);
  let listEl: HTMLDivElement | undefined = $state();

  const cur = $derived(app.player.current);
  const sel = $derived(videoState.selected);
  const list = $derived(videoState.filtered);
  const close = () => (phone.videoPickerOpen = false);

  const FILTERS: { id: VideoFilter; label: string }[] = [
    { id: "all", label: "video.filter.all" },
    { id: "official", label: "video.filter.official" },
    { id: "live", label: "video.filter.live" },
    { id: "lyric", label: "video.filter.lyric" },
  ];

  function submitSearch(e?: Event) {
    e?.preventDefault();
    if (!cur) return;
    void videoState.search(cur, videoState.query, false);
  }

  function clearSearch() {
    videoState.query = "";
    if (cur) void videoState.search(cur, null, false);
  }

  function pick(v: VideoCandidate) {
    void videoState.select(v, cur, true);
    close();
  }
</script>

<div class="dim" transition:fade={{ duration: 180 }} onclick={close} onkeydown={() => {}} role="presentation"></div>

<div
  class="sheet"
  style:transform="translateY({pull}px)"
  class:settle={pull === 0}
  in:sheetIn|global={{ duration: 280 }}
  out:sheetOut|global={{ duration: 220, radius: true }}
>
  <div
    class="grab-area"
    use:drag={{
      axis: "y",
      move: (_, dy) => (pull = Math.max(0, dy)),
      end: (_, dy, _vx, vy) => {
        if (shouldClose(dy, vy)) { close(); return; }
        pull = 0;
      },
    }}
  >
    <span class="grab"></span>
    <div class="head">
      <span class="ht">{t("video.results_title")}</span>
      {#if !videoState.loading && list.length > 0}
        <span class="cnt mono">{list.length}</span>
      {/if}
      <IconButton name="x" size={18} box={34} notip onclick={close} />
    </div>
  </div>

  <form class="search" onsubmit={submitSearch}>
    <Icon name="search" size={15} color="var(--dim)" />
    <input
      type="text"
      bind:value={videoState.query}
      placeholder={t("video.search_placeholder")}
      aria-label={t("video.search_placeholder")}
    />
    {#if videoState.query}
      <button type="button" class="clr" aria-label="Clear" onclick={clearSearch}>
        <Icon name="x" size={14} />
      </button>
    {/if}
  </form>

  <div class="filters">
    {#each FILTERS as f}
      <button
        type="button"
        class="fpill"
        class:on={videoState.filter === f.id}
        onclick={() => (videoState.filter = f.id)}
      >
        {t(f.label)}
      </button>
    {/each}
  </div>

  <div
    class="list scroll"
    bind:this={listEl}
    use:drag={{
      axis: "y",
      onlyAtTop: () => (listEl?.scrollTop ?? 0) <= 0,
      move: (_, dy) => (pull = Math.max(0, dy)),
      end: (_, dy, _vx, vy) => {
        if (shouldClose(dy, vy)) { close(); return; }
        pull = 0;
      },
    }}
  >
    {#if videoState.loading}
      {#each [0, 1, 2, 3, 4] as _}
        <div class="row sk">
          <Skeleton w="96px" h={54} radius={6} />
          <div class="ri">
            <Skeleton w="84%" h={14} radius={4} />
            <Skeleton w="54%" h={11} radius={4} />
          </div>
        </div>
      {/each}
    {:else if list.length === 0}
      <div class="empty">
        <Icon name="tv" size={24} color="var(--faint)" />
        <div class="et">{t("video.none.title")}</div>
        <div class="es">{t("video.none.sub")}</div>
      </div>
    {:else}
      {#each list as v (v.id)}
        {@const active = sel?.id === v.id}
        <button type="button" class="row" class:active onclick={() => pick(v)}>
          <div class="thumb">
            {#if v.thumbnail}
              <img src={v.thumbnail} alt="" loading="lazy" />
            {:else}
              <div class="tph"><Icon name="tv" size={18} color="var(--faint)" /></div>
            {/if}
            {#if v.duration_ms}
              <span class="dur mono">{fmtTime(v.duration_ms / 1000)}</span>
            {/if}
            {#if active}
              <span class="dot"><Icon name="play" size={10} color="#0a0a0c" /></span>
            {/if}
          </div>
          <div class="ri">
            <span class="rt">{v.title}</span>
            <div class="rm">
              <span class="rc trunc">{v.channel}</span>
              <span class="rk">{t(videoState.kindLabelKey(v.kind))}</span>
            </div>
          </div>
        </button>
      {/each}
    {/if}
  </div>
</div>

<style>
  .dim {
    position: fixed;
    inset: 0;
    z-index: 64;
    background: rgba(0, 0, 0, .62);
  }
  .sheet {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 65;
    height: 82%;
    max-height: 86vh;
    border-radius: 16px 16px 0 0;
    background: #121215;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    padding-bottom: var(--safe-bottom, env(safe-area-inset-bottom));
  }
  .sheet.settle { transition: transform .22s var(--ease); }

  .grab-area { flex: none; padding: 10px 16px 6px; display: flex; flex-direction: column; align-items: center; touch-action: none; }
  .grab { width: 36px; height: 4px; border-radius: 2px; background: rgba(255, 255, 255, .2); margin-bottom: 8px; }
  .head { width: 100%; display: flex; align-items: center; gap: 8px; }
  .ht { font-size: 16px; font-weight: 700; color: var(--text); }
  .cnt {
    height: 20px;
    padding: 0 7px;
    border-radius: 99px;
    background: #222228;
    color: var(--dim);
    font-size: 11px;
    display: inline-flex;
    align-items: center;
  }
  .head :global(.ib) { margin-left: auto; margin-right: -6px; }

  .search {
    flex: none;
    margin: 4px 16px 10px;
    height: 38px;
    padding: 0 10px 0 12px;
    border-radius: 10px;
    background: #1c1c21;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .search input { flex: 1; min-width: 0; background: transparent; border: none; outline: none; color: var(--text); font-size: 14px; }
  .search input::placeholder { color: var(--faint); }
  .clr { width: 26px; height: 26px; border-radius: 7px; display: grid; place-items: center; color: var(--dim); }

  .filters {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 16px 10px;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .filters::-webkit-scrollbar { display: none; }
  .fpill {
    flex: none;
    height: 28px;
    padding: 0 12px;
    border-radius: 8px;
    background: #1c1c21;
    color: var(--dim);
    font-size: 12.5px;
    font-weight: 500;
  }
  .fpill.on { background: var(--accent); color: #0a0a0c; font-weight: 600; }

  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 12px 20px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .row {
    width: 100%;
    display: flex;
    align-items: flex-start;
    gap: 11px;
    padding: 8px;
    border-radius: 10px;
    text-align: left;
  }
  .row:active { background: #1e1e24; }
  .row.active { background: #26222b; }

  .thumb {
    position: relative;
    flex: none;
    width: 96px;
    height: 54px;
    border-radius: 6px;
    overflow: hidden;
    background: #1c1c20;
  }
  .thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .tph { width: 100%; height: 100%; display: grid; place-items: center; }
  .dur {
    position: absolute;
    right: 4px;
    bottom: 4px;
    padding: 1px 5px;
    border-radius: 4px;
    background: rgba(8, 8, 10, .86);
    color: #fff;
    font-size: 10.5px;
  }
  .dot {
    position: absolute;
    left: 5px;
    top: 5px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: var(--accent);
    display: grid;
    place-items: center;
    padding-left: 1px;
  }

  .ri { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 5px; }
  .rt {
    font-size: 13.5px;
    font-weight: 500;
    line-height: 1.3;
    color: var(--text);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .row.active .rt { color: var(--accent); font-weight: 600; }
  .rm { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .rc { font-size: 12px; color: var(--dim); min-width: 0; }
  .rk {
    flex: none;
    padding: 1px 6px;
    border-radius: 4px;
    background: #1f1f25;
    color: var(--dim);
    font-size: 11px;
    font-weight: 500;
  }
  .row.active .rk { background: rgba(255, 255, 255, .1); color: var(--text); }

  .empty { padding: 36px 16px; display: flex; flex-direction: column; align-items: center; gap: 6px; text-align: center; }
  .et { font-size: 14px; font-weight: 600; color: var(--text); margin-top: 4px; }
  .es { font-size: 12.5px; color: var(--dim); }
</style>
