<script lang="ts">
  // Appearance > Performance: full effects or performance mode, each shown as
  // a small picture of the background (glow + characters, or a calm glow
  // with fewer, slower characters) and what it changes.
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import VisualCard from "./VisualCard.svelte";

  const perf = $derived(!!app.config.visuals.performance_mode);
  const pick = (on: boolean) => app.setConfig((c) => { c.visuals.performance_mode = on; });

  // A fixed character pattern (no randomness: the picture looks the same every time).
  const RAMP = " .-=+xX0S8#@";
  const ROWS = 5, COLS = 18;
  function cells(dense: boolean): string[] {
    const out: string[] = [];
    for (let r = 0; r < ROWS; r++) {
      let line = "";
      for (let c = 0; c < COLS; c++) {
        // A soft diagonal beam, brightest in the middle.
        const d = Math.abs(c / COLS - (1 - r / ROWS) * 0.9 - 0.05);
        const v = Math.max(0, 1 - d * 3.2);
        const keep = dense ? v > 0.28 : v > 0.62 && (r + c) % 2 === 0;
        line += keep ? RAMP[Math.min(RAMP.length - 1, Math.max(1, Math.round(v * (RAMP.length - 1))))] : " ";
      }
      out.push(line);
    }
    return out;
  }
  const FULL = cells(true);
  const LITE = cells(false);
</script>

<VisualCard label={t("sp.perf")} info={perf ? t("sp.perf.on.desc") : t("sp.perf.off.desc")}>
  <div class="grid" role="radiogroup" aria-label={t("sp.perf")}>
    {#each [false, true] as on (String(on))}
      {@const sel = perf === on}
      <button class="opt" class:on={sel} role="radio" aria-checked={sel} onclick={() => pick(on)}>
        <div class="stage" class:lite={on} aria-hidden="true">
          <span class="glow"></span>
          <pre class="chars">{(on ? LITE : FULL).join("\n")}</pre>
          <span class="fps mono">{on ? "12 fps" : "24 fps"}</span>
        </div>
        <div class="meta">
          <span class="lbl">{on ? t("sp.perf.on") : t("sp.perf.off")}</span>
          <span class="hint">{on ? t("sp.perf.on.hint") : t("sp.perf.off.hint")}</span>
        </div>
      </button>
    {/each}
  </div>
</VisualCard>

<style>
  .grid { display: grid; grid-template-columns: repeat(2, 1fr); gap: 12px; }
  .opt {
    display: flex; flex-direction: column; gap: 10px; padding: 12px;
    border-radius: 10px; background: #0b0b0b; border: 1.5px solid rgba(255, 255, 255, .14);
    text-align: left;
    transition: border-color .15s ease-out, background-color .15s ease-out, transform .15s var(--ease);
  }
  .opt:hover { border-color: rgba(255, 255, 255, .42); }
  .opt.on { border-color: #fff; background: #101012; }
  .opt:active { transform: scale(.985); }

  .stage { position: relative; height: 74px; border-radius: 7px; overflow: hidden; background: #050505; }
  .glow {
    position: absolute; inset: -20% -10%;
    background: radial-gradient(60% 55% at 62% 40%, rgba(var(--accent-rgb, 255, 95, 210), .55), rgba(var(--accent-rgb, 255, 95, 210), 0) 70%);
  }
  .stage.lite .glow { opacity: .6; }
  .chars {
    position: absolute; inset: 0; margin: 0; padding: 8px 10px;
    font: 500 10px/12px var(--mono, ui-monospace, monospace); letter-spacing: 1px;
    color: rgba(255, 255, 255, .55); white-space: pre; overflow: hidden;
  }
  .stage.lite .chars { color: rgba(255, 255, 255, .32); }
  .fps { position: absolute; right: 8px; bottom: 6px; font-size: 10px; color: rgba(255, 255, 255, .5); }
  .opt.on .fps { color: rgba(255, 255, 255, .8); }

  .meta { display: flex; flex-direction: column; gap: 2px; }
  .lbl { font-size: 13px; font-weight: 500; color: rgba(255, 255, 255, .78); }
  .opt.on .lbl { color: #fff; font-weight: 600; }
  .hint { font-size: 11.5px; color: rgba(255, 255, 255, .48); }
  .opt.on .hint { color: rgba(255, 255, 255, .68); }

  @media (max-width: 480px) { .grid { grid-template-columns: 1fr; } }
</style>
