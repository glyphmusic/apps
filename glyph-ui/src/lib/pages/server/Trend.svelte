<script lang="ts">
  // Server dashboard history: one series per day as a 2 px line over a soft
  // area, recessive grid, a crosshair with the day's value on hover / touch.
  let { days, values, fmt, height = 150 }: { days: string[]; values: number[]; fmt: (v: number) => string; height?: number } = $props();

  let w = $state(600);
  const PAD_L = 44, PAD_R = 8, PAD_T = 10, PAD_B = 22;
  const max = $derived(Math.max(1, ...values) * 1.1);
  const n = $derived(values.length);
  const x = (i: number) => PAD_L + (n <= 1 ? 0 : (i / (n - 1)) * (w - PAD_L - PAD_R));
  const y = (v: number) => PAD_T + (1 - v / max) * (height - PAD_T - PAD_B);
  const line = $derived(values.map((v, i) => `${i ? "L" : "M"}${x(i).toFixed(1)},${y(v).toFixed(1)}`).join(""));
  const area = $derived(n ? `${line}L${x(n - 1).toFixed(1)},${height - PAD_B}L${x(0).toFixed(1)},${height - PAD_B}Z` : "");
  const grid = $derived([0, 0.5, 1].map((k) => ({ y: y(max * k / 1.1), label: fmt(Math.round((max / 1.1) * k)) })));
  const ticks = $derived(n ? [...new Set([0, Math.floor((n - 1) / 2), n - 1])] : []);
  let hot = $state<number | null>(null);
  function move(e: PointerEvent) {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect();
    const px = e.clientX - r.left;
    hot = n ? Math.max(0, Math.min(n - 1, Math.round(((px - PAD_L) / Math.max(1, w - PAD_L - PAD_R)) * (n - 1)))) : null;
  }
  const short = (d: string) => `${d.slice(8, 10)}.${d.slice(5, 7)}.`;
</script>

<div class="trend" bind:clientWidth={w} style:height="{height}px">
  <svg width={w} {height} role="img" aria-label={days.map((d, i) => `${d}: ${fmt(values[i])}`).join(", ")}
    onpointermove={move} onpointerdown={move} onpointerleave={() => (hot = null)}>
    <defs>
      <linearGradient id="trend-fill" x1="0" x2="0" y1="0" y2="1">
        <stop offset="0" stop-color="var(--accent)" stop-opacity=".22" />
        <stop offset="1" stop-color="var(--accent)" stop-opacity="0" />
      </linearGradient>
    </defs>
    {#each grid as g}
      <line x1={PAD_L} x2={w - PAD_R} y1={g.y} y2={g.y} class="grid" />
      <text x={PAD_L - 8} y={g.y + 3} text-anchor="end" class="ax">{g.label}</text>
    {/each}
    {#each ticks as i (i)}
      <text x={x(i)} y={height - 6} text-anchor={i === 0 ? "start" : i === n - 1 ? "end" : "middle"} class="ax">{short(days[i] ?? "")}</text>
    {/each}
    {#if n > 1}
      <path d={area} fill="url(#trend-fill)" />
      <path d={line} class="ln" />
    {:else if n === 1}
      <circle cx={x(0)} cy={y(values[0])} r="4" fill="var(--accent)" />
    {/if}
    {#if hot != null}
      <line x1={x(hot)} x2={x(hot)} y1={PAD_T} y2={height - PAD_B} class="cross" />
      <circle cx={x(hot)} cy={y(values[hot])} r="4.5" fill="var(--accent)" stroke="#0d0d0d" stroke-width="2" />
    {/if}
  </svg>
  {#if hot != null}
    <div class="tip" style:left="{Math.min(w - 120, Math.max(0, x(hot) - 60))}px">
      <span class="td">{days[hot]}</span>
      <span class="tv">{fmt(values[hot])}</span>
    </div>
  {/if}
</div>

<style>
  .trend { position: relative; width: 100%; touch-action: pan-y; }
  svg { display: block; overflow: visible; }
  .grid { stroke: rgba(255, 255, 255, .06); stroke-width: 1; }
  .ax { font-size: 10px; fill: var(--faint); font-family: var(--mono); }
  .ln { fill: none; stroke: var(--accent); stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; }
  .cross { stroke: rgba(255, 255, 255, .25); stroke-width: 1; }
  .tip { position: absolute; top: -6px; width: 120px; padding: 6px 10px; border-radius: 8px; background: #1c1c1e; display: flex; flex-direction: column; gap: 2px; pointer-events: none; }
  .td { font-size: var(--xs); color: var(--dim); font-family: var(--mono); }
  .tv { font-size: var(--sm); font-weight: 600; color: var(--text); }
</style>
