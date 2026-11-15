<script lang="ts">
  // widgets/skeleton.rs skeleton_rows (cover 44, two bars, duration; gap
  // 12, px 8, py 6) and indicators.rs skeleton_artist_rows (round 48,
  // name bar, chip; gap 14, px 10, py 6). Bar widths vary a little.
  import Skeleton from "../../components/Skeleton.svelte";
  let { kind, n, prefix }: { kind: "rows" | "artists"; n: number; prefix: string } = $props();
  const TW = [0.42, 0.55, 0.36, 0.48, 0.6, 0.4], AW = [0.24, 0.3, 0.2, 0.27, 0.33, 0.22], NW = [0.3, 0.22, 0.26];
</script>

<div class="col">
  {#each Array(n) as _, i}
    {#if kind === "rows"}
      <div class="row">
        <Skeleton w={44} h={44} radius={6} id="{prefix}-sk-c{i}" />
        <div class="bars">
          <div style:width="{TW[i % 6] * 100}%"><Skeleton w="100%" h={12} radius={4} id="{prefix}-sk-t{i}" /></div>
          <div style:width="{AW[i % 6] * 100}%"><Skeleton w="100%" h={10} radius={4} id="{prefix}-sk-a{i}" /></div>
        </div>
        <Skeleton w={32} h={10} radius={4} id="{prefix}-sk-d{i}" />
      </div>
    {:else}
      <div class="row art">
        <Skeleton w={48} h={48} radius={24} id="{prefix}-ska-c{i}" />
        <div class="bars"><div style:width="{NW[i % 3] * 100}%"><Skeleton w="100%" h={13} radius={4} id="{prefix}-ska-n{i}" /></div></div>
        <Skeleton w={46} h={18} radius={4} id="{prefix}-ska-k{i}" />
      </div>
    {/if}
  {/each}
</div>

<style>
  .col { display: flex; flex-direction: column; gap: 2px; }
  .row { display: flex; align-items: center; gap: 12px; padding: 6px 8px; }
  .row.art { gap: 14px; padding: 6px 10px; }
  .bars { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
</style>
