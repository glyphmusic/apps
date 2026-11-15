<script lang="ts">
  // views.rs cover_row_with / recent_row (song card): flex none, w 192,
  // p 8, radius 10, gap 4, hover white 5 %; cover 176 radius 8, title md
  // medium (pt 4, E badge), artist sm dim (link). Hidden explicit songs are
  // dimmed (opacity .4) and do not play. Right click = song menu.
  import Cover from "../../components/Cover.svelte";
  import SongTitle from "../../components/SongTitle.svelte";
  import { app } from "../../state.svelte";
  import { primaryArtist, rating, type ListKey, type Track } from "../../api/types";
  let { track, onplay, from = null }: { track: Track; onplay: () => void; from?: { list: ListKey; index: number } | null } = $props();
  const blocked = $derived(!app.allowed(track));
  function menu(e: MouseEvent) { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track, from }); }
  function artistMenu(e: MouseEvent) { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: null, artist: primaryArtist(track.artist) }); }
</script>

<div class="card" class:off={blocked} class:menu={app.ctx?.track === track} onclick={onplay} oncontextmenu={menu} onkeydown={(e) => e.key === "Enter" && onplay()} role="button" tabindex="-1">
  <Cover {track} size={176} radius={8} />
  <div class="t"><SongTitle rating={rating(track)}>{track.title}</SongTitle></div>
  <div class="a">
    <button class="link trunc" onclick={(e) => { e.stopPropagation(); app.openArtistPage(track.artist); }} oncontextmenu={artistMenu}>{track.artist}</button>
  </div>
</div>

<style>
  .card { flex: none; width: 192px; padding: 8px; border-radius: 10px; display: flex; flex-direction: column; gap: 4px; cursor: pointer; transition: background-color .24s ease-out; }
  .card:hover, .card.menu { background: rgba(255, 255, 255, .05); transition-duration: .12s; }
  .card.off { opacity: .4; }
  .t { padding-top: 4px; min-width: 0; font-size: var(--md); font-weight: 500; }
  .a { display: flex; min-width: 0; font-size: var(--sm); color: var(--dim); }
  .link { min-width: 0; max-width: 100%; }
  .link:hover { color: var(--text); text-decoration: underline; }
</style>
