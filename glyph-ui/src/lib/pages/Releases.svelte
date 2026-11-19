<script lang="ts">
  // lists/artist.rs releases_view: "Show all" of an artist's releases.
  // Column gap 16: back pill, heading "Name · Albums", then every release
  // card (168) wrapping with gap 4 / ml -8; skeleton artist rows while the
  // artist loads, else the empty state.
  import SectionTitle from "../components/SectionTitle.svelte";
  import EmptyState from "../components/EmptyState.svelte";
  import BackPill from "./lists/BackPill.svelte";
  import ReleaseCard from "./lists/ReleaseCard.svelte";
  import SkeletonArtistRows from "./lists/SkeletonArtistRows.svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { groupAlbums, releaseKind } from "./lists/actions";
  import { albumLabel } from "./lists/albumMeta.svelte";
  import type { Release, Track } from "../api/types";
  import { uniqBy } from "../fmt";

  const name = $derived(app.openArtist ?? "");
  const page = $derived(app.artists[name.toLowerCase()]);
  const tracks = $derived(page?.tracks ?? []);
  const loading = $derived((!page || page.loading) && !tracks.length);
  const releases = $derived(uniqBy(page?.releases ?? [], (r) => r.id));
  const grouped = $derived(releases.length ? [] : groupAlbums(tracks));
  const seed = (r: Release): Track => ({ id: r.id, source: "deezer", title: r.title, artist: r.artist, album: r.title, cover_url: r.cover_url ?? undefined });
</script>

<div class="rv">
  <BackPill />
  <SectionTitle heading title="{name} · {t('artist.albums')}" />
  {#if releases.length}
    <div class="grid">
      {#each releases as r (r.id)}
        <ReleaseCard size={168} title={r.title} artist={r.artist} tracks={[seed(r)]} label={r.year ? `${r.year} · ${releaseKind(r.kind)}` : releaseKind(r.kind)} onopen={() => app.openAlbum(r.title, r.artist, [], seed(r))} />
      {/each}
    </div>
  {:else if grouped.length}
    <div class="grid">
      {#each grouped as g (g.title)}
        <ReleaseCard size={168} title={g.title} artist={name} tracks={g.tracks} label={albumLabel(g.title, name, g.tracks.length)} onopen={() => app.openListView({ kind: "album", title: g.title, artist: name, tracks: g.tracks })} />
      {/each}
    </div>
  {:else if loading}
    <SkeletonArtistRows prefix="releases" n={2} />
  {:else}
    <EmptyState title={t("artist.empty")} sub={t("artist.empty.sub")} />
  {/if}
</div>

<style>
  .rv { display: flex; flex-direction: column; gap: 16px; padding-bottom: 20px; }
  .grid { display: flex; flex-wrap: wrap; gap: 4px; margin-left: -8px; }
</style>
