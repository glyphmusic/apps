<script lang="ts">
  // Loading card of a Home row: same frame as a song card (w 192, p 8,
  // gap 4), ASCII skeleton for the cover (176, radius 8), title and artist.
  import Skeleton from "../../components/Skeleton.svelte";
  let { id, i = 0 }: { id: string; i?: number } = $props();
</script>

<div class="card">
  <Skeleton w={176} h={176} radius={8} id="{id}-c{i}" />
  <div class="t"><Skeleton w="{[0.7, 0.55, 0.62][i % 3] * 100}%" h={12} radius={4} id="{id}-t{i}" /></div>
  <Skeleton w="{[0.4, 0.5, 0.35][i % 3] * 100}%" h={10} radius={4} id="{id}-a{i}" />
</div>

<style>
  .card { flex: none; width: 192px; padding: 8px; display: flex; flex-direction: column; gap: 4px; }
  .t { padding: 4px 0 4px; }
</style>
