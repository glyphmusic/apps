<script lang="ts">
  // views.rs home_cards: flat dark cards (h 56, gap 14, card colour, radius
  // 8, hover white 4 %) as a grid: as many columns as fit at 260 px, each
  // stretched up to 420; rows are made even (6 cards = 3 + 3). First Liked
  // Songs (heart tile 56 in `active`), then up to two playlists you listened
  // to lately, active songs, other playlists until 6 cards are there.
  // "Active" is approximated from the recent list (the Rust side counts
  // plays over time, which the UI does not have).
  import Icon from "../../components/Icon.svelte";
  import ListCover from "../../components/ListCover.svelte";
  import Cover from "../../components/Cover.svelte";
  import SongTitle from "../../components/SongTitle.svelte";
  import { app } from "../../state.svelte";
  import { t, tf } from "../../i18n.svelte";
  import { primaryArtist, rating, trackKey, type ListKey, type Track } from "../../api/types";

  const MAX = 420;
  // Phones get two columns (150 px) instead of a tall stack.
  const MIN = $derived(app.compact ? 150 : 260), GAP = $derived(app.compact ? 8 : 10);
  let w = $state(1000);

  const hot = $derived.by(() => {
    const ids: string[] = [];
    for (const r of app.library.recent) {
      if (r.kind === "list" && r.list.kind === "playlist" && app.playlist(r.list.id) && !ids.includes(r.list.id)) ids.push(r.list.id);
      if (ids.length >= 2) break;
    }
    return ids;
  });
  const active = $derived.by<Track[]>(() => {
    const out: Track[] = [], seen = new Set<string>();
    for (const r of app.library.recent) {
      if (r.kind !== "song" || !app.allowed(r.track) || seen.has(trackKey(r.track))) continue;
      seen.add(trackKey(r.track));
      out.push(r.track);
      if (out.length >= 5) break;
    }
    return out;
  });
  const plIds = $derived.by(() => {
    const songs = Math.min(active.length, 5 - hot.length);
    const rest = app.library.playlists.filter((p) => !hot.includes(p.id)).map((p) => p.id).slice(0, 5 - hot.length - songs);
    return [...hot, ...rest];
  });
  const pls = $derived(plIds.map((id) => app.playlist(id)).filter((p) => !!p));
  const songs = $derived(active.slice(0, Math.max(0, 6 - 1 - pls.length)));
  const count = $derived(1 + pls.length + songs.length);
  const fit = $derived(Math.min(Math.max(1, Math.floor((w + GAP) / (MIN + GAP))), Math.max(1, count)));
  const rows = $derived(Math.ceil(count / fit));
  const cols = $derived(Math.ceil(count / rows));

  const open = (l: ListKey) => app.openListView(l);
  function listMenu(l: ListKey) { return (e: MouseEvent) => { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: null, list: l }); }; }
  function songMenu(tr: Track) { return (e: MouseEvent) => { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: tr }); }; }
  function artistMenu(name: string) { return (e: MouseEvent) => { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: null, artist: primaryArtist(name) }); }; }
</script>

<div class="wrap" style:--cg="{app.compact ? 10 : 14}px" bind:clientWidth={w}>
  <div class="grid" style:grid-template-columns="repeat({cols}, minmax(0, 1fr))" style:gap="{GAP}px" style:max-width="{cols * MAX + (cols - 1) * GAP}px">
    <div class="c" onclick={() => open({ kind: "liked" })} oncontextmenu={listMenu({ kind: "liked" })} onkeydown={(e) => e.key === "Enter" && open({ kind: "liked" })} role="button" tabindex="-1">
      <div class="liked"><Icon name="heart" size={20} color="var(--text)" /></div>
      <div class="tx">
        <div class="tl">{t("library.liked")}</div>
        <div class="sub">{tf("common.n_songs", { n: app.library.liked.length })}</div>
      </div>
    </div>
    {#each pls as p (p.id)}
      {@const l = { kind: "playlist", id: p.id } as ListKey}
      <div class="c" onclick={() => open(l)} oncontextmenu={listMenu(l)} onkeydown={(e) => e.key === "Enter" && open(l)} role="button" tabindex="-1">
        <ListCover list={l} size={56} />
        <div class="tx">
          <div class="tl trunc">{p.name}</div>
          <div class="sub trunc">{tf("common.n_songs", { n: p.tracks.length })}</div>
        </div>
      </div>
    {/each}
    {#each songs as tr, i (trackKey(tr))}
      <div class="c" class:off={!app.allowed(tr)} onclick={() => app.allowed(tr) && app.play(songs, i)} oncontextmenu={songMenu(tr)} onkeydown={(e) => e.key === "Enter" && app.play(songs, i)} role="button" tabindex="-1">
        <Cover track={tr} size={56} radius={8} />
        <div class="tx">
          <div class="tl"><SongTitle rating={rating(tr)}>{tr.title}</SongTitle></div>
          <div class="sub">
            <button class="link trunc" onclick={(e) => { e.stopPropagation(); app.openArtistPage(tr.artist); }} oncontextmenu={artistMenu(tr.artist)}>{tr.artist}</button>
          </div>
        </div>
      </div>
    {/each}
  </div>
</div>

<style>
  .wrap { width: 100%; position: relative; }
  .grid { display: grid; gap: 10px; width: 100%; }
  .c { position: relative; display: flex; align-items: center; gap: var(--cg, 14px); height: 56px; width: 100%; min-width: 0; overflow: hidden; border-radius: 8px; background: var(--card); cursor: pointer; --ink: 242, 242, 242; }
  .c::before { content: ""; position: absolute; inset: 0; background: rgba(255, 255, 255, .04); opacity: 0; transition: opacity .24s ease-out; pointer-events: none; }
  .c:hover::before { opacity: 1; transition-duration: .12s; }
  .c.off { opacity: .4; }
  .liked { flex: none; width: 56px; height: 56px; display: grid; place-items: center; background: var(--active); }
  .tx { position: relative; min-width: 0; display: flex; flex-direction: column; gap: 2px; padding-right: 10px; }
  .tl { font-size: var(--md); font-weight: 500; min-width: 0; }
  .sub { display: flex; min-width: 0; font-size: var(--sm); color: var(--dim); }
  .link { min-width: 0; max-width: 100%; }
  .link:hover { color: var(--text); text-decoration: underline; }
</style>
