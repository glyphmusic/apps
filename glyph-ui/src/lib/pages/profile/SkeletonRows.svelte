<script lang="ts">
  // widgets/skeleton.rs skeleton_rows: n rows (gap 2) of a 44 px cover, a
  // title bar (12 high) and an artist bar (10 high) in varying widths and a
  // small duration box; px 8, py 6, gap 12.
  import Skeleton from "../../components/Skeleton.svelte";
  let { prefix = "sk", n = 5 }: { prefix?: string; n?: number } = $props();
  const T = [0.42, 0.55, 0.36, 0.48, 0.6, 0.4], A = [0.24, 0.3, 0.2, 0.27, 0.33, 0.22];
</script>

<div class="rows">
  {#each Array.from({ length: n }, (_, i) => i) as i (i)}
    <div class="r">
      <Skeleton w={44} h={44} radius={6} id="{prefix}-c{i}" />
      <div class="m">
        <div style:width="{T[i % 6] * 100}%"><Skeleton w="100%" h={12} radius={4} id="{prefix}-t{i}" /></div>
        <div style:width="{A[i % 6] * 100}%"><Skeleton w="100%" h={10} radius={4} id="{prefix}-a{i}" /></div>
      </div>
      <Skeleton w={32} h={10} radius={4} id="{prefix}-d{i}" />
    </div>
  {/each}
</div>

<style>
  .rows { display: flex; flex-direction: column; gap: 2px; }
  .r { display: flex; align-items: center; gap: 12px; padding: 6px 8px; }
  .m { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
</style>
