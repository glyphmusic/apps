<script lang="ts">
  // mini.rs frac_bar: a clickable / draggable bar, 4 px line, a 12 px white
  // knob, a taller invisible hit area (14 px, or 20 px wide when vertical);
  // `onpick` gets the fraction under the mouse (0 at the left / bottom).
  let { value, vertical = false, track, fill, onpick, wheelStep = 0 }: {
    value: number; vertical?: boolean; track: string; fill: string; onpick: (f: number) => void; wheelStep?: number;
  } = $props();
  let el: HTMLDivElement;
  const v = $derived(Math.min(1, Math.max(0, value)));
  function frac(e: PointerEvent) {
    const r = el.getBoundingClientRect();
    return Math.min(1, Math.max(0, vertical ? 1 - (e.clientY - r.top) / r.height : (e.clientX - r.left) / r.width));
  }
  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    e.stopPropagation();
    el.setPointerCapture(e.pointerId);
    onpick(frac(e));
  }
  function move(e: PointerEvent) { if (e.buttons & 1) onpick(frac(e)); }
  function wheel(e: WheelEvent) {
    if (wheelStep <= 0) return;
    e.preventDefault(); e.stopPropagation();
    onpick(Math.min(1, Math.max(0, v - Math.sign(e.deltaY) * wheelStep)));
  }
</script>

<div class="hit" class:vertical bind:this={el} onpointerdown={down} onpointermove={move} onwheel={wheel} role="slider" aria-valuenow={Math.round(v * 100)} tabindex="-1">
  <div class="line" style:background={track}>
    <div class="fill" style:background={fill} style:width={vertical ? undefined : `${v * 100}%`} style:height={vertical ? `${v * 100}%` : undefined}></div>
    <div class="knob" style:left={vertical ? "-4px" : `calc(${v * 100}% - 6px)`} style:bottom={vertical ? `calc(${v * 100}% - 6px)` : undefined}></div>
  </div>
</div>

<style>
  .hit { position: relative; display: flex; align-items: center; width: 100%; height: 14px; cursor: pointer; touch-action: none; }
  .hit.vertical { width: 20px; height: 100%; justify-content: center; }
  .line { position: relative; width: 100%; height: 4px; border-radius: 2px; }
  .vertical .line { width: 4px; height: 100%; }
  .fill { position: absolute; left: 0; top: 0; bottom: 0; border-radius: 2px; }
  .vertical .fill { top: auto; right: 0; width: 100%; }
  .knob { position: absolute; top: -4px; width: 12px; height: 12px; border-radius: 50%; background: #fff; pointer-events: none; }
  .vertical .knob { top: auto; }
</style>
