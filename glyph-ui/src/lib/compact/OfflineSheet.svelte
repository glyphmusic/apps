<script lang="ts">
  // Phone: comes up once when the connection is gone for a while. Lists what
  // still plays: Downloads and every playlist with saved songs. Tap a row to
  // play its saved songs, the arrow to open it. Pull down or tap outside to close.
  import { fade } from "svelte/transition";
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import ListCover from "../components/ListCover.svelte";
  import { app } from "../state.svelte";
  import { t, tf } from "../i18n.svelte";
  import { offline, savedLists, type SavedList } from "../offline.svelte";
  import { drag, sheetIn, sheetOut, shouldClose } from "./cstate.svelte";

  let pull = $state(0);
  let listEl: HTMLDivElement | undefined = $state();
  const lists = $derived(savedLists());
  const close = () => (offline.sheetOpen = false);

  function play(l: SavedList) { void app.play(l.tracks, 0, l.list.kind === "downloads" ? "downloads" : `playlist:${(l.list as { id: string }).id}`); close(); }
  function open(l: SavedList) { app.openListView(l.list); close(); }
  const sub = (l: SavedList) => l.list.kind === "downloads"
    ? tf("common.n_songs", { n: l.total })
    : tf("playlist.offline_progress", { done: l.tracks.length, total: l.total });
</script>

<div class="dim" transition:fade={{ duration: 180 }} onclick={close} onkeydown={() => {}} role="presentation"></div>

<div class="sheet" style:transform="translateY({pull}px)" class:settle={pull === 0}
  in:sheetIn|global={{ duration: 360 }} out:sheetOut|global={{ duration: 220, radius: true }}>
  <div class="grab-area" use:drag={{ axis: "y", move: (_, dy) => (pull = Math.max(0, dy)), end: (_, dy, _vx, vy) => { if (shouldClose(dy, vy)) { close(); return; } pull = 0; } }}>
    <span class="grab"></span>
    <div class="head">
      <span class="ic"><Icon name="wifi-off" size={18} color="var(--text)" /></span>
      <div class="ht">
        <span class="t1">{t("offline.bar")}</span>
        <span class="t2">{t("offline.sheet_sub")}</span>
      </div>
      <IconButton name="x" size={18} box={34} notip onclick={close} />
    </div>
  </div>

  <div class="list scroll" bind:this={listEl}
    use:drag={{ axis: "y", onlyAtTop: () => (listEl?.scrollTop ?? 0) <= 0, move: (_, dy) => (pull = Math.max(0, dy)), end: (_, dy, _vx, vy) => { if (shouldClose(dy, vy)) { close(); return; } pull = 0; } }}>
    {#each lists as l (l.list.kind === "downloads" ? "dl" : (l.list as { id: string }).id)}
      <div class="row">
        <button class="main" onclick={() => play(l)}>
          <ListCover list={l.list} size={52} />
          <span class="ri">
            <span class="rt trunc">{l.list.kind === "downloads" ? t("library.downloads") : l.name}</span>
            <span class="rs trunc"><Icon name="arrow-down-to-line" size={11} color="var(--accent)" />{sub(l)}</span>
          </span>
          <span class="pb"><Icon name="play" size={13} color="rgba(10,10,10,1)" /></span>
        </button>
        <IconButton name="chevron-right" size={16} box={34} notip onclick={() => open(l)} />
      </div>
    {/each}
  </div>
</div>

<style>
  .dim { position: fixed; inset: 0; z-index: 64; background: rgba(0, 0, 0, .62); }
  .sheet { position: fixed; left: 0; right: 0; bottom: 0; z-index: 65; max-height: 70vh; border-radius: 16px 16px 0 0; background: #121215; display: flex; flex-direction: column; overflow: hidden; padding-bottom: var(--safe-bottom, env(safe-area-inset-bottom)); }
  .sheet.settle { transition: transform .22s var(--ease); }
  .grab-area { flex: none; padding: 10px 16px 10px; display: flex; flex-direction: column; align-items: center; touch-action: none; }
  .grab { width: 36px; height: 4px; border-radius: 2px; background: rgba(255, 255, 255, .2); margin-bottom: 12px; }
  .head { width: 100%; display: flex; align-items: center; gap: 12px; }
  .ic { flex: none; width: 38px; height: 38px; border-radius: 50%; background: #1f1f25; display: grid; place-items: center; }
  .ht { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .t1 { font-size: 16px; font-weight: 700; color: var(--text); }
  .t2 { font-size: 12.5px; color: var(--dim); }
  .head :global(.ib) { margin-right: -6px; }
  .list { flex: 1; min-height: 0; overflow-y: auto; padding: 4px 8px 16px; display: flex; flex-direction: column; gap: 2px; }
  .row { display: flex; align-items: center; gap: 2px; }
  .main { flex: 1; min-width: 0; display: flex; align-items: center; gap: 12px; padding: 7px 8px; border-radius: 10px; text-align: left; }
  .main:active { background: #1e1e24; }
  .ri { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .rt { font-size: 14.5px; font-weight: 600; color: var(--text); }
  .rs { font-size: 12.5px; color: var(--dim); display: flex; align-items: center; gap: 5px; }
  .pb { flex: none; width: 32px; height: 32px; border-radius: 50%; background: var(--accent); display: grid; place-items: center; padding-left: 2px; }
</style>
