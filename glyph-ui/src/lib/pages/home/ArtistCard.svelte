<script lang="ts">
  // views.rs recent_row (RecentItem::Artist): same card as a song, but the
  // picture is round (radius 88 of 176; no picture: card colour with a
  // user-round icon 56, white 55 %), name md medium, "Artist" sm dim.
  import Cover from "../../components/Cover.svelte";
  import Icon from "../../components/Icon.svelte";
  import { app } from "../../state.svelte";
  import { t } from "../../i18n.svelte";
  import type { Track } from "../../api/types";
  let { name, track = null }: { name: string; track?: Track | null } = $props();
  function menu(e: MouseEvent) { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: null, artist: name }); }
</script>

<div class="card" onclick={() => app.openArtistPage(name)} oncontextmenu={menu} onkeydown={(e) => e.key === "Enter" && app.openArtistPage(name)} role="button" tabindex="-1">
  {#if track}
    <Cover {track} size={176} radius={88} />
  {:else}
    <div class="ph"><Icon name="user-round" size={56} color="rgba(255,255,255,.55)" /></div>
  {/if}
  <div class="t trunc">{name}</div>
  <div class="a">{t("search.kind.artist")}</div>
</div>

<style>
  .card { flex: none; width: 192px; padding: 8px; border-radius: 10px; display: flex; flex-direction: column; gap: 4px; cursor: pointer; transition: background-color .24s ease-out; }
  .card:hover { background: rgba(255, 255, 255, .05); transition-duration: .12s; }
  .ph { flex: none; width: 176px; height: 176px; border-radius: 50%; background: var(--card); display: grid; place-items: center; }
  .t { padding-top: 4px; font-size: var(--md); font-weight: 500; }
  .a { font-size: var(--sm); color: var(--dim); }
</style>
