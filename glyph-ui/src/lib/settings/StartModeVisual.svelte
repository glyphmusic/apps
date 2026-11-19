<script lang="ts">
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import VisualCard from "./VisualCard.svelte";

  const d = $derived(app.config.desktop);
  const modeDesc = $derived(app.platform === "macos" ? t("sp.system.start_mode.desc_dock") : t("sp.system.start_mode.desc"));
  const pick = (tray: boolean) => app.setConfig((c) => { c.desktop.start_in_tray = tray; });
</script>

<VisualCard label={t("sp.system.start_mode")} info={modeDesc} dim={!d.autostart}>
  <div class="grid" role="radiogroup" aria-label={t("sp.system.start_mode")}>
    <button class="opt" class:on={!d.start_in_tray} role="radio" aria-checked={!d.start_in_tray} onclick={() => pick(false)}>
      <div class="screen">
        <div class="win">
          <div class="win-top"><span></span><span></span><span></span></div>
          <div class="win-body">
            <div class="win-side"></div>
            <div class="win-main"><span></span><span></span></div>
          </div>
        </div>
        <div class="taskbar"><span class="tb-app"></span></div>
      </div>
      <span class="lbl">{t("sp.system.mode.window")}</span>
    </button>

    <button class="opt" class:on={d.start_in_tray} role="radio" aria-checked={d.start_in_tray} onclick={() => pick(true)}>
      <div class="screen">
        <div class="taskbar">
          <span class="tb-tray"></span>
        </div>
      </div>
      <span class="lbl">{t("sp.system.mode.tray")}</span>
    </button>
  </div>
</VisualCard>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 12px;
  }
  .opt {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 9px;
    padding: 12px;
    border-radius: 10px;
    background: #0b0b0b;
    border: 0;
    transition: border-color .15s ease-out, background-color .15s ease-out, transform .15s var(--ease);
  }
  .opt:hover { background: #18181b; }
  /* No outlines: the picked card is the lighter one. */
  .opt.on { background: #26262b; }
  .opt:active { transform: scale(.985); }

  .screen {
    position: relative;
    width: 128px;
    max-width: 100%;
    height: 78px;
    border-radius: 7px;
    background: rgba(255, 255, 255, .04);
    border: 1px solid rgba(255, 255, 255, .1);
    overflow: hidden;
    display: grid;
    place-items: center;
  }
  .win {
    width: 82px;
    height: 48px;
    margin-bottom: 6px;
    border-radius: 5px;
    background: #17171b;
    border: 1px solid rgba(255, 255, 255, .2);
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .opt.on .win { border-color: rgba(255, 255, 255, .65); }
  .win-top { display: flex; gap: 2px; }
  .win-top span { width: 3px; height: 3px; border-radius: 50%; background: rgba(255, 255, 255, .3); }
  .win-body { flex: 1; display: flex; gap: 4px; }
  .win-side { width: 12px; border-radius: 2px; background: rgba(255, 255, 255, .14); }
  .win-main { flex: 1; display: flex; flex-direction: column; gap: 3px; justify-content: center; }
  .win-main span { height: 4px; border-radius: 2px; background: rgba(255, 255, 255, .28); }
  .opt.on .win-main span { background: var(--accent); }

  .taskbar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 12px;
    background: rgba(255, 255, 255, .07);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 6px;
  }
  .tb-app {
    width: 14px;
    height: 4px;
    border-radius: 2px;
    background: rgba(255, 255, 255, .5);
  }
  .opt.on .tb-app { background: var(--accent); }
  .tb-tray {
    margin-left: auto;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: rgba(255, 255, 255, .45);
    box-shadow: 0 0 0 2px rgba(255, 255, 255, .12);
  }
  .opt.on .tb-tray {
    background: var(--accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 32%, transparent);
  }

  .lbl {
    font-size: 13px;
    font-weight: 500;
    color: rgba(255, 255, 255, .72);
  }
  .opt:hover .lbl { color: #fff; }
  .opt.on .lbl { color: #fff; font-weight: 600; }

  @media (max-width: 440px) {
    .grid { grid-template-columns: 1fr; }
  }
</style>
