<script lang="ts">
  import Icon from "./Icon.svelte";
  import Cover from "./Cover.svelte";
  import { app } from "../state.svelte";
  import { artistTrack } from "../pages/lists/actions";

  let { name, size = 32, iconSize = Math.round(size * 0.45) }: {
    name: string;
    size?: number;
    iconSize?: number;
  } = $props();

  const photo = $derived(app.artistPhoto(name));
  const tr = $derived(photo ? null : artistTrack(name));
</script>

{#if photo}
  <img class="av" src={photo} alt="" style:width="{size}px" style:height="{size}px" />
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
