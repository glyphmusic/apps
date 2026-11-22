<script lang="ts">
  // Desktop Now Playing video mode: 16:9 Glyph video player on the left,
  // collapsible search & video picker panel on the right.
  import { onDestroy } from "svelte";
  import Icon from "../../components/Icon.svelte";
  import IconButton from "../../components/IconButton.svelte";
  import ToggleButton from "../../components/ToggleButton.svelte";
  import SongTitle from "../../components/SongTitle.svelte";
  import Skeleton from "../../components/Skeleton.svelte";
  import VideoPlayer from "./VideoPlayer.svelte";
  import { videoState, type VideoFilter } from "./video-state.svelte";
  import { app } from "../../state.svelte";
  import { saveMenu } from "../lists/actions";
  import { t } from "../../i18n.svelte";
  import { fmtTime } from "../../fmt";
  import { rating, type VideoCandidate } from "../../api/types";

  const cur = $derived(app.player.current);
  const sel = $derived(videoState.selected);
  const list = $derived(videoState.filtered);

  const FILTERS: { id: VideoFilter; label: string }[] = [
    { id: "all", label: "video.filter.all" },
    { id: "official", label: "video.filter.official" },
    { id: "live", label: "video.filter.live" },
    { id: "lyric", label: "video.filter.lyric" },
  ];

  // Auto-search videos whenever the playing track changes while in Video mode.
  $effect(() => {
    const track = cur;
    if (!track) return;
    void videoState.ensureForTrack(track);
  });

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
  }

  function backToSong() {
    videoState.leave(true);
    app.videoMode = false;
  }

  onDestroy(() => {
    videoState.leave(true);
  });
</script>

