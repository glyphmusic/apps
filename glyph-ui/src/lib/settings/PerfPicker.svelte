<script lang="ts">
  // The two performance cards: full effects or performance mode, each shown
  // as a small picture of the background (glow + characters, or a calm glow
  // with fewer, slower characters) and what it changes. Used by Settings >
  // Appearance > Performance and the onboarding's speed page, which marks
  // the one that suits this device (`recommended`: true = performance mode)
  // and leaves out the hints (`compact`).
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";

  let { recommended = null, compact = false }: { recommended?: boolean | null; compact?: boolean } = $props();

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

<div class="grid" class:compact role="radiogroup" aria-label={t("sp.perf")}>
  {#each [false, true] as on (String(on))}
    {@const sel = perf === on}
    <button class="opt" class:on={sel} role="radio" aria-checked={sel} onclick={() => pick(on)}>
      <div class="stage" class:lite={on} aria-hidden="true">
        <pre class="chars">{(on ? LITE : FULL).join("\n")}</pre>
        {#if recommended === on}<span class="rec">{t("sp.perf.recommended")}</span>{/if}
        <span class="fps mono">{on ? "12 fps" : "24 fps"}</span>
      </div>
      <div class="meta">
        <span class="lbl">{on ? t("sp.perf.on") : t("sp.perf.off")}</span>
        {#if !compact}<span class="hint">{on ? t("sp.perf.on.hint") : t("sp.perf.off.hint")}</span>{/if}
      </div>
    </button>
  {/each}
</div>

<style>
  .grid { display: grid; grid-template-columns: repeat(2, 1fr); gap: 12px; }
  .grid.compact { gap: 10px; }
  .opt {
    display: flex; flex-direction: column; gap: 10px; padding: 12px;
    border-radius: 10px; background: #0b0b0b; border: 0;
    text-align: left; min-width: 0;
    transition: border-color .15s ease-out, background-color .15s ease-out, transform .15s var(--ease);
  }
  .compact .opt { padding: 10px; gap: 8px; }
  .opt:hover { background: #18181b; }
  /* No outlines: the picked card is the lighter one. */
  .opt.on { background: #26262b; }
  .opt:active { transform: scale(.985); }

  /* The glow is the stage's own background and the characters sit in normal
     flow: no clipped, layered children, which some graphics drivers left
     blank (empty cards). The first background is for WebViews without
     color-mix; the second follows the theme's accent. */
  .stage {
    position: relative; height: 74px; border-radius: 7px;
    background: radial-gradient(66% 80% at 62% 40%, rgba(255, 95, 210, .55), rgba(255, 95, 210, 0) 70%), #050505;
    background: radial-gradient(66% 80% at 62% 40%, color-mix(in srgb, var(--accent) 55%, transparent), transparent 70%), #050505;
  }
  .stage.lite {
    background: radial-gradient(66% 80% at 62% 40%, rgba(255, 95, 210, .33), rgba(255, 95, 210, 0) 70%), #050505;
    background: radial-gradient(66% 80% at 62% 40%, color-mix(in srgb, var(--accent) 33%, transparent), transparent 70%), #050505;
  }
  .compact .stage { height: 64px; }
  .compact .chars { padding: 2px 10px; }
  .chars {
    height: 100%; margin: 0; padding: 8px 10px;
    font: 500 10px/12px var(--mono, ui-monospace, monospace); letter-spacing: 1px;
    color: rgba(255, 255, 255, .55); white-space: pre; overflow: hidden;
  }
  .stage.lite .chars { color: rgba(255, 255, 255, .32); }
  .rec {
    position: absolute; left: 8px; top: 6px; padding: 1px 6px; border-radius: 4px;
    font-size: 10px; font-weight: 600; color: #000; background: var(--accent);
  }
  .fps { position: absolute; right: 8px; bottom: 6px; font-size: 10px; color: rgba(255, 255, 255, .5); }
  .opt.on .fps { color: rgba(255, 255, 255, .8); }

  .meta { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .lbl { font-size: 13px; font-weight: 500; color: rgba(255, 255, 255, .78); }
  .opt.on .lbl { color: #fff; font-weight: 600; }
  .hint { font-size: 11.5px; color: rgba(255, 255, 255, .48); }
  .opt.on .hint { color: rgba(255, 255, 255, .68); }

  @media (max-width: 480px) { .grid:not(.compact) { grid-template-columns: 1fr; } }
</style>
