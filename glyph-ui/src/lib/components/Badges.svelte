<script lang="ts">
  // widgets/buttons.rs badge / badges: the source as a small chip (`YT`);
  // hovering names the source.
  import { PROVIDER_BADGE, PROVIDER_NAME, sources, type ProviderId, type Track } from "../api/types";
  import { tip } from "../ui.svelte";
  import { tf } from "../i18n.svelte";
  let { track, source }: { track?: Track; source?: ProviderId | null } = $props();
  const list = $derived(source ? [source] : track ? sources(track) : []);
</script>

<span class="bs">
  {#each list as p (p)}
    <span class="b" use:tip={tf("tip.source", { name: PROVIDER_NAME[p] })}>{PROVIDER_BADGE[p]}</span>
  {/each}
</span>

<style>
  .bs { display: flex; flex: none; gap: 2px; }
  .b { padding: 0 4px; border-radius: 3px; font-size: 10px; font-weight: 600; line-height: 15px; color: var(--dim); background: rgba(255, 255, 255, .07); }
</style>
