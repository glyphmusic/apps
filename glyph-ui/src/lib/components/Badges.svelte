<script lang="ts">
  // widgets/buttons.rs badge / badges: `[YT]` source badges, mono xs faint;
  // hovering names the source.
  import { PROVIDER_BADGE, PROVIDER_NAME, sources, type ProviderId, type Track } from "../api/types";
  import { tip } from "../ui.svelte";
  import { tf } from "../i18n.svelte";
  let { track, source }: { track?: Track; source?: ProviderId | null } = $props();
  const list = $derived(source ? [source] : track ? sources(track) : []);
</script>

<span class="bs">
  {#each list as p (p)}
    <span class="b mono" use:tip={tf("tip.source", { name: PROVIDER_NAME[p] })}>[{PROVIDER_BADGE[p]}]</span>
  {/each}
</span>

<style>
  .bs { display: flex; flex: none; gap: 2px; }
  .b { font-size: var(--xs); color: var(--faint); }
</style>
