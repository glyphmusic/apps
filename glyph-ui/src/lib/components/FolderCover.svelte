<script lang="ts">
  // Library folder picture: a card (radius like ListCover) holding one
  // picture per playlist (its own picture, else its first song's cover) for
  // the first four, in a 2x2 grid; empty cells stay faint. An empty folder
  // shows the folder icon. `open`: drop target hover.
  import Icon from "./Icon.svelte";
  import Cover from "./Cover.svelte";
  import { app } from "../state.svelte";
  import { cornerPx } from "../corners";
  let { id, size = 36, open = false }: { id: string; size?: number; open?: boolean } = $props();
  const r = $derived(Math.min(12, Math.max(6, size * 0.06)));
  const lists = $derived(app.folderPlaylists(id).slice(0, 4));
  const pad = $derived(Math.max(4, Math.round(size * 0.07)));
  const gap = $derived(Math.max(2, Math.round(size * 0.04)));
  const cell = $derived(Math.floor((size - pad * 2 - gap) / 2));
  const cr = $derived(Math.min(8, Math.max(2, cell * 0.08)));
  const isz = $derived(Math.min(72, Math.max(16, size * 0.3)));
</script>

<div class="fc" class:open style:width="{size}px" style:height="{size}px" style:border-radius={cornerPx(r)} style:--pad="{pad}px" style:--gap="{gap}px">
  {#if lists.length}
    <div class="grid">
      {#each [0, 1, 2, 3] as i (i)}
        {@const p = lists[i]}
        {#if p?.cover}
          <img src={p.cover} alt="" decoding="async" style:width="{cell}px" style:height="{cell}px" style:border-radius={cornerPx(cr)} />
        {:else if p?.tracks[0]}
          <Cover track={p.tracks[0]} size={cell} radius={cr} />
        {:else}
          <span class="empty" style:width="{cell}px" style:height="{cell}px" style:border-radius={cornerPx(cr)}>
            {#if p && cell >= 32}<Icon name="list-music" size={Math.round(cell * 0.36)} color="rgba(255,255,255,.4)" />{/if}
          </span>
        {/if}
      {/each}
    </div>
  {:else}
    <Icon name={open ? "folder-open" : "folder"} size={isz} color="rgba(255,255,255,.5)" />
  {/if}
</div>

<style>
  .fc { flex: none; display: grid; place-items: center; background: var(--card); transition: background-color .14s ease-out, transform .16s var(--ease); }
  .fc.open { background: color-mix(in srgb, var(--accent) 22%, var(--card)); transform: scale(1.03); }
  .grid { display: grid; grid-template-columns: auto auto; gap: var(--gap); padding: var(--pad); }
  .grid img { display: block; object-fit: cover; }
  .empty { display: grid; place-items: center; background: rgba(255, 255, 255, .04); }
</style>
