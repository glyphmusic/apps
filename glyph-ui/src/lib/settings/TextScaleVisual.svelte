<script lang="ts">
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import VisualCard from "./VisualCard.svelte";

  const STEPS = [0.9, 0.95, 1.0, 1.05, 1.1, 1.15, 1.2, 1.25, 1.3] as const;

  const scale = $derived(Math.min(1.3, Math.max(0.9, app.config.ui.text_scale ?? 1)));
  const activeIdx = $derived.by(() => {
    let best = 0;
    let minD = Infinity;
    for (let i = 0; i < STEPS.length; i++) {
      const d = Math.abs(scale - STEPS[i]);
      if (d < minD) { minD = d; best = i; }
    }
    return best;
  });
  const bucket = $derived<"compact" | "standard" | "enlarged">(
    scale < 0.975 ? "compact" : scale <= 1.05 ? "standard" : "enlarged",
  );
  const isDefault = $derived(Math.abs(scale - 1) < 0.01);

  function pick(k: number) {
    const next = Math.round(Math.min(1.3, Math.max(0.9, k)) * 20) / 20;
    app.setConfig((c) => { c.ui.text_scale = next; });
  }

  let rest = 0;
  function wheel(e: WheelEvent) {
    e.preventDefault();
    let n = 0;
    if (e.deltaMode === 1 || Math.abs(e.deltaY) >= 50) n = -Math.sign(e.deltaY);
    else { rest += -e.deltaY; n = Math.trunc(rest / 40); rest -= n * 40; }
    if (!n) return;
    const nextIdx = Math.min(STEPS.length - 1, Math.max(0, activeIdx + n));
    pick(STEPS[nextIdx]);
  }
</script>

