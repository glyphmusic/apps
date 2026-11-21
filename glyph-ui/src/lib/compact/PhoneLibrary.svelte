<script lang="ts">
  // compact.rs library: chips Playlists / Albums / Artists (tap again = all),
  // then rows: 56 px cover, title 15, "Playlist · 12 songs" 12.5 dim with a
  // pink pin when pinned; pinned lists first; followed artists round.
  import { fade } from "svelte/transition";
  import Icon from "../components/Icon.svelte";
  import Chip from "../components/Chip.svelte";
  import Cover from "../components/Cover.svelte";
  import ListCover from "../components/ListCover.svelte";
  import { app } from "../state.svelte";
  import { t, tf } from "../i18n.svelte";
  import { sameList, type ListKey } from "../api/types";
  import { phone } from "./cstate.svelte";
  const f = $derived(phone.libFilter);
  const set = (n: number) => {
    const next = phone.libFilter === n ? 0 : n;
    phone.libFilter = next;
    app.libraryTab = next;
  };
  const lists = $derived.by(() => {
    const out: { list: ListKey; sub: string }[] = [];
    const pl = t("list.kind.playlist");
    if (f <= 1) {
      out.push({ list: { kind: "liked" }, sub: `${pl} · ${tf("common.n_songs", { n: app.library.liked.length })}` });
      for (const p of app.library.playlists) out.push({ list: { kind: "playlist", id: p.id }, sub: `${pl} · ${tf("common.n_songs", { n: p.tracks.length })}` });
    }
    if (f === 0 || f === 2) for (const a of app.library.saved_albums) out.push({ list: a, sub: a.kind === "album" ? `${t("list.kind.album")} · ${a.artist}` : t("list.kind.album") });
    const rank = (l: ListKey) => { const i = app.library.pinned.findIndex((p) => sameList(p, l)); return i < 0 ? 1e9 : i; };
    return out.sort((a, b) => rank(a.list) - rank(b.list));
  });
  const artists = $derived(f === 0 || f === 3 ? app.library.followed : []);
  const artTrack = (name: string) => app.library.liked.find((x) => x.artist === name) ?? null;
  function listMenu(e: MouseEvent, l: ListKey) {
    e.preventDefault(); e.stopPropagation();
    app.openCtx({ x: e.clientX, y: e.clientY, track: app.listTracks(l)[0] ?? null, list: l });
  }
  function artistMenu(e: MouseEvent, name: string) {
    e.preventDefault(); e.stopPropagation();
    app.openCtx({ x: e.clientX, y: e.clientY, track: artTrack(name), artist: name });
  }
</script>

<div class="chips">
  <Chip on={f === 1} onclick={() => set(1)}>{t("library.filter.playlists")}</Chip>
  <Chip on={f === 2} onclick={() => set(2)}>{t("search.filter.albums")}</Chip>
  <Chip on={f === 3} onclick={() => set(3)}>{t("search.filter.artists")}</Chip>
</div>
<div class="stage">
  {#key f}
    <div
      class="slot"
      in:fade={{ duration: 150, delay: 95 }}
      out:fade={{ duration: 95 }}
      onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")}
    >
      {#each lists as it}
        <button class="lr" onclick={() => app.openListView(it.list)} oncontextmenu={(e) => listMenu(e, it.list)}>
          <ListCover list={it.list} size={56} />
          <span class="tx">
            <span class="t trunc">{app.listTitle(it.list)}</span>
            <span class="s">{#if app.isPinned(it.list)}<Icon name="pin-fill" size={12} color="var(--accent)" />{/if}<span class="trunc">{it.sub}</span></span>
          </span>
        </button>
      {/each}
      {#each artists as name}
        {@const tr = artTrack(name)}
        <button class="lr" onclick={() => app.openArtistPage(name)} oncontextmenu={(e) => artistMenu(e, name)}>
          {#if tr}<Cover track={tr} size={56} round />{:else}<span class="ph"><Icon name="user-round" size={22} color="rgba(255,255,255,.5)" /></span>{/if}
          <span class="tx"><span class="t trunc">{name}</span><span class="s">{t("search.kind.artist")}</span></span>
        </button>
      {/each}
      <div style:height="24px"></div>
    </div>
  {/key}
</div>

<style>
  .chips { display: flex; gap: 8px; padding: 0 16px 10px; }
  .stage { display: grid; }
  .slot { grid-area: 1 / 1; min-width: 0; }
  .lr { width: 100%; display: flex; align-items: center; gap: 12px; padding: 6px 16px; text-align: left; border-radius: 8px; transition: background-color .12s var(--ease), transform .16s var(--ease); }
  .lr:active { background: rgba(255, 255, 255, .05); transform: scale(.98); }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .t { font-size: 15px; color: var(--text); }
  .s { display: flex; align-items: center; gap: 5px; min-width: 0; font-size: 12.5px; color: var(--dim); }
  .ph { width: 56px; height: 56px; border-radius: 50%; background: var(--card); display: grid; place-items: center; }
</style>
