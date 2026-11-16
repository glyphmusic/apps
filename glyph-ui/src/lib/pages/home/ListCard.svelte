<script lang="ts">
  import ListCover from "../../components/ListCover.svelte";
  import { app } from "../../state.svelte";
  import { t, tf } from "../../i18n.svelte";
  import type { ListKey } from "../../api/types";

  let { list }: { list: ListKey } = $props();
  const title = $derived(app.listTitle(list));
  const tracks = $derived(app.listTracks(list));
  const sub = $derived.by(() => {
    switch (list.kind) {
      case "liked":
      case "downloads":
        return tf("common.n_songs", { n: tracks.length });
      case "playlist":
        return `${t("list.kind.playlist")} · ${tf("common.n_songs", { n: tracks.length })}`;
      case "album":
        return list.artist ? `${t("list.kind.album")} · ${list.artist}` : t("list.kind.album");
      case "artist_liked":
        return `${t("library.liked")} · ${list.artist}`;
      default:
        return "";
    }
  });

  function open() {
    if (list.kind === "album" && !list.artist.startsWith("@")) {
      app.openAlbum(list.title, list.artist, list.tracks ?? [], list.cover ?? tracks[0] ?? null);
    } else {
      app.openListView(list);
    }
  }
  function menu(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    app.openCtx({ x: e.clientX, y: e.clientY, track: null, list });
  }
</script>

<div class="card" onclick={open} oncontextmenu={menu} onkeydown={(e) => e.key === "Enter" && open()} role="button" tabindex="-1">
  <ListCover {list} size={176} />
  <div class="t trunc">{title}</div>
  {#if sub}<div class="a trunc">{sub}</div>{/if}
</div>

<style>
  .card { flex: none; width: 192px; padding: 8px; border-radius: 10px; display: flex; flex-direction: column; gap: 4px; cursor: pointer; transition: background-color .24s ease-out; }
  .card:hover { background: rgba(255, 255, 255, .05); transition-duration: .12s; }
  .t { padding-top: 4px; font-size: var(--md); font-weight: 500; }
  .a { font-size: var(--sm); color: var(--dim); }
</style>