<VisualCard label={t("sp.appearance.text")} info={t("sp.appearance.text.desc")}>
  {#snippet right()}
    <span class="mono pct">{Math.round(scale * 100)} %</span>
  {/snippet}

  <div class="thumbs" role="group" aria-label={t("sp.appearance.text")}>
    <button class="thumb" class:on={bucket === "compact"} onclick={() => pick(0.9)}>
      <div class="box">
        <div class="wf compact">
          <div class="wf-head h5">
            <span></span><span></span><span></span><span></span><span></span>
          </div>
          <div class="wf-grid g5">
            {#each Array(15) as _}<span></span>{/each}
          </div>
        </div>
      </div>
      <span class="tl">{t("sp.appearance.text.compact")}</span>
    </button>

    <button class="thumb" class:on={bucket === "standard"} onclick={() => pick(1.0)}>
      <div class="box">
        <div class="wf standard">
          <div class="wf-head h4">
            <span></span><span></span><span></span><span></span>
          </div>
          <div class="wf-grid g4">
            {#each Array(8) as _}<span></span>{/each}
          </div>
        </div>
      </div>
      <span class="tl">{t("sp.appearance.text.standard")}</span>
    </button>

    <button class="thumb" class:on={bucket === "enlarged"} onclick={() => pick(1.3)}>
      <div class="box">
        <div class="wf enlarged">
          <div class="wf-cols">
            <div class="wf-lines"><span></span><span></span><span></span></div>
            <div class="wf-lines"><span></span><span></span><span></span></div>
          </div>
          <div class="wf-grid g3">
            {#each Array(3) as _}<span></span>{/each}
          </div>
        </div>
      </div>
      <span class="tl">{t("sp.appearance.text.enlarged")}</span>
    </button>
  </div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="stops" onwheel={wheel} role="radiogroup" aria-label={t("sp.appearance.text")}>
    {#each STEPS as step, i (step)}
      {@const on = i === activeIdx}
      <button
        class="stop"
        class:on
        class:minor={i % 2 === 1}
        role="radio"
        aria-checked={on}
        onclick={() => pick(step)}
      >
        <span class="trk">
          <span class="seg" class:vis={i > 0}></span>
          <span class="seg" class:vis={i + 1 < STEPS.length}></span>
          <span class="dot"></span>
        </span>
        <span class="sl">{Math.round(step * 100)} %</span>
      </button>
    {/each}
  </div>

  <div class="foot">
    <button class="reset" disabled={isDefault} onclick={() => pick(1.0)}>
      {t("sp.appearance.text.reset")}
    </button>
  </div>
</VisualCard>

<style>
  .pct {
    padding: 2px 8px;
    border-radius: 6px;
    background: rgba(255, 255, 255, .07);
    font-size: 12px;
    color: rgba(255, 255, 255, .82);
  }

  .thumbs {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 12px;
    padding-top: 2px;
  }
  .thumb {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 4px 0;
  }
  .box {
    width: 112px;
    max-width: 100%;
    height: 74px;
    padding: 6px;
    border-radius: 6px;
    background: #0a0a0a;
    transition: background-color .15s ease-out, transform .15s var(--ease);
  }
  .thumb:hover .box { background: #18181b; }
  .thumb.on .box { background: #2a2a30; }
  .thumb:active .box { transform: scale(.97); }

  .wf { width: 100%; height: 100%; display: flex; flex-direction: column; justify-content: space-between; gap: 4px; }
  .wf-head { display: grid; gap: 2.5px; }
  .wf-head.h5 { grid-template-columns: repeat(5, 1fr); }
  .wf-head.h4 { grid-template-columns: repeat(4, 1fr); }
  .wf-head span { height: 4px; border-radius: 1px; background: rgba(255, 255, 255, .2); transition: background-color .15s; }
  .wf-head.h5 span { height: 3px; }
  .wf-grid { display: grid; gap: 2.5px; flex: 1; }
  .wf-grid.g5 { grid-template-columns: repeat(5, 1fr); grid-template-rows: repeat(3, 1fr); gap: 2px; }
  .wf-grid.g4 { grid-template-columns: repeat(4, 1fr); grid-template-rows: repeat(2, 1fr); gap: 3px; }
  .wf-grid.g3 { grid-template-columns: repeat(3, 1fr); gap: 4px; flex: 0 0 24px; }
  .wf-grid span { border-radius: 2px; background: rgba(255, 255, 255, .22); transition: background-color .15s; }

  .wf-cols { display: grid; grid-template-columns: 1fr 1fr; gap: 5px; }
  .wf-lines { display: flex; flex-direction: column; gap: 2.5px; }
  .wf-lines span { height: 5.5px; border-radius: 1.5px; background: rgba(255, 255, 255, .2); transition: background-color .15s; }

  .thumb.on .wf-head span,
  .thumb.on .wf-lines span {
    background: color-mix(in srgb, var(--accent) 82%, transparent);
  }
  .thumb.on .wf-grid span {
    background: var(--accent);
  }

  .tl {
    font-size: 13px;
    font-weight: 500;
    color: rgba(255, 255, 255, .72);
    transition: color .15s;
  }
  .thumb:hover .tl { color: #fff; }
  .thumb.on .tl { color: #fff; font-weight: 600; }

  .stops {
    display: flex;
    padding: 4px 0 2px;
  }
  .stop {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
  }
  .trk {
    position: relative;
    width: 100%;
    height: 18px;
    display: flex;
    align-items: center;
  }
  .seg {
    flex: 1;
    height: 1px;
  }
  .seg.vis { background: rgba(255, 255, 255, .18); }
  .dot {
    position: absolute;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    width: 13px;
    height: 13px;
    border-radius: 50%;
    background: #141414;
    border: 1.5px solid rgba(255, 255, 255, .36);
    transition: border-color .15s, border-width .15s, transform .15s var(--ease);
  }
  .stop:hover .dot { border-color: rgba(255, 255, 255, .8); transform: translate(-50%, -50%) scale(1.08); }
  .stop.on .dot {
    width: 15px;
    height: 15px;
    border: 4px solid var(--accent);
    background: #0e0e0e;
  }
  .sl {
    font-size: 11.5px;
    color: rgba(255, 255, 255, .52);
    white-space: nowrap;
    transition: color .15s;
  }
  .stop:hover .sl { color: rgba(255, 255, 255, .85); }
  .stop.on .sl { color: #fff; font-weight: 600; }

  .foot {
    display: flex;
    justify-content: flex-end;
    padding-top: 2px;
  }
  .reset {
    height: 32px;
    padding: 0 18px;
    border-radius: 999px;
    background: rgba(255, 255, 255, .1);
    font-size: 13px;
    font-weight: 600;
    color: #fff;
    transition: background-color .15s, border-color .15s, opacity .15s;
  }
  .reset:hover:not(:disabled) { background: rgba(255, 255, 255, .16); }
  .reset:disabled { opacity: .35; cursor: default; }

  @media (max-width: 480px) {
    .thumbs { gap: 8px; }
    .box { height: 60px; padding: 5px; }
    .tl { font-size: 12px; }
    .stop.minor:not(.on) .sl { opacity: 0; }
    .sl { font-size: 10.5px; }
  }
</style>
