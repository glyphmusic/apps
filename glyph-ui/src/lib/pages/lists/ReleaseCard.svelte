<script lang="ts">
  // lists/artist.rs release_card / album_card: w size + 16, p 8, radius 10,
  // gap 4, hover white 5 %; cover `size` radius 8, title md medium (pt 4,
  // truncated), "2019 · EP" mono 11 faint in caps. Right click: list menu.
  import Cover from "../../components/Cover.svelte";
  import { app } from "../../state.svelte";
  import type { ListKey, Track } from "../../api/types";
  let { title, artist, tracks, label, size = 160, onopen }: { title: string; artist: string; tracks: Track[]; label: string; size?: number; onopen: () => void } = $props();
  const lk = $derived<ListKey>({ kind: "album", title, artist, tracks });
  function menu(e: MouseEvent) {
    e.preventDefault(); e.stopPropagation();
    app.openCtx({ x: e.clientX, y: e.clientY, track: tracks[0] ?? null, list: lk });
  }
</script>

<button class="rc" style:width="{size + 16}px" onclick={onopen} oncontextmenu={menu}>
  <Cover track={tracks[0]} {size} radius={8} />
  <span class="tl trunc">{title}</span>
  <span class="lb mono">{label.toUpperCase()}</span>
</button>

<style>
  .rc { flex: none; padding: 8px; border-radius: 10px; display: flex; flex-direction: column; gap: 4px; transition: background-color .12s ease-out; }
  .rc:hover { background: rgba(255, 255, 255, .05); }
  .tl { padding-top: 4px; font-size: var(--md); font-weight: 500; color: var(--text); }
  .lb { font-size: 11px; color: var(--faint); }
</style>
