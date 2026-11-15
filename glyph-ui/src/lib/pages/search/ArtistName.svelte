<script lang="ts">
  // lists.rs artist_name_badged + verified_badge: the name (truncates) with
  // a round blue check badge right of it for verified artists (gap 6;
  // hsl 212 90 % 56 %, white check 62 % of the badge). Uses lightweight
  // `artist_verified` lookups instead of fetching full artist pages.
  import Icon from "../../components/Icon.svelte";
  import { app } from "../../state.svelte";
  import { tip } from "../../ui.svelte";
  import { t } from "../../i18n.svelte";
  import type { Snippet } from "svelte";
  let { name, badge = 14, children }: { name: string; badge?: number; children?: Snippet } = $props();
  const verified = $derived(app.isVerifiedArtist(name));
</script>

<span class="an">
  <span class="n trunc">{#if children}{@render children()}{:else}{name}{/if}</span>
  {#if verified}
    <span class="vb" style:width="{badge}px" style:height="{badge}px" use:tip={t("tip.verified")}><Icon name="check" size={badge * 0.62} color="#fff" /></span>
  {/if}
</span>

<style>
  .an { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .n { min-width: 0; }
  .vb { flex: none; border-radius: 50%; background: hsl(212 90% 56%); display: grid; place-items: center; }
</style>
