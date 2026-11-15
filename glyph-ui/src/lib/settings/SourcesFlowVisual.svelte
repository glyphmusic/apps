<script lang="ts">
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { onWeb } from "../web";
  import { PROVIDER_NAME } from "../api/types";
  import Icon from "../components/Icon.svelte";
  import DragList from "./DragList.svelte";
  import VisualCard from "./VisualCard.svelte";
  import { activeSearchSources, activeSources, youtubeRouteLabel } from "./sources";

  let {
    touch = false,
    onmove,
  }: {
    touch?: boolean;
    onmove: (from: number, to: number) => void;
  } = $props();

  const web = onWeb();
  const pv = $derived(app.config.providers);
  const active = $derived(activeSources(pv, web));
  const searchOnly = $derived(
    pv.youtube_only ? activeSearchSources(pv, web).filter((p) => p !== "youtube") : [],
  );
  const deezerOn = $derived(pv.enabled.includes("deezer"));
  const ytEnabled = $derived(pv.enabled.includes("youtube"));
  const ytRoute = $derived(youtubeRouteLabel(pv, web, touch));
</script>

<VisualCard label={t("sp.sources.flow")} info={t("settings.sources.hint")} extraLabels={[t("settings.priority")]}>
  {#snippet right()}
    {#if pv.youtube_only}
      <span class="hint">{t("sp.sources.flow.yt_only_badge")}</span>
    {:else if active.length > 1}
      <span class="hint">{t("settings.priority.hint")}</span>
    {/if}
  {/snippet}

  <div class="box">
    {#if pv.youtube_only}
      {#if ytEnabled}
        <div class="row on">
          <span class="mono num">1</span>
          <span class="nm">{PROVIDER_NAME.youtube}</span>
          <span class="mono sub">{ytRoute}</span>
        </div>
      {:else}
        <div class="empty">{t("sp.sources.flow.none")}</div>
      {/if}
      {#each searchOnly as p (p)}
        <div class="row dim">
          <span class="mono num">–</span>
          <span class="nm">{PROVIDER_NAME[p]}</span>
          <span class="sub">{t("sp.sources.flow.search_only")}</span>
        </div>
      {/each}
    {:else if active.length === 0 && !deezerOn}
      <div class="empty">{t("sp.sources.flow.none")}</div>
    {:else}
      {#if active.length > 0}
        <DragList items={active} pitch={34} key={(p) => p} {onmove}>
          {#snippet row(p, i)}
            <div class="row drag" class:on={i === 0}>
              <Icon name="grip-vertical" size={12} color="rgba(255,255,255,.32)" />
              <span class="mono num">{i + 1}</span>
              <span class="nm">{PROVIDER_NAME[p]}</span>
              {#if p === "youtube"}
                <span class="mono sub">{ytRoute}</span>
              {/if}
            </div>
          {/snippet}
        </DragList>
      {/if}
      {#if deezerOn}
        <div class="row dim">
          <span class="mono num">~</span>
          <span class="nm">{PROVIDER_NAME.deezer}</span>
          <span class="sub">{t("sp.sources.flow.preview")}</span>
        </div>
      {/if}
    {/if}
  </div>
</VisualCard>

<style>
  .hint {
    font-size: 11.5px;
    color: rgba(255, 255, 255, .44);
  }
  .box {
    padding: 8px 10px;
    border-radius: 9px;
    background: #0b0b0b;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row {
    height: 30px;
    padding: 0 6px;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: rgba(255, 255, 255, .66);
    min-width: 0;
  }
  .row.drag {
    height: 100%;
  }
  .row.on {
    color: #fff;
    font-weight: 500;
  }
  .row.dim {
    color: rgba(255, 255, 255, .4);
  }
  .num {
    width: 14px;
    text-align: center;
    font-size: 11px;
    color: rgba(255, 255, 255, .36);
    flex: none;
  }
  .row.on .num {
    color: var(--accent);
    font-weight: 600;
  }
  .nm {
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    font-size: 11px;
    color: rgba(255, 255, 255, .4);
    white-space: nowrap;
    flex: none;
  }
  .empty {
    padding: 6px;
    font-size: 12.5px;
    color: rgba(255, 255, 255, .42);
  }
</style>
