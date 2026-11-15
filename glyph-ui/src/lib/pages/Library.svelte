<script lang="ts">
  // lists/library.rs library: heading + actions (Local files, Transfer
  // music, Your stats: h 32 px 14 radius 8 1 px border, sm white 90 %, 14 px
  // icon gap 6, hover card_hover; New playlist the same on a card fill),
  // column gap 18. Tiles (Liked, Recent, Downloads, own playlists, saved
  // albums; pinned first in pin order) wrap with gap 8 / ml -8: w 184, p 8,
  // radius 10, gap 4, hover white 5 %, cover 168 with a pin badge (24 round,
  // black 60 %, pin-fill 12 accent, top / right 8), title md medium (pt 6),
  // "kind · n songs" sm dim. Followed artists below (section title, round
  // cards). Deleted playlists last: restore, or × to delete for good.
  // Phones (compact.rs library): filter chips Playlists / Albums / Artists
  // (app.libraryTab: 0 all, 1, 2, 3).
  import Icon from "../components/Icon.svelte";
  import ListCover from "../components/ListCover.svelte";
  import SectionTitle from "../components/SectionTitle.svelte";
  import Chip from "../components/Chip.svelte";
  import Select from "../settings/Select.svelte";
  import ArtistCard from "./lists/ArtistCard.svelte";
  import { app } from "../state.svelte";
  import { phone } from "../compact/cstate.svelte";
  import { onWeb } from "../web";
  import { t, tf } from "../i18n.svelte";
  import { sameList, type ListKey } from "../api/types";
  import { playlistX, scopeKey } from "./lists/actions";

  const touch = $derived(app.platform === "android" || app.platform === "ios");
  const filter = $derived(app.compact ? app.libraryTab : 0);

  const tiles = $derived.by(() => {
    const out: { l: ListKey; kind: string }[] = [];
    const pl = t("list.kind.playlist");
    if (filter <= 1) {
      out.push({ l: { kind: "liked" }, kind: pl });
      if (filter === 0 && app.library.recent.length) out.push({ l: { kind: "recent" }, kind: pl });
      if (filter === 0 && app.library.downloads.length) out.push({ l: { kind: "downloads" }, kind: pl });
      for (const p of app.library.playlists) out.push({ l: { kind: "playlist", id: p.id }, kind: t(scopeKey(playlistX(p.id))) });
    }
    if (filter === 0 || filter === 2) {
      for (const k of app.library.saved_albums) out.push({ l: k, kind: k.kind === "album" ? `${t("list.kind.album")} · ${k.artist}` : t("list.kind.album") });
    }
    // Pinned lists first, in the order they were pinned (stable).
    const rank = (l: ListKey) => { const i = app.library.pinned.findIndex((p) => sameList(p, l)); return i < 0 ? Infinity : i; };
    return out.map((x, i) => ({ ...x, i })).sort((a, b) => rank(a.l) - rank(b.l) || a.i - b.i);
  });
  const followed = $derived(filter === 0 || filter === 3 ? app.library.followed : []);

  function menu(e: MouseEvent, l: ListKey) {
    e.preventDefault(); e.stopPropagation();
    const first = app.listTracks(l)[0];
    if (!first && l.kind !== "playlist") return;
    app.openCtx({ x: e.clientX, y: e.clientY, track: first ?? null, list: l });
  }
  const pick = (f: number) => {
    const next = app.libraryTab === f ? 0 : f;
    app.libraryTab = next;
    phone.libFilter = next;
  };

  type LibAction = "local" | "migrate" | "stats" | "new";
  const actionOptions = $derived.by(() => {
    const opts: { value?: LibAction; label?: string; icon?: string; sep?: boolean }[] = [];
    if (!touch && !onWeb()) opts.push({ value: "local", label: t("local.title"), icon: "hard-drive" });
    if (!onWeb()) opts.push({ value: "migrate", label: t("library.migrate"), icon: "arrow-right-left" });
    opts.push({ value: "stats", label: t("stats.title"), icon: "chart-column" });
    opts.push({ sep: true });
    opts.push({ value: "new", label: t("library.playlist.new"), icon: "plus" });
    return opts;
  });
  function pickAction(v: LibAction | null) {
    if (v === "local") app.navigate("local");
    else if (v === "migrate") app.openSettings("migrate");
    else if (v === "stats") app.navigate("stats");
    else if (v === "new") app.nameDialog = { tracks: [], rename: null, initial: "" };
  }
</script>

<div class="lib">
  <div class="head">
    <SectionTitle heading title={t("nav.library")} />
    <Select value={null as LibAction | null} label={t("ctx.more")} options={actionOptions} onpick={pickAction} />
  </div>

  {#if app.compact}
    <div class="chips">
      <Chip on={filter === 1} onclick={() => pick(1)}>{t("library.filter.playlists")}</Chip>
      <Chip on={filter === 2} onclick={() => pick(2)}>{t("search.filter.albums")}</Chip>
      <Chip on={filter === 3} onclick={() => pick(3)}>{t("search.filter.artists")}</Chip>
    </div>
  {/if}

  {#if tiles.length}
    <div class="grid">
      {#each tiles as { l, kind } (JSON.stringify({ ...l, tracks: undefined }))}
        <button class="tile" onclick={() => app.openListView(l)} oncontextmenu={(e) => menu(e, l)}>
          <span class="art">
            <ListCover list={l} size={168} />
            {#if app.isPinned(l)}<span class="pin"><Icon name="pin-fill" size={12} color="var(--accent)" /></span>{/if}
          </span>
          <span class="tl trunc">{app.listTitle(l)}</span>
          <span class="sub trunc">{kind} · {tf("common.n_songs", { n: app.listTracks(l).length })}</span>
        </button>
      {/each}
    </div>
  {/if}

  {#if followed.length}
    <div class="artists">
      <SectionTitle icon="user-round" title={t("library.artists")} />
      <div class="grid">
        {#each followed as name (name)}<ArtistCard {name} />{/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .lib { display: flex; flex-direction: column; gap: 18px; }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .chips { display: flex; gap: 8px; }
  .grid { display: flex; flex-wrap: wrap; gap: 8px; margin-left: -8px; }
  .tile { flex: none; width: 184px; padding: 8px; border-radius: 10px; display: flex; flex-direction: column; gap: 4px; transition: background-color .12s ease-out; animation: rise-in .25s var(--ease) both; }
  .tile:hover { background: rgba(255, 255, 255, .05); }
  .art { position: relative; display: block; }
  .pin { position: absolute; top: 8px; right: 8px; width: 24px; height: 24px; border-radius: 50%; background: rgba(0, 0, 0, .6); display: grid; place-items: center; animation: pop .26s var(--ease); }
  .tl { padding-top: 6px; font-size: var(--md); font-weight: 500; color: var(--text); }
  .sub { font-size: var(--sm); color: var(--dim); }
  .artists { display: flex; flex-direction: column; gap: 10px; padding-top: 10px; }
</style>
