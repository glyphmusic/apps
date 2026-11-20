<script lang="ts">
  // views.rs new_releases_row: release card (cover 176 radius 8, title md
  // medium, artist link, mono 11 faint "SINGLE · 2026-10-12"); click opens
  // the release's song list, right click = album menu.
  import Cover from "../../components/Cover.svelte";
  import { app } from "../../state.svelte";
  import { t } from "../../i18n.svelte";
  import { primaryArtist, type Release, type Track } from "../../api/types";
  let { release }: { release: Release } = $props();
  /** `Release::cover_track`: a stand-in song that carries the cover. */
  const seed = $derived<Track>({ id: release.id, source: "deezer", title: release.title, artist: release.artist, album: release.title, cover_url: release.cover_url ?? undefined });
  const label = $derived.by(() => {
    const kind = t(`album.kind.${release.kind}`);
    return (release.date ? `${kind} · ${release.date}` : kind).toUpperCase();
  });
  const open = () => app.openAlbum(release.title, release.artist, [], seed, { kind: release.kind, tracks: release.tracks });
  function menu(e: MouseEvent) {
    e.preventDefault(); e.stopPropagation();
    app.openCtx({ x: e.clientX, y: e.clientY, track: null, list: { kind: "album", title: release.title, artist: release.artist, tracks: [seed] } });
  }
  function artistMenu(e: MouseEvent) { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: null, artist: primaryArtist(release.artist) }); }
</script>

<div class="card" onclick={open} oncontextmenu={menu} onkeydown={(e) => e.key === "Enter" && open()} role="button" tabindex="-1">
  <Cover track={seed} size={176} radius={8} />
  <div class="t trunc">{release.title}</div>
  <div class="a">
    <button class="link trunc" onclick={(e) => { e.stopPropagation(); app.openArtistPage(release.artist); }} oncontextmenu={artistMenu}>{release.artist}</button>
  </div>
  <div class="k mono">{label}</div>
</div>

<style>
  .card { flex: none; width: 192px; padding: 8px; border-radius: 10px; display: flex; flex-direction: column; gap: 4px; cursor: pointer; transition: background-color .24s ease-out; }
  .card:hover { background: rgba(255, 255, 255, .05); transition-duration: .12s; }
  .t { padding-top: 4px; font-size: var(--md); font-weight: 500; color: var(--text); }
  .a { display: flex; min-width: 0; font-size: var(--sm); color: var(--dim); }
  .link { min-width: 0; max-width: 100%; }
  .link:hover { color: var(--text); text-decoration: underline; }
  .k { font-size: 11px; color: var(--faint); }
</style>
