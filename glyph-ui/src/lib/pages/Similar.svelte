<script lang="ts">
  // lists/artist.rs similar_view: "Show all" of an artist's similar artists.
  // Column gap 16: back pill, heading "Name · Similar artists", then every
  // round artist card wrapping with gap 4 / ml -8; skeleton artist rows
  // while they load, else the empty state.
  import SectionTitle from "../components/SectionTitle.svelte";
  import EmptyState from "../components/EmptyState.svelte";
  import SkeletonSwap from "../components/SkeletonSwap.svelte";
  import BackPill from "./lists/BackPill.svelte";
  import ArtistCard from "./lists/ArtistCard.svelte";
  import SkeletonArtistRows from "./lists/SkeletonArtistRows.svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";

  const name = $derived(app.openArtist ?? "");
  const page = $derived(app.artists[name.toLowerCase()]);
  const similar = $derived(page?.similar ?? []);
  const loading = $derived(!page || page.loading);
</script>

<div class="sv">
  <BackPill />
  <SectionTitle heading title="{name} · {t('artist.similar')}" />
  <SkeletonSwap loading={!similar.length && loading}>
    {#snippet skeleton()}
      <SkeletonArtistRows prefix="similar" n={2} />
    {/snippet}
    {#if similar.length}
      <div class="grid" class:mob={app.compact}>{#each similar as n (n)}<ArtistCard name={n} fluid={app.compact} />{/each}</div>
    {:else}
      <EmptyState title={t("artist.similar")} />
    {/if}
  </SkeletonSwap>
</div>

<style>
  .sv { display: flex; flex-direction: column; gap: 16px; padding-bottom: 20px; }
  .grid { display: flex; flex-wrap: wrap; gap: 4px; margin-left: -8px; }
  /* Phones: two per row (the cards fill their column) instead of one big card per row. */
  .grid.mob { display: grid; grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: 4px; margin: 0 -8px; }
</style>
