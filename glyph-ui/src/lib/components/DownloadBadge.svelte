<script lang="ts">
  // widgets/indicators.rs download_badge: while downloading a ring fills
  // around the arrow (accent, clockwise from 12; a spinning arc while the
  // size is unknown); done = the accent check that pops in.
  import Icon from "./Icon.svelte";
  let { state, progress = null, size = 16, arrow = "arrow-down" }: { state: "none" | "busy" | "done"; progress?: number | null; size?: number; arrow?: string } = $props();
  const stroke = $derived(Math.max(1.5, size / 9));
  const r = $derived(size / 2 - stroke / 2);
  const c = $derived(2 * Math.PI * r);
</script>

{#if state === "busy"}
  <span class="db" style:width="{size}px" style:height="{size}px">
    <svg width={size} height={size} class:spin={progress == null}>
      <circle cx={size / 2} cy={size / 2} {r} fill="none" stroke="rgba(255,255,255,.16)" stroke-width={stroke} />
      <circle cx={size / 2} cy={size / 2} {r} fill="none" stroke="var(--accent)" stroke-width={stroke} stroke-dasharray="{c * (progress ?? 0.28)} {c}" transform="rotate(-90 {size / 2} {size / 2})" />
    </svg>
    <Icon name={arrow} size={size * 0.58} color="var(--text)" />
  </span>
{:else if state === "done"}
  <span class="db pop" style:width="{size}px" style:height="{size}px"><Icon name="circle-check" {size} color="var(--accent)" /></span>
{/if}

<style>
  .db { position: relative; flex: none; display: grid; place-items: center; }
  .db svg { position: absolute; inset: 0; }
  .spin { animation: spin 0.9s linear infinite; }
  .pop { animation: dl-pop .5s var(--ease); }
  @keyframes dl-pop { 0% { transform: scale(.4); opacity: 0; } 60% { transform: scale(1.15); opacity: 1; } 100% { transform: scale(1); } }
</style>
