<script lang="ts">
  // widgets/skeleton.rs skeleton_rows: `n` loading rows (cover 44 radius 6,
  // title / artist bars, duration), gap 2, px 8 py 6, gap 12.
  import Skeleton from "../../components/Skeleton.svelte";
  let { prefix, n = 5 }: { prefix: string; n?: number } = $props();
  const tw = [0.42, 0.55, 0.36, 0.48, 0.6, 0.4], aw = [0.24, 0.3, 0.2, 0.27, 0.33, 0.22];
</script>

<div class="sr">
  {#each Array(n) as _, i}
    <div class="r">
      <Skeleton w={44} h={44} radius={6} id="{prefix}-sk-c{i}" />
      <div class="tx">
        <Skeleton w="{tw[i % 6] * 100}%" h={12} radius={4} id="{prefix}-sk-t{i}" />
        <Skeleton w="{aw[i % 6] * 100}%" h={10} radius={4} id="{prefix}-sk-a{i}" />
      </div>
      <Skeleton w={32} h={10} radius={4} id="{prefix}-sk-d{i}" />
    </div>
  {/each}
</div>

<style>
  .sr { display: flex; flex-direction: column; gap: 2px; }
  .r { display: flex; align-items: center; gap: 12px; padding: 6px 8px; }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
</style>
