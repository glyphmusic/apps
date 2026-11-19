<script lang="ts">
  // lists/rows.rs recent_artist_row: a "recently played" row that is an
  // artist. h 56, px 10, gap 14, radius 8, hover white 5 %; user-round 14
  // faint in the 26 px number column, round 40 picture (card + user-round 18
  // white 50 % without one), name md medium + verified badge, "Artist" sm
  // dim. Click opens the artist; right click the artist menu.
  import Icon from "../../components/Icon.svelte";
  import ArtistAvatar from "../../components/ArtistAvatar.svelte";
  import ArtistName from "./ArtistName.svelte";
  import { app } from "../../state.svelte";
  import { t } from "../../i18n.svelte";
  import { primaryArtist, type Track } from "../../api/types";
  let { name, track = null }: { name: string; track?: Track | null } = $props();
  function menu(e: MouseEvent) {
    e.preventDefault(); e.stopPropagation();
    app.openCtx({ x: e.clientX, y: e.clientY, track: null, artist: primaryArtist(name) });
  }
</script>

<div class="ar" onclick={() => app.openArtistPage(name)} oncontextmenu={menu} onkeydown={(e) => e.key === "Enter" && app.openArtistPage(name)} role="row" tabindex="-1">
  <span class="n"><Icon name="user-round" size={14} color="var(--faint)" /></span>
  <ArtistAvatar {name} {track} size={40} iconSize={18} />
  <span class="tx">
    <span class="nm"><ArtistName {name} /></span>
    <span class="kd">{t("search.kind.artist")}</span>
  </span>
</div>

<style>
  .ar { display: flex; align-items: center; gap: 14px; height: 56px; padding: 0 10px; border-radius: 8px; cursor: pointer; transition: background-color .12s ease-out; }
  .ar:hover { background: rgba(255, 255, 255, .05); }
  .n { flex: none; width: 26px; display: grid; place-items: center; }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .nm { font-size: var(--md); font-weight: 500; color: var(--text); min-width: 0; }
  .kd { font-size: var(--sm); color: var(--dim); }
</style>
