<script lang="ts">
  // stats.rs tile: flex 1, min-w 150, p 16, radius 12, card fill, 1 px
  // border-faint, gap 4: label (sm dim), value (26 bold) and an
  // optional sub line (sm dim).
  let { label, value, sub, delay = 0 }: { label: string; value: string; sub?: string | null; delay?: number } = $props();
</script>

<div class="tile" style:animation-delay="{delay}ms">
  <div class="l">{label}</div>
  <div class="v">{value}</div>
  {#if sub}<div class="s trunc">{sub}</div>{/if}
</div>

<style>
  .tile { flex: 1; min-width: 150px; padding: 16px 18px; border-radius: 12px; background: var(--card); display: flex; flex-direction: column; gap: 6px; animation: rise-in .3s var(--ease) both; }
  .l { font-size: var(--sm); font-weight: 500; color: var(--dim); }
  .v { font-size: 26px; font-weight: 700; color: var(--text); letter-spacing: -.01em; }
  .s { font-size: var(--xs); color: var(--faint); }
</style>
