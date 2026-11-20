<script lang="ts">
  // Server dashboard ring chart: a 2 px gap between slices, hover (or tap) a
  // slice or its legend row and the middle shows that slice. The legend is
  // always there, so no slice is told apart by colour alone.
  import { fmtBytes, type Slice } from "../../admin/api";
  let { slices, total: totalLabel, size = 148 }: { slices: Slice[]; total: string; size?: number } = $props();

  const R = 60, W = 16, C = 2 * Math.PI * R, GAP = 2;
  const sum = $derived(slices.reduce((a, s) => a + Math.max(0, s.value), 0));
  const arcs = $derived.by(() => {
    let at = 0;
    const live = slices.filter((s) => s.value > 0);
    return slices.map((s) => {
      const len = sum > 0 ? (Math.max(0, s.value) / sum) * C : 0;
      const gap = live.length > 1 && len > 0 ? Math.min(GAP, len / 2) : 0;
      const a = { ...s, dash: Math.max(0, len - gap), off: -at, share: sum > 0 ? s.value / sum : 0 };
      at += len;
      return a;
    });
  });
  let hot = $state<number | null>(null);
  const pct = (x: number) => (x >= 0.995 ? "100 %" : x < 0.01 && x > 0 ? "<1 %" : `${Math.round(x * 100)} %`);
</script>

<div class="donut">
  <svg width={size} height={size} viewBox="0 0 148 148" role="img" aria-label={slices.map((s) => `${s.label} ${fmtBytes(s.value)}`).join(", ")}>
    <circle cx="74" cy="74" r={R} fill="none" stroke="rgba(255,255,255,.06)" stroke-width={W} />
    {#each arcs as a, i (a.label)}
      {#if a.dash > 0}
        <circle class="arc" class:dim={hot != null && hot !== i} cx="74" cy="74" r={R} fill="none" stroke={a.color} stroke-width={hot === i ? W + 4 : W}
          stroke-dasharray="{a.dash} {C}" stroke-dashoffset={a.off} transform="rotate(-90 74 74)"
          role="presentation" onpointerenter={() => (hot = i)} onpointerleave={() => (hot = null)} />
      {/if}
    {/each}
    <text x="74" y="72" text-anchor="middle" class="big">{hot != null ? fmtBytes(arcs[hot].value) : totalLabel}</text>
    <text x="74" y="90" text-anchor="middle" class="sm">{hot != null ? pct(arcs[hot].share) : ""}</text>
  </svg>
  <div class="legend">
    {#each arcs as a, i (a.label)}
      <button class="lg" class:on={hot === i} onpointerenter={() => (hot = i)} onpointerleave={() => (hot = null)} onclick={() => (hot = hot === i ? null : i)}>
        <span class="dot" style:background={a.color}></span>
        <span class="ln trunc">{a.label}</span>
        <span class="lv mono">{fmtBytes(a.value)}</span>
      </button>
    {/each}
  </div>
</div>

<style>
  .donut { display: flex; align-items: center; gap: 20px; }
  :global(html.compact) .donut { flex-direction: column; align-items: stretch; }
  :global(html.compact) .donut svg { align-self: center; }
  svg { flex: none; overflow: visible; }
  .arc { transition: stroke-width .15s var(--ease), opacity .15s; cursor: default; }
  .arc.dim { opacity: .35; }
  .big { font-size: 17px; font-weight: 700; fill: var(--text); }
  .sm { font-size: 11px; fill: var(--dim); font-family: var(--mono); }
  .legend { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .lg { display: flex; align-items: center; gap: 10px; height: 30px; padding: 0 8px; border-radius: 7px; transition: background-color .12s ease-out; }
  .lg:hover, .lg.on { background: rgba(255, 255, 255, .05); }
  .dot { flex: none; width: 10px; height: 10px; border-radius: 50%; }
  .ln { flex: 1; min-width: 0; font-size: var(--sm); color: var(--text); text-align: left; }
  .lv { flex: none; font-size: var(--xs); color: var(--dim); }
</style>
