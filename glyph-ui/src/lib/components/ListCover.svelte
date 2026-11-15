<script lang="ts">
  // lists.rs list_cover: Liked = heart on a dark accent tile, Recent =
  // history icon, Downloads = download icon; playlists get a mosaic of
  // distinct song covers (4 → 2x2, 3 → one tall + two small, 2 → halves),
  // albums their first song's cover. Radius = size x 0.06, 6..12.
  import Icon from "./Icon.svelte";
  import Cover from "./Cover.svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { accentTile, themeColors } from "../theme";
  import type { ListKey } from "../api/types";
  let { list, size = 36 }: { list: ListKey; size?: number } = $props();
  const r = $derived(Math.min(12, Math.max(6, size * 0.06)));
  const accent = $derived(themeColors(app.config?.visuals.theme ?? "glyph").accent);
  const tracks = $derived(app.listTracks(list).slice(0, 12));
  const coverTrack = $derived(list.kind === "album" ? (list.cover ?? tracks[0] ?? null) : (tracks[0] ?? null));
  const own = $derived(list.kind === "playlist" ? app.playlist(list.id) : undefined);
  const mosaic = $derived(list.kind === "playlist" || (list.kind === "album" && list.artist.startsWith("@")));
  let urls = $state<string[]>([]);
  $effect(() => {
    if (!mosaic) return;
    const ts = tracks;
    let alive = true;
    Promise.all(ts.map((t) => backend().cover(t))).then((us) => {
      if (!alive) return;
      const out: string[] = [];
      for (const u of us) if (u && !out.includes(u)) { out.push(u); if (out.length === 4) break; }
      urls = out;
    });
    return () => { alive = false; };
  });
  const tile = $derived(
    list.kind === "liked" || list.kind === "artist_liked" ? { bg: accentTile(0.22, 0.45, accent), icon: "heart-fill", fg: "var(--accent)" }
    : list.kind === "recent" ? { bg: "var(--card)", icon: "history", fg: "rgba(255,255,255,.75)" }
    : list.kind === "downloads" ? { bg: accentTile(0.16, 0.3, accent), icon: "download", fg: "var(--accent)" }
    : null,
  );
  const isz = $derived(Math.min(72, Math.max(16, size * 0.3)));
</script>

{#if tile}
  <div class="tile" style:width="{size}px" style:height="{size}px" style:border-radius="{r}px" style:background={tile.bg}><Icon name={tile.icon} size={isz} color={tile.fg} /></div>
{:else if own?.cover}
  <img class="one" src={own.cover} alt="" style:width="{size}px" style:height="{size}px" style:border-radius="{r}px" />
{:else if mosaic && urls.length >= 2}
  <div class="mo m{Math.min(4, urls.length)}" style:width="{size}px" style:height="{size}px" style:--r="{r}px">
    {#each urls.slice(0, urls.length === 3 ? 3 : urls.length >= 4 ? 4 : 2) as u, i}<img class="q{i}" src={u} alt="" />{/each}
  </div>
{:else if coverTrack}
  <Cover track={coverTrack} {size} radius={r} />
{:else}
  <div class="tile" style:width="{size}px" style:height="{size}px" style:border-radius="{r}px" style:background="var(--card)"><Icon name={list.kind === "album" ? "disc-3" : "list-music"} size={isz} color="rgba(255,255,255,.5)" /></div>
{/if}

<style>
  .tile { flex: none; display: grid; place-items: center; }
  .one { flex: none; object-fit: cover; }
  .mo { position: relative; flex: none; }
  .mo img { position: absolute; object-fit: cover; background: var(--card); }
  .m4 img { width: 50%; height: 50%; }
  .m4 .q0 { left: 0; top: 0; border-radius: var(--r) 0 0 0; }
  .m4 .q1 { right: 0; top: 0; border-radius: 0 var(--r) 0 0; }
  .m4 .q2 { left: 0; bottom: 0; border-radius: 0 0 0 var(--r); }
  .m4 .q3 { right: 0; bottom: 0; border-radius: 0 0 var(--r) 0; }
  .m2 img { top: 0; width: 50%; height: 100%; }
  .m2 .q0 { left: 0; border-radius: var(--r) 0 0 var(--r); }
  .m2 .q1 { right: 0; border-radius: 0 var(--r) var(--r) 0; }
  .m3 .q0 { left: 0; top: 0; width: 50%; height: 100%; border-radius: var(--r) 0 0 var(--r); }
  .m3 .q1 { right: 0; top: 0; width: 50%; height: 50%; border-radius: 0 var(--r) 0 0; }
  .m3 .q2 { right: 0; bottom: 0; width: 50%; height: 50%; border-radius: 0 0 var(--r) 0; }
</style>
