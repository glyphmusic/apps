<script lang="ts">
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import Icon from "../components/Icon.svelte";
  import VisualCard from "./VisualCard.svelte";

  const big = $derived(app.config.player.mini_large);
  const pick = (v: boolean) => app.setConfig((c) => { c.player.mini_large = v; });
</script>

<VisualCard label={t("sp.appearance.mini")} info={t("sp.appearance.mini.desc")}>
  <div class="grid" role="radiogroup" aria-label={t("sp.appearance.mini")}>
    <button class="opt" class:on={!big} role="radio" aria-checked={!big} onclick={() => pick(false)}>
      <div class="stage">
        <div class="mini-bar">
          <span class="mb-cov"><Icon name="music-2" size={12} color="rgba(255,255,255,.7)" /></span>
          <span class="mb-txt">
            <span class="l1"></span>
            <span class="l2"></span>
          </span>
          <span class="mb-ctl">
            <span class="d"></span>
            <span class="play"></span>
            <span class="d"></span>
          </span>
          <span class="mb-prog"></span>
        </div>
      </div>
      <span class="lbl">{t("sp.appearance.mini.bar")}</span>
    </button>

    <button class="opt" class:on={big} role="radio" aria-checked={big} onclick={() => pick(true)}>
      <div class="stage">
        <div class="mini-card">
          <div class="mc-cov">
            <Icon name="music-2" size={18} color="rgba(255,255,255,.78)" />
            <span class="mc-prog"><i></i></span>
          </div>
          <div class="mc-meta">
            <span class="mc-lines"><span class="l1"></span><span class="l2"></span></span>
            <span class="mc-heart"><Icon name="heart" size={10} color={!big ? "rgba(255,255,255,.35)" : "var(--accent)"} /></span>
          </div>
        </div>
      </div>
      <span class="lbl">{t("sp.appearance.mini.card")}</span>
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

  .stage {
    width: 100%;
    height: 92px;
    display: grid;
    place-items: center;
  }

  .mini-bar {
    position: relative;
    width: 176px;
    max-width: 100%;
    height: 36px;
    padding: 0 8px 0 6px;
    border-radius: 9px;
    background: rgba(255, 255, 255, .07);
    display: flex;
    align-items: center;
    gap: 8px;
    overflow: hidden;
  }
  .mb-cov {
    width: 24px;
    height: 24px;
    border-radius: 5px;
    background: rgba(255, 255, 255, .1);
    display: grid;
    place-items: center;
    flex: none;
  }
  .opt.on .mb-cov { background: color-mix(in srgb, var(--accent) 26%, #181818); }
  .mb-txt { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .l1 { width: 72%; height: 4px; border-radius: 2px; background: rgba(255, 255, 255, .45); }
  .l2 { width: 48%; height: 3px; border-radius: 2px; background: rgba(255, 255, 255, .2); }
  .mb-ctl { display: flex; align-items: center; gap: 5px; flex: none; }
  .d { width: 5px; height: 5px; border-radius: 50%; background: rgba(255, 255, 255, .35); }
  .play { width: 14px; height: 14px; border-radius: 50%; background: rgba(255, 255, 255, .75); }
  .opt.on .play { background: #fff; }
  .mb-prog {
    position: absolute;
    left: 0;
    bottom: 0;
    width: 58%;
    height: 2px;
    background: rgba(255, 255, 255, .35);
    transition: background-color .15s;
  }
  .opt.on .mb-prog { background: var(--accent); }

  .mini-card {
    width: 76px;
    padding: 6px;
    border-radius: 9px;
    background: rgba(255, 255, 255, .07);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .mc-cov {
    position: relative;
    width: 100%;
    height: 54px;
    border-radius: 6px;
    background: rgba(255, 255, 255, .08);
    display: grid;
    place-items: center;
    overflow: hidden;
  }
  .opt.on .mc-cov {
    background: linear-gradient(135deg, color-mix(in srgb, var(--accent) 36%, #181818), rgba(255, 255, 255, .08));
  }
  .mc-prog {
    position: absolute;
    left: 5px;
    right: 5px;
    bottom: 5px;
    height: 2px;
    border-radius: 1px;
    background: rgba(255, 255, 255, .18);
  }
  .mc-prog i {
    display: block;
    width: 60%;
    height: 100%;
    border-radius: 1px;
    background: rgba(255, 255, 255, .5);
  }
  .opt.on .mc-prog i { background: var(--accent); }
  .mc-meta { display: flex; align-items: center; gap: 6px; }
  .mc-lines { flex: 1; display: flex; flex-direction: column; gap: 3px; }
  .mc-heart { flex: none; display: flex; }

  .lbl {
    font-size: 13px;
    font-weight: 500;
    color: rgba(255, 255, 255, .72);
    transition: color .15s;
  }
  .opt:hover .lbl { color: #fff; }
  .opt.on .lbl { color: #fff; font-weight: 600; }

  @media (max-width: 440px) {
    .grid { grid-template-columns: 1fr; }
  }
</style>
