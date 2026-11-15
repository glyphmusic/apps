<script lang="ts">
  // lists.rs artist_name_badged: the name (truncated) with the verified
  // badge right of it, gap 6.
  import VerifiedBadge from "./VerifiedBadge.svelte";
  import { isVerified } from "./actions";
  let { name, badge = 14 }: { name: string; badge?: number } = $props();
</script>

<span class="an"><span class="trunc">{name}</span>{#if isVerified(name)}<VerifiedBadge size={badge} />{/if}</span>

<style>
  .an { display: flex; align-items: center; gap: 6px; min-width: 0; }
</style>
