<script lang="ts">
  // stats.rs bars: vertical bars with labels under them (hours, weekdays,
  // months). Row h 140, gap 4; each column min-w 6 with a bar up to 110 high
  // (at least 3 when > 0, else 1), radius 4 on top, accent (the highlighted
  // one) or accent at 25 + 55 % of its share; label mono 10 faint, h 14.
  // The bars grow in (ease-out 350 ms).
  let { values, labels, highlight = null }: { values: number[]; labels: string[]; highlight?: number | null } = $props();
  const max = $derived(Math.max(1, ...values));
</script>

<div class="bars">
  {#each values as v, i (i)}
    {@const k = v / max}
    <div class="col">
      <div class="bar" class:hl={highlight === i} style:height="{Math.max(110 * k, v > 0 ? 3 : 1)}px" style:--a={0.25 + 0.55 * k} style:animation-delay="{Math.min(i, 24) * 12}ms"></div>
      <div class="lb mono">{labels[i] ?? ""}</div>
    </div>
  {/each}
</div>

<style>
  .bars { display: flex; align-items: flex-end; gap: 4px; height: 140px; }
  .col { flex: 1; min-width: 6px; height: 100%; display: flex; flex-direction: column; justify-content: flex-end; align-items: center; gap: 4px; }
  .bar { width: 100%; border-radius: 4px 4px 0 0; background: var(--accent); opacity: var(--a); transform-origin: bottom; animation: grow .35s var(--ease) both; }
  .bar.hl { opacity: 1; }
  .lb { height: 14px; font-size: 10px; color: var(--faint); white-space: nowrap; }
  @keyframes grow { from { transform: scaleY(0); } to { transform: scaleY(1); } }
</style>
