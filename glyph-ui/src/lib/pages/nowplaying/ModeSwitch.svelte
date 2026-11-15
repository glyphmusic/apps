<script lang="ts">
  // now_playing.rs mode_switch + mode_segment: "Song + lyrics" / "Lyrics" /
  // "Zen" as one segmented control. Container p 3, gap 2, radius 10, hsla
  // 0 0 6 % .82, 1 px white 10 % border. Segment h 28, px 10, radius 7, gap
  // 6, 12.5 px medium, 14 px icon; the active one is filled white with black
  // text, the others white 65 % (hover 95 % + white 8 % fill).
  import Icon from "../../components/Icon.svelte";
  import { app } from "../../state.svelte";
  import { t } from "../../i18n.svelte";
  const segs = $derived([
    { icon: "layout-split", label: t("np.mode.split"), on: !app.lyricsOnly, run: () => (app.lyricsOnly = false) },
    { icon: "text-center", label: t("np.mode.lyrics"), on: app.lyricsOnly, run: () => (app.lyricsOnly = true) },
    { icon: "maximize-2", label: t("np.mode.zen"), on: false, run: () => (app.zen = true) },
  ]);
</script>

<div class="ms" onpointerdown={(e) => e.stopPropagation()} onmousedown={(e) => e.stopPropagation()} onclick={(e) => e.stopPropagation()} role="presentation">
  {#each segs as s}
    <button class="seg" class:on={s.on} type="button" onpointerdown={(e) => e.stopPropagation()} onclick={(e) => { e.stopPropagation(); s.run(); }}><Icon name={s.icon} size={14} />{s.label}</button>
  {/each}
</div>

<style>
  .ms { display: inline-flex; align-items: center; gap: 2px; padding: 3px; border-radius: 9px; background: #161618; }
  .seg { display: flex; align-items: center; gap: 6px; height: 28px; padding: 0 12px; border-radius: 7px; font-size: 13px; color: rgba(255, 255, 255, .6); transition: background-color .14s ease-out, color .14s ease-out; }
  .seg :global(.ic) { color: rgba(255, 255, 255, .55); transition: color .14s ease-out; }
  .seg:hover { background: #222226; color: var(--text); }
  .seg:hover :global(.ic) { color: var(--text); }
  .seg.on, .seg.on:hover { background: #2c2c32; color: var(--text); font-weight: 500; }
  .seg.on :global(.ic), .seg.on:hover :global(.ic) { color: var(--text); }
</style>
