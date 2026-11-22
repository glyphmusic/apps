<script lang="ts">
  // Flat Daily Mix card for Home ("Made for you"): 192 px wide like SongCard,
  // 176 px 2x2 mosaic cover of top mix tracks with subtle hue tint and hover
  // play button. Clicking opens the full mix; play button starts it at once.
  import Cover from "../../components/Cover.svelte";
  import Icon from "../../components/Icon.svelte";
  import { app } from "../../state.svelte";
  import { t } from "../../i18n.svelte";
  import type { DailyMix } from "../../api/types";

  let { mix }: { mix: DailyMix } = $props();

  const tiles = $derived(mix.tracks.slice(0, 4));
  const kindLabel = $derived(
    mix.kind === "time"
      ? t("taste.mix.time_pill")
      : mix.kind === "discovery"
        ? t("taste.mix.discovery_pill")
        : (mix.tags?.[0] ?? t("taste.mix.daily_pill")),
  );

  function openMix() {
    app.openSection({
      title: mix.title,
      icon: mix.kind === "time" ? "clock" : mix.kind === "discovery" ? "sparkles" : "disc-3",
      tracks: mix.tracks,
      subtitle: mix.subtitle,
      tags: mix.tags,
      canSavePlaylist: true,
    });
  }

  function playMix(e: MouseEvent) {
    e.stopPropagation();
    if (mix.tracks.length) app.play(mix.tracks, 0, mix.title);
  }
</script>

<div
  class="card"
  onclick={openMix}
  onkeydown={(e) => e.key === "Enter" && openMix()}
  role="button"
  tabindex="-1"
>
  <div class="art" style:--hue={mix.accent_hue}>
    {#if tiles.length >= 4}
      <div class="mosaic">
        {#each tiles as tr, i (`${tr.id}-${i}`)}
          <Cover track={tr} size={88} radius={0} />
        {/each}
      </div>
    {:else if tiles.length > 0}
      <Cover track={tiles[0]} size={176} radius={8} />
    {:else}
      <div class="ph"><Icon name="sparkles" size={32} color="rgba(255,255,255,.55)" /></div>
    {/if}
    <div class="tint"></div>
    {#if kindLabel}
      <span class="pill trunc">{kindLabel}</span>
    {/if}
    <button
      type="button"
      class="play"
      onclick={playMix}
      aria-label={t("tip.play")}
    >
      <Icon name="play" size={16} color="#000" />
    </button>
  </div>
  <div class="t trunc">{mix.title}</div>
  <div class="sub trunc">{mix.subtitle}</div>
</div>

<style>
  .card {
    flex: none;
    width: 192px;
    padding: 8px;
    border-radius: 10px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    cursor: pointer;
    transition: background-color .24s ease-out;
  }
  .card:hover { background: rgba(255, 255, 255, .05); transition-duration: .12s; }
  .art {
    position: relative;
    width: 176px;
    height: 176px;
    border-radius: 8px;
    overflow: hidden;
    background: hsl(var(--hue, 328) 28% 16%);
  }
  .mosaic {
    width: 176px;
    height: 176px;
    display: grid;
    grid-template-columns: 88px 88px;
    grid-template-rows: 88px 88px;
  }
  .ph { width: 100%; height: 100%; display: grid; place-items: center; }
  .tint {
    position: absolute;
    inset: 0;
    background: linear-gradient(180deg, hsla(var(--hue, 328) 55% 32% / .16) 0%, rgba(0, 0, 0, .42) 100%);
    pointer-events: none;
  }
  .pill {
    position: absolute;
    left: 8px;
    bottom: 8px;
    max-width: 114px;
    height: 22px;
    padding: 0 8px;
    border-radius: 999px;
    background: rgba(14, 14, 16, .82);
    color: rgba(255, 255, 255, .92);
    font-size: 11.5px;
    font-weight: 500;
    display: inline-flex;
    align-items: center;
    pointer-events: none;
  }
  .play {
    position: absolute;
    right: 8px;
    bottom: 8px;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: #fff;
    display: grid;
    place-items: center;
    padding-left: 2px;
    opacity: 0;
    transform: translateY(4px);
    transition: opacity .18s ease-out, transform .18s var(--ease);
  }
  .card:hover .play, .play:focus-visible { opacity: 1; transform: none; }
  .play:active { transform: scale(.93); }
  .t { padding-top: 4px; font-size: var(--md); font-weight: 600; color: var(--text); }
  .sub { font-size: var(--sm); color: var(--dim); }
</style>
