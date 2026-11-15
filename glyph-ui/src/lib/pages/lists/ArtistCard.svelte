<script lang="ts">
  // lists/artist.rs similar_artist_card (also the library's followed-artist
  // tiles): w 184, p 8, radius 10, gap 4, hover white 5 %; round 168 px
  // picture (a known song's cover, else a card with user-round 48 white
  // 50 %), name md medium + verified badge (pt 6), "Artist" sm dim.
  // Right click opens the artist menu.
  import Icon from "../../components/Icon.svelte";
  import Cover from "../../components/Cover.svelte";
  import ArtistName from "./ArtistName.svelte";
  import { app } from "../../state.svelte";
  import { t } from "../../i18n.svelte";
  import { artistTrack } from "./actions";
  import { primaryArtist } from "../../api/types";
  let { name }: { name: string } = $props();
  const photo = $derived(app.artistPhoto(name));
  const tr = $derived(photo ? null : artistTrack(name));
  function menu(e: MouseEvent) {
    e.preventDefault(); e.stopPropagation();
    app.openCtx({ x: e.clientX, y: e.clientY, track: null, artist: primaryArtist(name) });
  }
</script>

<button class="ac" onclick={() => app.openArtistPage(name)} oncontextmenu={menu}>
  {#if photo}<img class="ph" src={photo} alt="" />
  {:else if tr}<Cover track={tr} size={168} round />
  {:else}<span class="none"><Icon name="user-round" size={48} color="rgba(255,255,255,.5)" /></span>{/if}
  <span class="nm"><ArtistName {name} /></span>
  <span class="kd">{t("search.kind.artist")}</span>
</button>

<style>
  .ac { flex: none; width: 184px; padding: 8px; border-radius: 10px; display: flex; flex-direction: column; gap: 4px; transition: background-color .12s ease-out; }
  .ac:hover { background: rgba(255, 255, 255, .05); }
  .ph, .none { width: 168px; height: 168px; border-radius: 50%; flex: none; }
  .ph { object-fit: cover; }
  .none { display: grid; place-items: center; background: var(--card); }
  .nm { padding-top: 6px; font-size: var(--md); font-weight: 500; color: var(--text); min-width: 0; }
  .kd { font-size: var(--sm); color: var(--dim); }
</style>
