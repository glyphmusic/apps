<script lang="ts">
  // views.rs section + section_head: the title (18 px icon, lg semibold)
  // sits close above its content (gap 10); "Show all" (text button, sm
  // medium) at the right end.
  import type { Snippet } from "svelte";
  import SectionTitle from "../../components/SectionTitle.svelte";
  import Button from "../../components/Button.svelte";
  import { t } from "../../i18n.svelte";
  let { icon, title, more, moreLabel, children }: { icon: string; title: string; more?: () => void; moreLabel?: string; children: Snippet } = $props();
</script>

<section class="sec">
  <SectionTitle {icon} {title}>
    {#if more}<span class="all"><Button kind="text" onclick={more}>{moreLabel ?? t("home.show_all")}</Button></span>{/if}
  </SectionTitle>
  {@render children()}
</section>

<style>
  .sec { display: flex; flex-direction: column; gap: 10px; min-width: 0; }
  .all { display: contents; }
  .all :global(.btn) { font-size: var(--sm); font-weight: 500; }
</style>
