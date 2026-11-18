<script lang="ts">
  // Playback > Streaming quality as four cards: bars for the sound quality
  // and roughly how much data an hour of music uses, so "Low" and "High"
  // mean something.
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import Icon from "../components/Icon.svelte";
  import VisualCard from "./VisualCard.svelte";

  const QUALITIES = [
    { id: "auto", bars: 0, mb: "" },
    { id: "low", bars: 1, mb: "~45 MB" },
    { id: "standard", bars: 2, mb: "~70 MB" },
    { id: "high", bars: 4, mb: "~140 MB" },
  ] as const;
  const cur = $derived(app.config.audio.quality);
  const pick = (q: string) => app.setConfig((c) => { c.audio.quality = q as typeof c.audio.quality; });
</script>

<VisualCard label={t("sp.audio.quality")} info={t("sp.audio.quality.desc")}>
  <div class="grid" role="radiogroup" aria-label={t("sp.audio.quality")}>
    {#each QUALITIES as q (q.id)}
      {@const on = cur === q.id}
      <button class="opt" class:on role="radio" aria-checked={on} onclick={() => pick(q.id)}>
        <div class="stage" aria-hidden="true">
          {#if q.id === "auto"}
            <Icon name="activity" size={20} color={on ? "var(--accent)" : "rgba(255,255,255,.5)"} />
          {:else}
            <span class="bars">
              {#each [1, 2, 3, 4] as b (b)}<span class="bar" class:lit={b <= q.bars} style:height="{6 + b * 4}px"></span>{/each}
            </span>
          {/if}
        </div>
        <span class="lbl">{t(`sp.audio.q.${q.id}`)}</span>
        <span class="hint">{q.mb ? `${q.mb} ${t("sp.audio.q.per_hour")}` : t("sp.audio.q.auto.hint")}</span>
      </button>
    {/each}
  </div>
</VisualCard>

<style>
  .grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px; }
  .opt {
    display: flex; flex-direction: column; align-items: flex-start; gap: 4px; padding: 10px;
    border-radius: 10px; background: #0b0b0b; border: 1.5px solid rgba(255, 255, 255, .14);
    text-align: left; min-width: 0;
    transition: border-color .15s ease-out, background-color .15s ease-out, transform .15s var(--ease);
  }
  .opt:hover { border-color: rgba(255, 255, 255, .42); }
  .opt.on { border-color: #fff; background: #101012; }
  .opt:active { transform: scale(.985); }
  .stage { height: 34px; width: 100%; display: flex; align-items: center; justify-content: flex-start; margin-bottom: 4px; }
  .bars { display: flex; align-items: flex-end; gap: 3px; height: 24px; }
  .bar { width: 5px; border-radius: 2px; background: rgba(255, 255, 255, .14); }
  .bar.lit { background: rgba(255, 255, 255, .55); }
  .opt.on .bar.lit { background: var(--accent); }
  .lbl { font-size: 12.5px; font-weight: 500; color: rgba(255, 255, 255, .78); line-height: 1.25; }
  .opt.on .lbl { color: #fff; font-weight: 600; }
  .hint { font-size: 11px; color: rgba(255, 255, 255, .46); line-height: 1.3; }
  .opt.on .hint { color: rgba(255, 255, 255, .66); }
  @media (max-width: 560px) { .grid { grid-template-columns: repeat(2, 1fr); } }
</style>
