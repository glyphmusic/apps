<script lang="ts">
  import { app } from "../state.svelte";
  import { onWeb } from "../web";
  import { t } from "../i18n.svelte";
  import Icon from "../components/Icon.svelte";
  import Tog from "./Tog.svelte";
  import VisualCard from "./VisualCard.svelte";
  import { tip } from "../ui.svelte";

  const ui = $derived(app.config.ui);
  function setLyrics(v: boolean) {
    app.setConfig((c) => { c.ui.np_lyrics = v; });
    app.showLyrics = v;
  }
  function setDl(v: boolean) {
    app.setConfig((c) => { c.ui.np_download = v; });
  }
</script>

<VisualCard
  label={t("sp.player.np_preview")}
  extraLabels={[t("sp.player.lyrics"), t("sp.player.np_download")]}
>
  <div class="stage">
    <div class="np-win" class:lyrics={ui.np_lyrics}>
      <div class="np-left">
        <div class="np-cov">
          <Icon name="music-2" size={18} color="rgba(255,255,255,.78)" />
        </div>
        <div class="np-meta">
          <span class="t1"></span>
          <span class="t2"></span>
        </div>
        <div class="np-seek"><i></i></div>
        <div class="np-acts">
          <span class="ic"><Icon name="heart" size={11} color="var(--accent)" /></span>
          <span class="ic"><Icon name="clock" size={11} color="rgba(255,255,255,.55)" /></span>
          {#if !onWeb()}
            <span class="ic dl" class:on={ui.np_download}>
              <Icon name="download" size={11} color={ui.np_download ? "var(--accent)" : "rgba(255,255,255,.22)"} />
            </span>
          {/if}
          <span class="ic"><Icon name="maximize-2" size={11} color="rgba(255,255,255,.55)" /></span>
        </div>
      </div>

      {#if ui.np_lyrics}
        <div class="np-lyrics">
          <span class="ll w70"></span>
          <span class="ll w85 active"></span>
          <span class="ll w60"></span>
          <span class="ll w75"></span>
          <span class="ll w50"></span>
        </div>
      {/if}
    </div>
  </div>

  <div class="togs">
    <div class="trow">
      <div class="tlab">
        <span>{t("sp.player.lyrics")}</span>
        <span class="info" use:tip={{ label: t("sp.player.lyrics.desc"), wrap: true }}>
          <Icon name="info" size={13} color="rgba(255,255,255,.42)" />
        </span>
      </div>
      <Tog on={ui.np_lyrics} onchange={setLyrics} label={t("sp.player.lyrics")} />
    </div>

    {#if !onWeb()}
      <div class="trow">
        <div class="tlab">
          <span>{t("sp.player.np_download")}</span>
          <span class="info" use:tip={{ label: t("sp.player.np_download.desc"), wrap: true }}>
            <Icon name="info" size={13} color="rgba(255,255,255,.42)" />
          </span>
        </div>
        <Tog on={ui.np_download} onchange={setDl} label={t("sp.player.np_download")} />
      </div>
    {/if}
  </div>
</VisualCard>

<style>
  .stage {
    padding: 14px;
    border-radius: 10px;
    background: #0a0a0c;
    display: flex;
    justify-content: center;
  }
  .np-win {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 24px;
    width: 100%;
    max-width: 340px;
    min-height: 118px;
  }
  .np-left {
    width: 120px;
    flex: none;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 7px;
  }
  .np-cov {
    width: 62px;
    height: 62px;
    border-radius: 8px;
    background: linear-gradient(135deg, color-mix(in srgb, var(--accent) 34%, #181818), rgba(255, 255, 255, .07));
    display: grid;
    place-items: center;
  }
  .np-meta {
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
  }
  .t1 { width: 72%; height: 4.5px; border-radius: 2px; background: rgba(255, 255, 255, .6); }
  .t2 { width: 46%; height: 3.5px; border-radius: 2px; background: rgba(255, 255, 255, .28); }
  .np-seek {
    width: 86%;
    height: 2.5px;
    border-radius: 2px;
    background: rgba(255, 255, 255, .14);
    overflow: hidden;
  }
  .np-seek i { display: block; width: 45%; height: 100%; background: #fff; }
  .np-acts {
    display: flex;
    align-items: center;
    gap: 7px;
    padding-top: 2px;
  }
  .ic {
    width: 20px;
    height: 20px;
    border-radius: 6px;
    background: rgba(255, 255, 255, .05);
    display: grid;
    place-items: center;
    transition: opacity .15s, background-color .15s, transform .15s var(--ease);
  }
  .ic.dl:not(.on) { opacity: .28; transform: scale(.86); background: transparent; }
  .ic.dl.on { background: color-mix(in srgb, var(--accent) 18%, transparent); }

  .np-lyrics {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 12px;
    border-radius: 8px;
    background: rgba(255, 255, 255, .03);
  }
  .ll { height: 5px; border-radius: 3px; background: rgba(255, 255, 255, .16); }
  .ll.active { background: #fff; height: 6px; box-shadow: 0 0 10px color-mix(in srgb, var(--accent) 45%, transparent); }
  .ll.w70 { width: 70%; }
  .ll.w85 { width: 88%; }
  .ll.w60 { width: 60%; }
  .ll.w75 { width: 76%; }
  .ll.w50 { width: 52%; }

  .togs {
    display: flex;
    flex-direction: column;
    border-top: 1px solid rgba(255, 255, 255, .06);
    padding-top: 4px;
  }
  .trow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 0;
  }
  .trow:not(:last-child) { border-bottom: 1px solid rgba(255, 255, 255, .05); }
  .tlab {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13.5px;
    font-weight: 500;
    color: rgba(255, 255, 255, .92);
  }
  .info { flex: none; width: 18px; height: 18px; border-radius: 9px; display: grid; place-items: center; }
  .info:hover { background: rgba(255, 255, 255, .08); }
  /* Bigger tap target on touch screens (a tap shows the explanation). */
  .info { position: relative; cursor: help; }
  .info::after { content: ""; position: absolute; inset: -10px; }
</style>