{#if cur}
  <div class="vroot">
    <div class="vwrap" class:solo={!videoState.listOpen}>
      <div class="vmain">
        <VideoPlayer onOpenPicker={() => (videoState.listOpen = true)} />

        <div class="vinfo">
          <div class="vmeta">
            <div class="vtitle-row">
              <div class="vtitle"><SongTitle rating={rating(cur)}>{sel?.title ?? cur.title}</SongTitle></div>
              {#if sel}
                <span class="kpill">{t(videoState.kindLabelKey(sel.kind))}</span>
              {/if}
            </div>
            <div class="vsub">
              <button type="button" class="artist" onclick={() => app.openArtistPage(cur.artist)}>
                {cur.artist}
              </button>
              {#if sel?.channel && sel.channel.toLowerCase() !== cur.artist.toLowerCase()}
                <span class="dot">·</span>
                <span class="chan trunc">{sel.channel}</span>
              {/if}
              {#if videoState.pausedAudioForVideo && videoState.playing}
                <span class="dot">·</span>
                <span class="note trunc">{t("video.audio_paused")}</span>
              {/if}
            </div>
          </div>

          <div class="vacts">
            <ToggleButton
              name="heart"
              size={16}
              on={app.isLiked(cur)}
              accent="var(--accent)"
              onclick={() => app.toggleLike(cur)}
              oncontextmenu={saveMenu(cur)}
            />
            <button type="button" class="flat-btn" onclick={backToSong}>
              <Icon name="music-2" size={14} />{t("video.back_to_song")}
            </button>
            <button
              type="button"
              class="flat-btn"
              class:on={videoState.listOpen}
              onclick={() => (videoState.listOpen = !videoState.listOpen)}
            >
              <Icon name="search" size={14} />{t("video.switch")}
            </button>
          </div>
        </div>
      </div>

      {#if videoState.listOpen}
        <aside class="vpanel" aria-label={t("video.results_title")}>
          <div class="phead">
            <span class="ptitle">{t("video.results_title")}</span>
            {#if !videoState.loading && videoState.results.length > 0}
              <span class="pcnt mono">{list.length}</span>
            {/if}
            <IconButton name="x" size={15} label={t("video.toggle_list")} onclick={() => (videoState.listOpen = false)} />
          </div>

          <form class="psearch" onsubmit={submitSearch}>
            <span class="sic"><Icon name="search" size={14} color="var(--dim)" /></span>
            <input
              type="text"
              bind:value={videoState.query}
              placeholder={t("video.search_placeholder")}
              aria-label={t("video.search_placeholder")}
            />
            {#if videoState.query}
              <button type="button" class="clr" aria-label="Clear" onclick={clearSearch}>
                <Icon name="x" size={13} />
              </button>
            {/if}
          </form>

          <div class="pfilters">
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

          <div class="plist scroll">
            {#if videoState.loading}
              {#each [0, 1, 2, 3, 4] as _}
                <div class="vrow sk">
                  <Skeleton w="104px" h={58} radius={6} />
                  <div class="vrow-info">
                    <Skeleton w="88%" h={13} radius={4} />
                    <Skeleton w="56%" h={11} radius={4} />
                  </div>
                </div>
              {/each}
            {:else if list.length === 0}
              <div class="pempty">
                <Icon name="tv" size={22} color="var(--faint)" />
                <div class="pe-t">{t("video.none.title")}</div>
                <div class="pe-s">{t("video.none.sub")}</div>
              </div>
            {:else}
              {#each list as v (v.id)}
                {@const active = sel?.id === v.id}
                <button type="button" class="vrow" class:active onclick={() => pick(v)}>
                  <div class="thumb">
                    {#if v.thumbnail}
                      <img src={v.thumbnail} alt="" loading="lazy" />
                    {:else}
                      <div class="thumb-ph"><Icon name="tv" size={18} color="var(--faint)" /></div>
                    {/if}
                    {#if v.duration_ms}
                      <span class="dur mono">{fmtTime(v.duration_ms / 1000)}</span>
                    {/if}
                    {#if active}
                      <span class="now-dot"><Icon name="play" size={10} color="#0a0a0c" /></span>
                    {/if}
                  </div>
                  <div class="vrow-info">
                    <span class="vrow-title">{v.title}</span>
                    <div class="vrow-meta">
                      <span class="vrow-ch trunc">{v.channel}</span>
                      <span class="vrow-kind">{t(videoState.kindLabelKey(v.kind))}</span>
                    </div>
                  </div>
                </button>
              {/each}
            {/if}
          </div>
        </aside>
      {/if}
    </div>
  </div>
{/if}

<style>
  .vroot {
    flex: 1;
    min-height: 0;
    width: 100%;
    display: flex;
    justify-content: center;
    padding: 12px var(--page-x) 20px;
    overflow: hidden;
  }
  .vwrap {
    width: 100%;
    max-width: 1280px;
    height: 100%;
    min-height: 0;
    display: flex;
    align-items: flex-start;
    gap: 20px;
  }
  .vwrap.solo { max-width: 960px; }

  .vmain {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .vinfo {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 2px 2px 0;
  }
  .vmeta { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .vtitle-row { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .vtitle { font-size: 16px; font-weight: 600; color: var(--text); min-width: 0; }
  .kpill {
    flex: none;
    height: 20px;
    padding: 0 7px;
    border-radius: 5px;
    background: #222227;
    color: var(--dim);
    font-size: 11px;
    font-weight: 600;
    display: inline-flex;
    align-items: center;
  }
  .vsub { display: flex; align-items: center; gap: 6px; min-width: 0; font-size: 13px; color: var(--dim); }
  .artist:hover { color: var(--text); text-decoration: underline; }
  .dot { color: var(--faint); flex: none; }
  .chan { min-width: 0; }
  .note { color: var(--accent); font-size: 12px; min-width: 0; }

  .vacts { flex: none; display: flex; align-items: center; gap: 8px; }
  .flat-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 12px;
    border-radius: 8px;
    background: #1a1a1e;
    color: var(--dim);
    font-size: 12.5px;
    font-weight: 500;
    transition: background-color .14s ease-out, color .14s ease-out;
  }
  .flat-btn:hover { background: #26262c; color: var(--text); }
  .flat-btn.on { background: #2a2a31; color: var(--text); }

  /* Side video picker panel: flat dark fill, no outline/shadow */
  .vpanel {
    flex: none;
    width: 348px;
    max-height: 100%;
    height: 100%;
    border-radius: 12px;
    background: #131316;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .phead {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 10px 8px 14px;
  }
  .ptitle { font-size: 14px; font-weight: 600; color: var(--text); }
  .pcnt {
    height: 20px;
    padding: 0 7px;
    border-radius: 99px;
    background: #222227;
    color: var(--dim);
    font-size: 11px;
    display: inline-flex;
    align-items: center;
    margin-right: auto;
  }
  .phead :global(.ib) { margin-left: auto; }
  .pcnt + :global(.ib) { margin-left: 0; }

  .psearch {
    flex: none;
    position: relative;
    margin: 0 12px 10px;
    display: flex;
    align-items: center;
    background: #1c1c21;
    border-radius: 8px;
    height: 34px;
    padding: 0 8px 0 10px;
    gap: 8px;
  }
  .psearch input {
    flex: 1;
    min-width: 0;
    background: transparent;
    border: none;
    outline: none;
    color: var(--text);
    font-size: 13px;
  }
  .psearch input::placeholder { color: var(--faint); }
  .sic { display: grid; place-items: center; flex: none; }
  .clr {
    width: 22px;
    height: 22px;
    border-radius: 6px;
    display: grid;
    place-items: center;
    color: var(--dim);
  }
  .clr:hover { background: #2c2c33; color: var(--text); }

  .pfilters {
    flex: none;
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 0 12px 10px;
  }
  .fpill {
    height: 26px;
    padding: 0 10px;
    border-radius: 7px;
    background: #1c1c21;
    color: var(--dim);
    font-size: 12px;
    font-weight: 500;
    transition: background-color .14s ease-out, color .14s ease-out;
  }
  .fpill:hover { background: #25252b; color: var(--text); }
  .fpill.on { background: var(--accent); color: #0a0a0c; font-weight: 600; }

  .plist {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .vrow {
    width: 100%;
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 7px;
    border-radius: 9px;
    text-align: left;
    transition: background-color .14s ease-out;
  }
  .vrow:not(.sk):hover { background: #1e1e23; }
  .vrow.active { background: #252229; }

  .thumb {
    position: relative;
    flex: none;
    width: 104px;
    height: 58px;
    border-radius: 6px;
    overflow: hidden;
    background: #1c1c20;
  }
  .thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .thumb-ph { width: 100%; height: 100%; display: grid; place-items: center; }
  .dur {
    position: absolute;
    right: 4px;
    bottom: 4px;
    padding: 1px 5px;
    border-radius: 4px;
    background: rgba(8, 8, 10, .86);
    color: #fff;
    font-size: 10.5px;
    line-height: 1.3;
  }
  .now-dot {
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

  .vrow-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding-top: 1px;
  }
  .vrow-title {
    font-size: 12.5px;
    font-weight: 500;
    line-height: 1.3;
    color: var(--text);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .vrow.active .vrow-title { color: var(--accent); font-weight: 600; }
  .vrow-meta { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .vrow-ch { font-size: 11.5px; color: var(--dim); min-width: 0; }
  .vrow-kind {
    flex: none;
    padding: 1px 6px;
    border-radius: 4px;
    background: #1e1e23;
    color: var(--dim);
    font-size: 10.5px;
    font-weight: 500;
  }
  .vrow.active .vrow-kind { background: rgba(255, 255, 255, .1); color: var(--text); }

  .pempty {
    padding: 36px 16px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    text-align: center;
  }
  .pe-t { font-size: 13.5px; font-weight: 600; color: var(--text); margin-top: 4px; }
  .pe-s { font-size: 12px; color: var(--dim); }

  @media (max-width: 980px) {
    .vwrap { flex-direction: column; overflow-y: auto; }
    .vpanel { width: 100%; height: 320px; flex: none; }
  }
</style>
