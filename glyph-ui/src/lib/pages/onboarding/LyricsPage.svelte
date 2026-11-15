<script lang="ts">
  // onboarding.rs lyrics(): synced lines (bars of varying width) glide up
  // while the song plays; the cover pauses it. Line pitch 30, the sung part
  // of the current line fills white; cover 96 (76 narrow), radius 12, accent
  // tile, white play / pause disc 40; progress 3 px; time mono 10.5.
  import Icon from "../../components/Icon.svelte";
  import { app } from "../../state.svelte";
  import { accentTile, themeColors } from "../../theme";
  import { easeInOut, k, LINE_S, LINES, ob, songTime } from "./anim.svelte";
  let { now, still, w, h }: { now: number; still: boolean; w: number; h: number } = $props();

  const lt = $derived(songTime(now));
  const pos = $derived(lt / LINE_S);
  const line = $derived(Math.floor(pos));
  const frac = $derived(pos - line);
  const scroll = $derived(still ? line : line + easeInOut(k(frac, 0.8, 0.2)));
  const narrow = $derived(w < 460);
  const c = $derived(narrow ? 76 : 96);
  const viewH = $derived(h - 36);
  const linesW = $derived(w - 48 - c - 24);
  const playing = $derived(ob.lyricsPaused == null);
  const secs = $derived(Math.floor(lt + 52));
  const accent = $derived(themeColors(app.config?.visuals.theme ?? "glyph").accent);
  const idx = $derived(Array.from({ length: 10 }, (_, i) => line - 4 + i));

  function toggle() {
    if (ob.lyricsPaused != null) { ob.lyricsBase = ob.lyricsPaused; ob.lyricsAt = performance.now(); ob.lyricsPaused = null; }
    else ob.lyricsPaused = songTime(performance.now());
  }
</script>

<div class="p">
  <div class="side">
    <button class="cover" style:width="{c}px" style:height="{c}px" style:background={accentTile(0.32, 0.55, accent)} onclick={toggle}>
      <span class="disc"><Icon name={playing ? "pause" : "play"} size={18} color="rgba(0,0,0,.9)" /></span>
    </button>
    <div class="prog" style:width="{c}px"><div class="pf" style:width="{c * (((lt + 52) / 214) % 1)}px"></div></div>
    <div class="tm mono">{Math.floor(secs / 60)}:{String(secs % 60).padStart(2, "0")}</div>
  </div>
  <div class="lines" style:height="{viewH}px">
    {#each idx as j (j)}
      {@const d = j - scroll}
      {@const focus = Math.min(1, Math.max(0, 1 - Math.abs(d)))}
      {@const fade = Math.min(1, Math.max(0, 1 - Math.abs(d) / 4))}
      {@const bw = linesW * LINES[((j % LINES.length) + LINES.length) % LINES.length]}
      {@const bh = 9 + 3 * focus}
      {@const sung = j < line ? 1 : j === line ? Math.min(1, frac / 0.8) : 0}
      <div class="ln" style:top="{viewH / 2 + d * 30 - bh / 2}px" style:width="{bw}px" style:height="{bh}px" style:background="rgba(255,255,255,{(0.07 + 0.1 * focus) * fade})">
        <div class="fill" style:width="{bw * sung}px" style:background="rgba(255,255,255,{(0.22 + 0.73 * focus) * fade})"></div>
      </div>
    {/each}
  </div>
</div>

<style>
  .p { width: 100%; height: 100%; display: flex; align-items: center; gap: 24px; padding: 0 24px; }
  .side { flex: none; display: flex; flex-direction: column; gap: 8px; }
  .cover { border-radius: 12px; display: grid; place-items: center; transition: transform .15s var(--ease); }
  .cover:active { transform: scale(.96); }
  .disc { width: 40px; height: 40px; border-radius: 50%; background: rgba(255, 255, 255, .92); display: grid; place-items: center; }
  .prog { height: 3px; border-radius: 2px; background: rgba(255, 255, 255, .1); overflow: hidden; }
  .pf { height: 100%; border-radius: 2px; background: var(--accent); }
  .tm { font-size: 10.5px; color: rgba(255, 255, 255, .45); }
  .lines { position: relative; flex: 1; overflow: hidden; }
  .ln { position: absolute; left: 0; border-radius: 999px; overflow: hidden; }
  .fill { height: 100%; }
</style>
