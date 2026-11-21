<script lang="ts">
  // Multi-select action bar for ListView (queue, playlist, download, remove).
  import Button from "../../components/Button.svelte";
  import IconButton from "../../components/IconButton.svelte";
  import Select from "../../settings/Select.svelte";
  import { app } from "../../state.svelte";
  import { onWeb } from "../../web";
  import { t, tf } from "../../i18n.svelte";
  import type { ListKey, Track } from "../../api/types";

  let {
    list,
    tracks,
    canRemove = false,
  }: {
    list: ListKey;
    tracks: Track[];
    canRemove?: boolean;
  } = $props();

  const pickedTracks = $derived(app.pickedTracksOf(tracks));
  const pickedN = $derived(pickedTracks.length);
  const allPicked = $derived(tracks.length > 0 && pickedN === tracks.length);

  const playlistOptions = $derived.by(() => {
    const opts: { value?: string; label?: string; icon?: string; sep?: boolean }[] = [
      { value: "__new__", label: t("library.playlist.new"), icon: "plus" },
    ];
    const others = app.library.playlists.filter((p) => !(list.kind === "playlist" && list.id === p.id));
    if (others.length) {
      opts.push({ sep: true });
      for (const p of others) opts.push({ value: p.id, label: p.name, icon: "list-music" });
    }
    return opts;
  });

  function addPickedToQueue() {
    if (!pickedN) return;
    app.queueAdd(pickedTracks, false);
    app.stopPicking();
  }

  function addPickedToPlaylist(id: string | null) {
    if (!id || !pickedN) return;
    if (id === "__new__") {
      app.nameDialog = { tracks: [...pickedTracks], rename: null, initial: "" };
      app.stopPicking();
      return;
    }
    app.addToPlaylist(id, pickedTracks);
    app.stopPicking();
  }

  function downloadPicked() {
    if (!pickedN) return;
    app.download(pickedTracks);
    app.stopPicking();
  }
</script>

<div class="pickbar" role="toolbar" aria-label={t("pick.select")}>
  <Button icon="x" onclick={() => app.stopPicking()}>{t("pick.done")}</Button>
  <span class="pick-n">{tf("pick.n", { n: pickedN })}</span>
  <Button kind="text" onclick={() => app.pickAll(tracks)}>{t(allPicked ? "pick.none" : "pick.all")}</Button>
  <span class="flex1"></span>
  <div class="pacts">
    <IconButton name="list-plus" size={16} box={32} disabled={!pickedN} label={t("pick.queue")} onclick={addPickedToQueue} />
    <span class="pl-sel" class:off={!pickedN}>
      <Select value={null as string | null} label={t("pick.playlist")} options={playlistOptions} onpick={addPickedToPlaylist} />
    </span>
    {#if !onWeb() && list.kind !== "downloads"}
      <IconButton name="arrow-down-to-line" size={16} box={32} disabled={!pickedN} label={t("pick.download")} onclick={downloadPicked} />
    {/if}
    {#if list.kind === "liked"}
      <Button kind="text" icon="trash-2" onclick={() => app.clearLiked()}>{t("liked.clear")}</Button>
    {/if}
    {#if canRemove}
      <Button kind="danger" icon="trash-2" disabled={!pickedN} onclick={() => app.removePicked(list)}>{t("pick.remove")}</Button>
    {/if}
  </div>
</div>

<style>
  .pickbar {
    position: sticky;
    top: 0;
    z-index: 4;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    margin: 10px 0 0;
    padding: 8px 10px;
    border-radius: 12px;
    background: #161618;
  }
  .pick-n { font-size: var(--md); font-weight: 600; color: var(--text); padding: 0 4px; }
  .pacts { display: flex; align-items: center; flex-wrap: wrap; gap: 6px; }
  .pl-sel.off { opacity: .4; pointer-events: none; }
</style>
