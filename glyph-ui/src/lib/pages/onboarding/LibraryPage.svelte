<script lang="ts">
  // onboarding.rs library(): songs with hearts to tap (the first one taps
  // itself at 1.1 s), a playlist cover that assembles from four tiles and
  // the code that shares it (click copies). Rows h 42, pl 8, pr 4, radius 10,
  // white 4 %, cover 28; mosaic 88 (64 narrow) with 3 px gaps; code chip
  // h 26, px 8, radius 7, mono 10.5.
  import Icon from "../../components/Icon.svelte";
  import ToggleButton from "../../components/ToggleButton.svelte";
  import { t, tf } from "../../i18n.svelte";
  import { decode } from "../../ui.svelte";
  import { easeOut, k, LIB_SONGS, MOSAIC, MOSAIC_COVERS, ob, SHARE_CODE } from "./anim.svelte";
  let { s, now, w }: { s: number; now: number; w: number } = $props();

  const narrow = $derived(w < 460);
  const rows = $derived(narrow ? 2 : 3);
  const m = $derived(narrow ? 64 : 88);
  const tile = $derived((m - 3) / 2);
  const copied = $derived(ob.copiedAt > 0 && now - ob.copiedAt < 1600);

  $effect(() => {
    if (s > 1.1 && !ob.autoLiked) { ob.autoLiked = true; if (!(ob.likes & 1)) ob.likes |= 1; }
  });
  function like(i: number) { ob.autoLiked = true; ob.likes ^= 1 << i; }
  function copy() { ob.copiedAt = performance.now(); try { navigator.clipboard.writeText(SHARE_CODE); } catch { /* preview */ } }
</script>

<div class="p" class:narrow>
  <div class="songs">
    {#each LIB_SONGS.slice(0, rows) as [title, artist, hue, cover], i (title)}
      {@const a = easeOut(k(s, 0.08 * i, 0.35))}
      <div class="r" style:opacity={a} style:transform="translateX({-10 * (1 - a)}px)">
        <span class="cv" style:background="hsl({hue * 360} 45% 42%)" style:background-image="url({cover})" style:background-size="cover"></span>
        <span class="tt"><span class="a">{title}</span><span class="b">{artist}</span></span>
        <ToggleButton name="heart" size={16} on={!!(ob.likes & (1 << i))} accent="var(--accent)" onclick={() => like(i)} />
      </div>
    {/each}
  </div>

  <div class="list">
    <div class="mos" style:width="{m}px" style:height="{m}px">
      {#each MOSAIC as hue, i (i)}
        {@const col = i % 2}
        {@const row = Math.floor(i / 2)}
        {@const e = easeOut(k(s, 0.35 + 0.1 * i, 0.5))}
        <span class="t" style:width="{tile}px" style:height="{tile}px" style:opacity={e}
          style:left="{col * (tile + 3) + (col * 2 - 1) * 26 * (1 - e)}px" style:top="{row * (tile + 3) + (row * 2 - 1) * 26 * (1 - e)}px"
          style:background="hsl({hue * 360} 50% 45%)" style:background-image="url({MOSAIC_COVERS[i]})" style:background-size="cover"></span>
      {/each}
    </div>
    <div class="side">
      <div class="nm" style:opacity={easeOut(k(s, 0.8, 0.3))}>
        <div class="n1">Roadtrip</div>
        <div class="n2">{tf("common.n_songs", { n: 24 })}</div>
      </div>
      <button class="code" style:opacity={easeOut(k(s, 1, 0.3))} onclick={copy}>
        <span class="mono cd">{decode(SHARE_CODE, k(s, 1.1, 0.8))}</span>
        <Icon name={copied ? "check" : "copy"} size={12} color={copied ? "var(--ok)" : "rgba(255,255,255,.5)"} />
      </button>
    </div>
  </div>
</div>

<style>
  .p { width: 100%; height: 100%; display: flex; align-items: center; justify-content: center; gap: 18px; padding: 18px 20px; }
  .p.narrow { flex-direction: column; }
  .songs { flex: 1; max-width: 290px; min-width: 0; display: flex; flex-direction: column; gap: 6px; }
  .narrow .songs { flex: none; width: 100%; max-width: none; }
  .r { display: flex; align-items: center; gap: 10px; height: 42px; padding: 0 4px 0 8px; border-radius: 10px; background: rgba(255, 255, 255, .04); }
  .cv { flex: none; width: 28px; height: 28px; border-radius: 5.6px; }
  .tt { flex: 1; min-width: 0; overflow: hidden; white-space: nowrap; display: flex; flex-direction: column; }
  .a { font-size: 12.5px; color: rgba(255, 255, 255, .9); }
  .b { font-size: 11px; color: rgba(255, 255, 255, .42); }
  .list { flex: none; display: flex; flex-direction: column; align-items: center; gap: 8px; }
  .narrow .list { flex-direction: row; gap: 12px; }
  .mos { position: relative; flex: none; }
  .t { position: absolute; border-radius: 5px; }
  .side { display: flex; flex-direction: column; align-items: center; gap: 8px; }
  .narrow .side { align-items: flex-start; }
  .nm { display: flex; flex-direction: column; align-items: center; }
  .narrow .nm { align-items: flex-start; }
  .n1 { font-size: 12.5px; font-weight: 600; color: rgba(255, 255, 255, .9); }
  .n2 { font-size: 11px; color: rgba(255, 255, 255, .42); }
  .code { display: flex; align-items: center; gap: 6px; height: 26px; padding: 0 8px; border-radius: 7px; background: rgba(255, 255, 255, .05); transition: background-color .12s; }
  .code:hover { background: rgba(255, 255, 255, .09); }
  .cd { font-size: 10.5px; color: rgba(255, 255, 255, .7); }
</style>
