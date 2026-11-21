<script lang="ts">
  // Songs or a playlist shared or opened with Glyph (Android share sheet /
  // "Open with", desktop "Send to" or a copied link): where they go. Song
  // (or playlist) header, "Save as playlist" for a shared playlist, Liked
  // Songs and every playlist as check rows (several at once), "New
  // playlist"; Play and Done below. Nothing is added until Done; closing
  // (dim, drag down, Escape) adds nothing.
  // Phone: a bottom sheet like the song menu (ContextMenu.svelte bs-*).
  // Desktop: a dialog like NameDialog (w 380, p 20, radius 12).
  // On Android it is also the whole page of the share sheet over another
  // app (share-sheet.svelte.ts): it opens at once while the link is read
  // (`loading`), and Done or closing leaves that app as it was.
  import { fade, scale } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Icon from "../components/Icon.svelte";
  import Cover from "../components/Cover.svelte";
  import ListCover from "../components/ListCover.svelte";
  import Button from "../components/Button.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import ErrorCard from "../components/ErrorCard.svelte";
  import { app, type PickSongs } from "../state.svelte";
  import { t, tf } from "../i18n.svelte";
  import { fadeEdges } from "../ui.svelte";
  import { trackKey, type Track } from "../api/types";
  import { backend } from "../api/backend";
  import { drag, sheetIn, sheetOut, shouldClose } from "../compact/cstate.svelte";
  import { createDraftPlaylists } from "../draft-playlists.svelte";
  import { closeShareSheet, retryShare, shareProgress } from "../share-sheet.svelte";

  const empty: PickSongs = { tracks: [] };
  // Kept while the sheet slides out (app.pickSongs is null by then).
  let data = $state<PickSongs>(($state.snapshot(app.pickSongs) as PickSongs | null) ?? empty);
  $effect(() => {
    const p = app.pickSongs;
    if (p) data = $state.snapshot(p) as PickSongs;
  });
  const tracks: Track[] = $derived(data.tracks);
  const first = $derived(tracks[0]);
  const keys = $derived(new Set(tracks.map(trackKey)));
  const list = $derived(data.list ?? null);
  const ready = $derived(!data.loading && !data.error && tracks.length > 0);
  const compact = $derived(app.compact || !!data.sheet);

  const allLiked = $derived(ready && tracks.every((x) => app.isLiked(x)));
  // Liked Songs is ticked at the start for songs (what sharing a song did
  // before); a shared playlist is saved as a playlist instead.
  let likeSet = $state<boolean | null>(null);
  const like = $derived(likeSet ?? !list);
  let saveList = $state(true);
  let picked = $state<string[]>([]);
  const drafts = createDraftPlaylists();
  let pull = $state(0);
  let busy = $state(false);

  /** Playlists that already hold every one of the songs. */
  function holdsAll(id: string): boolean {
    const p = app.playlist(id);
    if (!p) return false;
    const have = new Set(p.tracks.map(trackKey));
    return [...keys].every((k) => have.has(k));
  }
  const lists = $derived(app.library.playlists);

  function toggle(id: string) {
    picked = picked.includes(id) ? picked.filter((x) => x !== id) : [...picked, id];
  }
  function close() {
    drafts.clear();
    app.pickSongs = null;
    if (data.sheet) closeShareSheet();
  }
  /** Adds the songs where they were ticked; returns how many places. */
  async function apply(): Promise<number> {
    let n = 0;
    const wait: Promise<unknown>[] = [];
    if (like && !allLiked) {
      const add = tracks.filter((x) => !app.isLiked(x));
      // The share sheet closes right after: one call now, not a batch later.
      if (data.sheet) wait.push(app.setLiked(add, true));
      else for (const x of add) void app.toggleLike(x);
      n++;
    }
    if (list && saveList) {
      wait.push(list.code
        ? backend().call("share_save", { code: list.code, link: list.link ?? null, author: list.author ?? null })
        : backend().call("share_save_list", { name: list.name, origin: list.origin ?? null, tracks }));
      n++;
    }
    for (const d of drafts.items) {
      wait.push(app.createPlaylist(d.name, tracks));
      n++;
    }
    for (const id of picked) {
      const p = app.playlist(id);
      if (!p) continue;
      const have = new Set(p.tracks.map(trackKey));
      const add = tracks.filter((x) => !have.has(trackKey(x)));
      if (add.length) { wait.push(app.addToPlaylist(id, add, true)); n++; }
    }
    if (data.sheet) await Promise.allSettled(wait);
    return n;
  }
  async function done() {
    if (!ready || busy) return;
    busy = true;
    const liked = like && !allLiked;
    const createdNames = drafts.items.map((d) => d.name);
    const n = await apply();
    drafts.clear();
    const saved = list && saveList;
    app.pickSongs = null;
    if (data.sheet) {
      // Said by Android once the sheet is gone (the app below stays as it was).
      const msg = saved ? tf("share_in.saved_list", { name: list!.name }) : n ? t("share_in.saved") : "";
      closeShareSheet(msg);
      return;
    }
    // One toast for all of it (a saved list says so itself).
    if (saved && n === 1) return;
    if (liked && n === 1) app.toast(tracks.length === 1 ? t("import.song.added") : tf("share_in.liked_n", { n: tracks.length }));
    else if (n === 1 && createdNames.length === 1) app.toast(tracks.length === 1 ? tf("toast.added_to_playlist", { name: createdNames[0] }) : tf("toast.added_n_to_playlist", { n: tracks.length, name: createdNames[0] }));
    else if (n === 1 && picked.length === 1) app.toast(tf("toast.added_to_playlist", { name: app.playlist(picked[0])?.name ?? "" }));
    else if (n > 1) app.toast(t("share_in.saved"));
  }
  function newPlaylist() {
    if (!ready) return;
    drafts.promptCreate({ onCreated: () => {} });
  }
  function play() {
    if (ready) app.play(tracks, 0);
  }
  function key(e: KeyboardEvent) {
    if (app.nameDialog) return;
    if (e.key === "Escape") { e.preventDefault(); close(); }
  }
