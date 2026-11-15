<script lang="ts">
  // settings/eq.rs eq_curve: a smooth curve (Catmull-Rom, 16 samples per gap)
  // through ten bands with a filled area below (accent 45 % -> 2 %), points
  // to drag up and down (-12..+12 dB, half dB steps). H 210, labels column
  // 52 px (+12dB / 0dB / -12dB, 12 px), grid: a line per band (white 6 %)
  // and the 0 dB line (white 14 %). Stroke 2.5. Points r 5 white (r 7 and a
  // soft ring 24 px accent 25 % while dragged). Anywhere in a band's column
  // picks it; double-click puts a band back to 0 dB; the wheel steps 0.5 dB.
  // Names under the curve (12 semibold, white 65 %) and the gains (mono 11.5,
  // accent 90 %, 0 = white 35 %). Off: curve and fill at 40 %.
  import { app } from "../state.svelte";
  import { eqGains, eqMatch, setEq, setEqLive } from "./eq";
  let { on }: { on: boolean } = $props();
  const H = 210, LEFT = 52;
  let width = $state(600);
  let drag = $state<number | null>(null);
  let box: HTMLDivElement | undefined = $state();
  const gains = $derived(eqGains(app.config.audio.eq_gains));
  const plotW = $derived(Math.max(100, width - LEFT));
  const cell = $derived(plotW / 10);
  const names = $derived(plotW < 420 ? ["32", "64", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"] : ["32Hz", "64Hz", "125Hz", "250Hz", "500Hz", "1kHz", "2kHz", "4kHz", "8kHz", "16kHz"]);
  const yOf = (g: number) => ((12 - Math.min(12, Math.max(-12, g))) / 24) * H;
  const xs = $derived(Array.from({ length: 10 }, (_, i) => cell * (i + 0.5)));
  const pts = $derived(gains.map((g, i) => [xs[i], yOf(g)] as const));
  const curve = $derived.by(() => {
    const out: (readonly [number, number])[] = [];
    for (let i = 0; i < 9; i++) {
      const p0 = pts[Math.max(0, i - 1)], p1 = pts[i], p2 = pts[i + 1], p3 = pts[Math.min(9, i + 2)];
      for (let k = 0; k < 16; k++) {
        const t = k / 16, t2 = t * t, t3 = t2 * t;
        const f = (a: number, b: number, c: number, d: number) => 0.5 * (2 * b + (-a + c) * t + (2 * a - 5 * b + 4 * c - d) * t2 + (-a + 3 * b - 3 * c + d) * t3);
        out.push([f(p0[0], p1[0], p2[0], p3[0]), Math.min(H, Math.max(0, f(p0[1], p1[1], p2[1], p3[1])))] as const);
      }
    }
    out.push(pts[9]);
    return out;
  });
  const line = $derived(curve.map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)} ${y.toFixed(1)}`).join(" "));
  const area = $derived(`${line} L${curve[curve.length - 1][0].toFixed(1)} ${H} L${curve[0][0].toFixed(1)} ${H} Z`);
  const alpha = $derived(on ? 1 : 0.4);

  const gainAt = (y: number) => Math.min(12, Math.max(-12, Math.round((12 - (y / H) * 24) * 2) / 2));
  const bandAt = (x: number) => Math.min(9, Math.max(0, Math.floor(x / cell)));
  function local(e: { clientX: number; clientY: number }) {
    const r = box!.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  }
  function apply(band: number, g: number, save: boolean) {
    const gs = [...gains];
    gs[band] = g;
    // A curve that equals a preset shows that preset's name.
    const preset = eqMatch(app.config.audio.eq_preset, gs, app.config.audio.eq_user_presets);
    if (save) setEq(true, preset, gs); else setEqLive(true, preset, gs);
  }
  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    const p = local(e), band = bandAt(p.x);
    if (e.detail >= 2) { drag = null; apply(band, 0, true); return; }
    drag = band;
    box!.setPointerCapture(e.pointerId);
    apply(band, gainAt(p.y), false);
  }
  function move(e: PointerEvent) { if (drag != null) apply(drag, gainAt(local(e).y), false); }
  function up(e: PointerEvent) { if (drag == null) return; const b = drag; drag = null; apply(b, gainAt(local(e).y), true); }
  let rest = 0;
  function wheel(e: WheelEvent) {
    e.preventDefault();
    let n = 0;
    if (e.deltaMode === 1 || Math.abs(e.deltaY) >= 50) n = -Math.sign(e.deltaY);
    else { rest += -e.deltaY; n = Math.trunc(rest / 40); rest -= n * 40; }
    if (!n) return;
    const band = bandAt(local(e).x);
    apply(band, Math.min(12, Math.max(-12, gains[band] + n * 0.5)), true);
  }
</script>

<div class="eq" bind:clientWidth={width}>
  <div class="plot" style:height="{H}px">
    <div class="ax" style:top="-7px"><b>+12dB</b></div>
    <div class="ax mid" style:top="{H / 2 - 8}px">0dB</div>
    <div class="ax" style:bottom="-7px"><b>-12dB</b></div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="area" style:left="{LEFT}px" bind:this={box} onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={() => (drag = null)} onwheel={wheel}>
      <svg width={plotW} height={H} viewBox="0 0 {plotW} {H}" aria-hidden="true">
        <defs>
          <linearGradient id="eq-fill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stop-color="var(--accent)" stop-opacity={0.45 * alpha} />
            <stop offset="1" stop-color="var(--accent)" stop-opacity="0.02" />
          </linearGradient>
        </defs>
        {#each xs as x}<rect x={x - 0.5} y="0" width="1" height={H} fill="rgba(255,255,255,.06)" />{/each}
        <rect x="0" y={yOf(0) - 0.5} width={plotW} height="1" fill="rgba(255,255,255,.14)" />
        <path d={area} fill="url(#eq-fill)" />
        <path d={line} fill="none" stroke="var(--accent)" stroke-opacity={alpha} stroke-width="2.5" stroke-linejoin="round" stroke-linecap="round" />
        {#each pts as [x, y], i}
          {#if drag === i}<circle cx={x} cy={y} r="12" fill="var(--accent)" fill-opacity=".25" />{/if}
          <circle cx={x} cy={y} r={drag === i ? 7 : 5} fill="#fff" fill-opacity={on ? 1 : 0.6} class="pt" />
        {/each}
      </svg>
    </div>
  </div>
  <div class="names" style:margin-left="{LEFT}px">{#each names as n}<span>{n}</span>{/each}</div>
  <div class="vals mono" style:margin-left="{LEFT}px">
    {#each gains as g}<span class:z={Math.abs(g) < 0.05}>{Math.abs(g) < 0.05 ? "0" : `${g > 0 ? "+" : ""}${g.toFixed(1)}`}</span>{/each}
  </div>
</div>

<style>
  .eq { display: flex; flex-direction: column; gap: 10px; }
  .plot { position: relative; }
  .ax { position: absolute; left: 0; font-size: 12px; color: rgba(255, 255, 255, .7); }
  .ax b { font-weight: 600; }
  .ax.mid { color: rgba(255, 255, 255, .4); }
  .area { position: absolute; right: 0; top: 0; bottom: 0; cursor: pointer; touch-action: none; }
  svg { display: block; overflow: visible; }
  .pt { transition: r .12s ease-out; }
  .names, .vals { display: flex; }
  .names span { flex: 1; text-align: center; font-size: 12px; font-weight: 600; color: rgba(255, 255, 255, .65); }
  .vals span { flex: 1; text-align: center; font-size: 11.5px; color: color-mix(in srgb, var(--accent) 90%, transparent); }
  .vals span.z { color: rgba(255, 255, 255, .35); }
</style>
