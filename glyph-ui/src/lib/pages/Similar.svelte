<script lang="ts">
  // lists/artist.rs similar_view: "Show all" of an artist's similar artists.
  // Column gap 16: back pill, heading "Name · Similar artists", then every
  // round artist card wrapping with gap 4 / ml -8; skeleton artist rows
  // while they load, else the empty state.
  import SectionTitle from "../components/SectionTitle.svelte";
  import EmptyState from "../components/EmptyState.svelte";
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
  {#if similar.length}
    <div class="grid">{#each similar as n (n)}<ArtistCard name={n} />{/each}</div>
  {:else if loading}
    <SkeletonArtistRows prefix="similar" n={2} />
  {:else}
    <EmptyState title={t("artist.similar")} />
  {/if}
</div>

<style>
  .sv { display: flex; flex-direction: column; gap: 16px; padding-bottom: 20px; }
  .grid { display: flex; flex-wrap: wrap; gap: 4px; margin-left: -8px; }
</style>