</script>

<svelte:window onkeydown={key} />

{#snippet head()}
  <div class="hd">
    {#if data.loading}
      <Skeleton w={compact ? 48 : 44} h={compact ? 48 : 44} radius={6} id="share-head" />
      <div class="ht">
        <span class="t1 trunc">{data.files ? t("share_in.reading_files") : t("share_in.reading")}</span>
        <span class="t2 trunc">{shareProgress.total ? tf("share_in.found", { done: shareProgress.done, total: shareProgress.total }) : data.label ?? ""}</span>
      </div>
    {:else if list}
      <ListCover list={{ kind: "album", title: list.name, artist: "", tracks }} size={compact ? 48 : 44} />
      <div class="ht">
        <span class="t1 trunc">{list.name}</span>
        <span class="t2 trunc">{list.author ? `${list.author} · ` : ""}{tf("common.n_songs", { n: tracks.length })}</span>
      </div>
    {:else if first}
      <Cover track={first} size={compact ? 48 : 44} radius={6} />
      <div class="ht">
        <span class="t1 trunc">{tracks.length === 1 ? first?.title : tf("common.n_songs", { n: tracks.length })}</span>
        <span class="t2 trunc">{tracks.length === 1 ? first?.artist : tracks.slice(0, 3).map((x) => x.title).join(", ")}</span>
      </div>
    {/if}
    {#if ready}
      <button class="play" onclick={play} aria-label={t("common.play")}>
        <Icon name="play" size={16} color="#000" />
      </button>
    {/if}
  </div>
{/snippet}

{#snippet rows()}
  {#if data.error}
    <div class="err"><ErrorCard compact title={t("share_in.failed")} body={data.error} onretry={data.sheet ? retryShare : undefined} /></div>
  {:else if data.loading}
    {#each [0, 1, 2, 3] as i (i)}
      <div class="row sk-row"><Skeleton w={compact ? 40 : 32} h={compact ? 40 : 32} radius={4} id="share-row-{i}" /><Skeleton w="{46 - i * 7}%" h={12} radius={4} id="share-txt-{i}" /></div>
    {/each}
  {:else}
    {#if list}
      <button class="row" onclick={() => (saveList = !saveList)}>
        <span class="nw"><Icon name="list-plus" size={compact ? 18 : 15} color="rgba(255,255,255,.85)" /></span>
        <span class="tx">
          <span class="nm trunc">{t("share_in.save_list")}</span>
          <span class="ct trunc">{list.name}</span>
        </span>
        <span class="chk" class:on={saveList}>{#if saveList}<Icon name="check" size={13} color="#000" />{/if}</span>
      </button>
    {/if}
    <button class="row" onclick={() => { if (!allLiked) likeSet = !like; }} disabled={allLiked}>
      <span class="lk"><Icon name="heart-fill" size={compact ? 18 : 15} color="#fff" /></span>
      <span class="tx">
        <span class="nm trunc">{t("library.liked")}</span>
        {#if allLiked}<span class="ct trunc">{t("share_in.already")}</span>{/if}
      </span>
      <span class="chk" class:on={allLiked || like}>{#if allLiked || like}<Icon name="check" size={13} color="#000" />{/if}</span>
    </button>
    <div class="sep"></div>
    <button class="row" onclick={newPlaylist}>
      <span class="nw"><Icon name="plus" size={compact ? 18 : 15} color="rgba(255,255,255,.85)" /></span>
      <span class="tx"><span class="nm trunc">{t("library.playlist.new")}</span></span>
    </button>
    {#each drafts.items as d (d.id)}
      <button class="row" onclick={() => drafts.remove(d.id)}>
        <ListCover list={{ kind: "playlist", id: d.id }} size={compact ? 40 : 32} />
        <span class="tx">
          <span class="nm trunc">{d.name}</span>
          <span class="ct trunc">{tf("common.n_songs", { n: 0 })}</span>
        </span>
        <span class="chk on"><Icon name="check" size={13} color="#000" /></span>
      </button>
    {/each}
    {#each lists as p (p.id)}
      {@const has = holdsAll(p.id)}
      {@const on = has || picked.includes(p.id)}
      <button class="row" onclick={() => toggle(p.id)} disabled={has}>
        <ListCover list={{ kind: "playlist", id: p.id }} size={compact ? 40 : 32} />
        <span class="tx">
          <span class="nm trunc">{p.name}</span>
          <span class="ct trunc">{has ? t("share_in.already") : tf("common.n_songs", { n: p.tracks.length })}</span>
        </span>
        <span class="chk" class:on>{#if on}<Icon name="check" size={13} color="#000" />{/if}</span>
      </button>
    {/each}
  {/if}
{/snippet}

{#if compact}
  <div class="bs-dim" class:sheet={data.sheet} transition:fade|global={{ duration: 180 }} onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")} onpointerdown={close} role="presentation">
    <div class="bs-panel" style:transform="translateY({pull}px)" class:settle={pull === 0} in:sheetIn|global={{ duration: 260 }} out:sheetOut|global={{ duration: 210 }} onpointerdown={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
      <div class="bs-grab" use:drag={{ axis: "y", move: (_, dy) => (pull = Math.max(0, dy)), end: (_, dy, _vx, vy) => { if (shouldClose(dy, vy)) { close(); return; } pull = 0; } }}>
        <span class="hdl"></span>
        <div class="title">{t("share_in.title")}</div>
        {@render head()}
      </div>
      <div class="bs-list scroll" use:fadeEdges={{ top: 12, bottom: 12, minOverflow: 8 }}>{@render rows()}</div>
      <div class="foot">
        {#if data.error}
          <button class="done" onclick={close}>{t("share_in.close")}</button>
        {:else}
          <button class="done" onclick={done} disabled={!ready || busy}>{t("share_in.done")}</button>
        {/if}
      </div>
    </div>
  </div>
{:else}
  <div class="layer" transition:fade|global={{ duration: 160 }}>
    <div class="tdim"></div>
    <div class="bd" onpointerdown={(e) => e.target === e.currentTarget && close()} role="presentation">
      <div class="panel" transition:scale|global={{ start: 0.97, duration: 180, easing: cubicOut }} role="dialog" tabindex="-1">
        <div class="tt">{t("share_in.title")}</div>
        {@render head()}
        <div class="list scroll" use:fadeEdges={{ top: 10, bottom: 10, minOverflow: 8 }}>{@render rows()}</div>
        <div class="dfoot">
          <span class="flex1"></span>
          <Button onclick={close}>{t("dialog.cancel")}</Button>
          <Button kind="primary" onclick={done} disabled={!ready || busy}>{t("share_in.done")}</Button>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .hd { display: flex; align-items: center; gap: 12px; padding: 4px 6px; }
  .ht { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .t1 { font-size: 14px; font-weight: 600; color: rgba(255, 255, 255, .95); }
  .t2 { font-size: 12.5px; color: rgba(255, 255, 255, .55); }
  .play { flex: none; width: 36px; height: 36px; border-radius: 50%; background: #fff; display: grid; place-items: center; transition: transform .14s var(--ease); }
  .play:active { transform: scale(.92); }

  .row { flex: none; width: 100%; min-height: 44px; padding: 6px 10px; border-radius: 8px; display: flex; align-items: center; gap: 12px; text-align: left; cursor: pointer; transition: background-color .12s ease-out, transform .16s var(--ease); }
  .row:hover { background: rgba(255, 255, 255, .07); }
  .row:active { background: rgba(255, 255, 255, .1); transform: scale(.985); }
  .row:disabled { cursor: default; }
  .row:disabled:hover, .row:disabled:active { background: none; transform: none; }
  .sk-row { cursor: default; }
  .sk-row:hover, .sk-row:active { background: none; transform: none; }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .nm { font-size: 13.5px; font-weight: 500; color: rgba(255, 255, 255, .9); }
  .ct { font-size: 12px; color: rgba(255, 255, 255, .5); }
  .lk, .nw { flex: none; width: 32px; height: 32px; border-radius: 4px; display: grid; place-items: center; }
  .lk { background: var(--accent); }
  .nw { background: rgba(255, 255, 255, .08); }
  .chk { flex: none; width: 20px; height: 20px; border-radius: 5px; background: rgba(255, 255, 255, .16); display: grid; place-items: center; transition: background-color .12s ease-out; }
  .chk.on { background: var(--accent); }
  .row:disabled .chk.on { opacity: .5; }
  .sep { height: 1px; margin: 4px 8px; background: rgba(255, 255, 255, .07); }
  .err { padding: 8px 4px; }

  /* desktop dialog (NameDialog) */
  .layer { position: fixed; inset: 0; z-index: 90; }
  .tdim { position: absolute; left: 0; right: 0; top: 0; height: var(--titlebar-h); pointer-events: none; background: rgba(0, 0, 0, .45); }
  .bd { position: absolute; top: var(--titlebar-h); left: 0; right: 0; bottom: 0; padding-bottom: var(--titlebar-h); display: flex; align-items: center; justify-content: center; background: rgba(0, 0, 0, .45); }
  .panel { width: 380px; max-width: calc(100vw - 32px); max-height: calc(100vh - var(--titlebar-h) * 2 - 32px); padding: 20px; border-radius: 12px; background: #161616; display: flex; flex-direction: column; gap: 14px; }
  .tt { font-size: 16px; font-weight: 600; color: rgba(255, 255, 255, .95); }
  .list { flex: 0 1 auto; min-height: 0; max-height: 320px; margin: 0 -6px; padding: 0 6px; display: flex; flex-direction: column; }
  .dfoot { display: flex; align-items: center; gap: 8px; }
  .flex1 { flex: 1; }

  /* phone sheet (ContextMenu bs-*) */
  .bs-dim { position: fixed; inset: 0; z-index: 80; background: rgba(0, 0, 0, .55); }
  /* Over another app the window itself is dimmed (it shows before the page). */
  .bs-dim.sheet { background: transparent; }
  .bs-panel { position: absolute; left: 0; right: 0; bottom: 0; max-height: min(82vh, calc(100vh - var(--safe-top, env(safe-area-inset-top)) - 16px)); border-radius: 16px 16px 0 0; border-top: 1px solid rgba(255, 255, 255, .08); background: hsl(0 0% 8%); box-shadow: 0 -12px 40px rgba(0, 0, 0, .6); padding-bottom: max(10px, var(--safe-bottom, env(safe-area-inset-bottom)), var(--ime-bottom, 0px)); display: flex; flex-direction: column; overflow: hidden; }
  .bs-panel.settle { transition: transform .24s var(--ease); }
  .bs-grab { flex: none; touch-action: none; padding: 0 10px 10px; border-bottom: 1px solid rgba(255, 255, 255, .07); }
  .hdl { display: block; width: 36px; height: 4px; margin: 10px auto 6px; border-radius: 2px; background: rgba(255, 255, 255, .25); }
  .title { padding: 2px 6px 10px; font-size: 16px; font-weight: 700; color: rgba(255, 255, 255, .95); }
  .bs-panel .t1 { font-size: 15px; }
  .bs-list { flex: 0 1 auto; min-height: 0; padding: 6px; display: flex; flex-direction: column; }
  .bs-list .row { min-height: 56px; padding: 7px 10px; gap: 14px; }
  .bs-list .nm { font-size: 15px; }
  .bs-list .ct { font-size: 12.5px; }
  .bs-list .lk, .bs-list .nw { width: 40px; height: 40px; }
  .foot { flex: none; padding: 10px 14px 4px; border-top: 1px solid rgba(255, 255, 255, .07); }
  .done { width: 100%; height: 46px; border-radius: 999px; background: #fff; color: #000; font-size: 15px; font-weight: 700; text-align: center; transition: transform .14s var(--ease), opacity .14s ease-out; }
  .done:active { transform: scale(.98); }
  .done:disabled { opacity: .4; }
  .done:disabled:active { transform: none; }
</style>
