<script lang="ts">
  // widgets/indicators.rs skeleton_artist_rows: round 48 picture, name bar,
  // a chip; px 10 py 6, gap 14.
  import Skeleton from "../../components/Skeleton.svelte";
  let { prefix, n = 2 }: { prefix: string; n?: number } = $props();
  const nw = [0.3, 0.22, 0.26];
</script>

<div class="sr">
  {#each Array(n) as _, i}
    <div class="r">
      <Skeleton w={48} h={48} radius={24} id="{prefix}-ska-c{i}" />
      <div class="tx"><Skeleton w="{nw[i % 3] * 100}%" h={13} radius={4} id="{prefix}-ska-n{i}" /></div>
      <Skeleton w={46} h={18} radius={4} id="{prefix}-ska-k{i}" />
    </div>
  {/each}
</div>

<style>
  .sr { display: flex; flex-direction: column; gap: 2px; }
  .r { display: flex; align-items: center; gap: 14px; padding: 6px 10px; }
  .tx { flex: 1; min-width: 0; }
</style>
