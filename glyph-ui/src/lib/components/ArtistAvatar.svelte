<script lang="ts">
  // An artist's round picture: the photo (looked up in the background the
  // first time), else the cover of `track` (e.g. the song played from the
  // artist) or a known song of theirs, else a user-round icon.
  import Icon from "./Icon.svelte";
  import Cover from "./Cover.svelte";
  import { app } from "../state.svelte";
  import { artistTrack } from "../pages/lists/actions";
  import type { Track } from "../api/types";

  let { name, size = 32, iconSize = Math.round(size * 0.45), track = null }: {
    name: string;
    size?: number;
    iconSize?: number;
    track?: Track | null;
  } = $props();

  const photo = $derived(app.artistPhoto(name));
  const tr = $derived(photo ? null : (track ?? artistTrack(name)));
</script>

{#if photo}
  <img class="av" src={photo} alt="" draggable="false" decoding="async" style:width="{size}px" style:height="{size}px" onerror={() => app.dropArtistPhoto(name)} />
{:else if tr}
  <Cover track={tr} {size} round />
{:else}
  <span class="none" style:width="{size}px" style:height="{size}px">
    <Icon name="user-round" size={iconSize} color="rgba(255,255,255,.55)" />
  </span>
{/if}

<style>
  .av, .none { border-radius: 50%; flex: none; }
  .av { object-fit: cover; display: block; }
  .none { display: grid; place-items: center; background: var(--card); }
</style>
