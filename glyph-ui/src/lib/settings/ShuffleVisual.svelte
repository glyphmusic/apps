<script lang="ts">
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import type { ShuffleStyle } from "../api/types";
  import VisualCard from "./VisualCard.svelte";

  const style = $derived(app.config.playback.shuffle);
  const pick = (v: ShuffleStyle) => app.setConfig((c) => { c.playback.shuffle = v; });

  // Each color represents a different artist in the queue.
  const SMART_COLORS = ["#4da3ff", "#ff8a3d", "#3ddc84", "#b07cff", "#4da3ff", "#ff5fd2"];
  const RANDOM_COLORS = ["#4da3ff", "#4da3ff", "#b07cff", "#ff8a3d", "#ff8a3d", "#ff8a3d"];
</script>

<VisualCard
  label={t("sp.shuffle")}
  info={style === "smart" ? t("sp.shuffle.smart.desc") : t("sp.shuffle.random.desc")}
>
  <div class="grid" role="radiogroup" aria-label={t("sp.shuffle")}>
    <button class="opt" class:on={style === "smart"} role="radio" aria-checked={style === "smart"} onclick={() => pick("smart")}>
      <div class="seq" aria-hidden="true">
        {#each SMART_COLORS as col, i}
          <div class="track">
            <span class="dot" style:background={col}></span>
            <span class="bar" style:width="{58 + ((i * 19) % 28)}%"></span>
          </div>
        {/each}
      </div>
      <div class="meta">
        <span class="lbl">{t("sp.shuffle.smart")}</span>
        <span class="hint">{t("sp.shuffle.smart.hint")}</span>
      </div>
    </button>

    <button class="opt" class:on={style === "random"} role="radio" aria-checked={style === "random"} onclick={() => pick("random")}>
      <div class="seq" aria-hidden="true">
        {#each RANDOM_COLORS as col, i}
          <div class="track">
            <span class="dot" style:background={col}></span>
            <span class="bar" style:width="{54 + ((i * 23) % 30)}%"></span>
          </div>
        {/each}
      </div>
      <div class="meta">
        <span class="lbl">{t("sp.shuffle.random")}</span>
        <span class="hint">{t("sp.shuffle.random.hint")}</span>
      </div>
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
    gap: 10px;
    padding: 12px;
    border-radius: 10px;
    background: #0b0b0b;
    border: 0;
    text-align: left;
    transition: border-color .15s ease-out, background-color .15s ease-out, transform .15s var(--ease);
  }
  .opt:hover { background: #18181b; }
  /* No outlines: the picked card is the lighter one. */
  .opt.on { background: #26262b; }
  .opt:active { transform: scale(.985); }

  .seq {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 5px 10px;
    padding: 8px 10px;
    border-radius: 7px;
    background: rgba(255, 255, 255, .03);
  }
  .track { display: flex; align-items: center; gap: 6px; }
  .dot { width: 7px; height: 7px; border-radius: 50%; flex: none; opacity: .82; }
  .opt.on .dot { opacity: 1; }
  .bar { height: 4px; border-radius: 2px; background: rgba(255, 255, 255, .24); }
  .opt.on .bar { background: rgba(255, 255, 255, .42); }

  .meta { display: flex; flex-direction: column; gap: 2px; }
  .lbl { font-size: 13px; font-weight: 500; color: rgba(255, 255, 255, .78); }
  .opt.on .lbl { color: #fff; font-weight: 600; }
  .hint { font-size: 11.5px; color: rgba(255, 255, 255, .48); }
  .opt.on .hint { color: rgba(255, 255, 255, .68); }

  @media (max-width: 480px) {
    .grid { grid-template-columns: 1fr; }
  }
</style>
