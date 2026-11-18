<script lang="ts">
  // Appearance > Animations: who draws the background, the graphics card or
  // the processor. Same picture cards as Performance (PerformanceVisual):
  // a small background with its characters and the chip that draws it.
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { gpuBackgroundSupported, type RenderMode } from "../glow";
  import VisualCard from "./VisualCard.svelte";

  const mode = $derived<RenderMode>(app.config.visuals.render_mode === "cpu" ? "cpu" : "gpu");
  // "gpu" is stored as "" (the default).
  const pick = (m: RenderMode) => app.setConfig((c) => { c.visuals.render_mode = m === "cpu" ? "cpu" : ""; });
  // Without WebGL 2 the processor draws anyway: say so under the card.
  const gpuOk = gpuBackgroundSupported();
  const info = $derived(!gpuOk && mode === "gpu" ? t("sp.render.gpu.missing") : mode === "gpu" ? t("sp.render.gpu.desc") : t("sp.render.cpu.desc"));

  // A fixed character pattern (no randomness: the picture looks the same every time).
  const RAMP = " .-=+xX0S8#@";
  const ROWS = 5, COLS = 18;
  const CELLS = (() => {
    const out: string[] = [];
    for (let r = 0; r < ROWS; r++) {
      let line = "";
      for (let c = 0; c < COLS; c++) {
        // A soft diagonal beam, brightest in the middle.
        const d = Math.abs(c / COLS - (1 - r / ROWS) * 0.9 - 0.05);
        const v = Math.max(0, 1 - d * 3.2);
        line += v > 0.28 ? RAMP[Math.min(RAMP.length - 1, Math.max(1, Math.round(v * (RAMP.length - 1))))] : " ";
      }
      out.push(line);
    }
    return out.join("\n");
  })();
  const OPTIONS: RenderMode[] = ["gpu", "cpu"];
</script>

<VisualCard label={t("sp.render")} {info}>
  <div class="grid" role="radiogroup" aria-label={t("sp.render")}>
    {#each OPTIONS as m (m)}
      {@const sel = mode === m}
      <button class="opt" class:on={sel} role="radio" aria-checked={sel} onclick={() => pick(m)}>
        <div class="stage" aria-hidden="true">
          <span class="glow"></span>
          <pre class="chars">{CELLS}</pre>
          <span class="chip mono">{m === "gpu" ? "GPU" : "CPU"}</span>
        </div>
        <div class="meta">
          <span class="lbl">{m === "gpu" ? t("sp.render.gpu") : t("sp.render.cpu")}</span>
          <span class="hint">{m === "gpu" ? t("sp.render.gpu.hint") : t("sp.render.cpu.hint")}</span>
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
  .chars {
    position: absolute; inset: 0; margin: 0; padding: 8px 10px;
    font: 500 10px/12px var(--mono, ui-monospace, monospace); letter-spacing: 1px;
    color: rgba(255, 255, 255, .55); white-space: pre; overflow: hidden;
  }
  .chip {
    position: absolute; right: 8px; bottom: 6px; padding: 1px 6px; border-radius: 4px;
    font-size: 10px; color: rgba(255, 255, 255, .6); background: rgba(255, 255, 255, .08);
  }
  .opt.on .chip { color: #fff; background: rgba(255, 255, 255, .16); }

  .meta { display: flex; flex-direction: column; gap: 2px; }
  .lbl { font-size: 13px; font-weight: 500; color: rgba(255, 255, 255, .78); }
  .opt.on .lbl { color: #fff; font-weight: 600; }
  .hint { font-size: 11.5px; color: rgba(255, 255, 255, .48); }
  .opt.on .hint { color: rgba(255, 255, 255, .68); }

  @media (max-width: 480px) { .grid { grid-template-columns: 1fr; } }
</style>
