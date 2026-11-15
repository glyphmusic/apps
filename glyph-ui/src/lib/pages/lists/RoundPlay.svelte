<script lang="ts">
  // lists/rows.rs round_play: white circle, play / pause / loading mark at
  // 42 % of the size (the triangle nudged 2 px right), hover black 10 %.
  import PlayGlyph from "../../components/PlayGlyph.svelte";
  import { tip, press } from "../../ui.svelte";
  import { tipFor } from "../../i18n.svelte";
  let { size = 48, playing = false, loading = false, accent = false, onclick }: { size?: number; playing?: boolean; loading?: boolean; accent?: boolean; onclick?: (e: MouseEvent) => void } = $props();
</script>

<button class="rp" class:acc={accent} style:width="{size}px" style:height="{size}px" use:tip={tipFor(playing ? "pause" : "play")} use:press {onclick} aria-label={tipFor(playing ? "pause" : "play")}>
  <span class="g" class:nudge={!playing && !loading}><PlayGlyph {loading} {playing} size={size * 0.42} button={size} /></span>
</button>

<style>
  .rp { position: relative; flex: none; display: grid; place-items: center; border-radius: 50%; background: #fff; }
  .rp.acc { background: var(--accent); }
  .rp::after { content: ""; position: absolute; inset: 0; border-radius: 50%; background: rgba(0, 0, 0, .1); opacity: 0; transition: opacity .12s ease-out; }
  .rp:hover::after { opacity: 1; }
  .rp:global(.pressing) { animation: press .25s var(--ease); }
  .g { display: grid; place-items: center; }
  .g.nudge { padding-left: 2px; }
</style>
