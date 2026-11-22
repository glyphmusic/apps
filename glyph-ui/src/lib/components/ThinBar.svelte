<script lang="ts">
  // widgets/bars.rs thin_bar: 14 px hit height, 4 px track white 18 %, fill
  // in text colour. Hover / drag: 5 px, accent fill, 11 px white knob, value
  // label above. Seek bars (`live` false) seek on release; volume applies
  // while dragging. The wheel steps by `step` (or calls `onwheelstep`).
  let {
    width = 340, value, onpick, live = true, label, step = 0.05, onwheelstep, vertical = false,
  }: {
    width?: number | string; value: number; onpick: (f: number) => void; live?: boolean; label?: (f: number) => string;
    step?: number; onwheelstep?: (n: number) => void; vertical?: boolean;
  } = $props();
  let el: HTMLDivElement;
  let drag = $state<number | null>(null);
  let hoverAt = $state<number | null>(null);
  const shown = $derived(Math.min(1, Math.max(0, drag ?? value)));
  const hot = $derived(drag != null || hoverAt != null);
  const at = $derived(Math.min(1, Math.max(0, drag ?? hoverAt ?? 0)));
  const len = $derived(typeof width === "number" ? `${width}px` : width);
  function frac(e: PointerEvent) {
    const r = el.getBoundingClientRect();
    return Math.min(1, Math.max(0, vertical ? 1 - (e.clientY - r.top) / r.height : (e.clientX - r.left) / r.width));
  }
  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    e.stopPropagation();
    el.setPointerCapture(e.pointerId);
    drag = frac(e);
    if (live) onpick(drag);
  }
  function move(e: PointerEvent) {
    const f = frac(e);
    hoverAt = f;
    if (drag != null) { drag = f; if (live) onpick(f); }
  }
  function up(e: PointerEvent) {
    if (drag == null) return;
    const f = frac(e);
    drag = null;
    onpick(f);
  }
  let rest = 0;
  function wheel(e: WheelEvent) {
    e.preventDefault();
    e.stopPropagation();
    let n = 0;
    if (e.deltaMode === 1 || Math.abs(e.deltaY) >= 50) n = -Math.sign(e.deltaY);
    else { rest += -e.deltaY; n = Math.trunc(rest / 40); rest -= n * 40; }
    if (!n) return;
    if (onwheelstep) onwheelstep(n); else onpick(Math.min(1, Math.max(0, value + n * step)));
  }
</script>

<div class="thin" class:hot class:vertical data-drag-own bind:this={el} style:width={vertical ? undefined : len} style:height={vertical ? len : undefined}
  onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={() => (drag = null)} onpointerleave={() => (hoverAt = null)} onwheel={wheel}
  role="slider" aria-valuenow={Math.round(value * 100)} tabindex="-1">
  <div class="track"><div class="fill" style:width={vertical ? undefined : `${shown * 100}%`} style:height={vertical ? `${shown * 100}%` : undefined}></div></div>
  {#if hot}
    <div class="knob" style:left={vertical ? undefined : `calc(${shown * 100}% - 5.5px)`} style:bottom={vertical ? `calc(${shown * 100}% - 5.5px)` : undefined}></div>
    {#if label && !vertical}<div class="lbl mono" style:left="{at * 100}%">{label(at)}</div>{/if}
  {/if}
</div>

<style>
  .thin { position: relative; flex: none; height: 14px; cursor: pointer; touch-action: none; }
  .thin::before { content: ""; position: absolute; inset: -4px; }
  .track { position: absolute; left: 0; right: 0; top: 5px; height: 4px; border-radius: 2px; background: rgba(255, 255, 255, .18); overflow: hidden; }
  .fill { height: 100%; background: var(--text); border-radius: inherit; }
  .hot .track { top: 4.5px; height: 5px; border-radius: 2.5px; }
  .hot .fill { background: var(--accent); }
  .knob { position: absolute; top: 1.5px; width: 11px; height: 11px; border-radius: 50%; background: #fff; pointer-events: none; }
  .lbl { position: absolute; bottom: 21px; transform: translateX(-50%); height: 19px; padding: 0 6px; border-radius: 5px; background: rgba(31, 31, 31, .98); color: rgba(255, 255, 255, .95); font-size: 11px; line-height: 19px; white-space: nowrap; pointer-events: none; z-index: 5; }
  .vertical { width: 14px; }
  .vertical .track { left: 5px; right: auto; top: 0; bottom: 0; width: 4px; height: auto; display: flex; align-items: flex-end; }
  .vertical .fill { width: 100%; }
  .vertical.hot .track { left: 4.5px; width: 5px; top: 0; height: auto; }
  .vertical .knob { left: 1.5px; top: auto; }
</style>
