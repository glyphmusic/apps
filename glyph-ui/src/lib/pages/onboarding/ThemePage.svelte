<script lang="ts">
  // onboarding.rs themes(): a small player in the current colours (w <= 380,
  // h 64, radius 12, white 5 %: 44 cover tile, three skeleton bars, a moving
  // progress line and a 36 accent disc); swatches (38 round, 26 colour dot,
  // white 90 % ring on the current one) recolour the whole app right away;
  // the theme's name `[ OCEAN ]` decodes in below.
  import Icon from "../../components/Icon.svelte";
  import { app } from "../../state.svelte";
  import { t } from "../../i18n.svelte";
  import { accentTile, css, themeColors, THEMES } from "../../theme";
  import { decode, revealK, revealRestart } from "../../ui.svelte";
  import { backOut, k } from "./anim.svelte";
  let { s, still, w }: { s: number; still: boolean; w: number } = $props();

  const current = $derived(app.config?.visuals.theme ?? "glyph");
  const accent = $derived(themeColors(current).accent);
  const pw = $derived(Math.min(w - 48, 380));
  const prog = $derived(still ? 0.4 : (0.18 + s * 0.035) % 1);
  const name = $derived((THEMES.find((x) => x.id === current) ? t(THEMES.find((x) => x.id === current)!.name) : "").toUpperCase());
  const shown = $derived(name ? decode(name, revealK("onb-theme-name", 0.4)) : "");

  function pick(id: string) { app.setConfig((c) => { c.visuals.theme = id; }); revealRestart("onb-theme-name"); }
</script>

<div class="p">
  <div class="card" style:width="{pw}px">
    <span class="tile" style:background={accentTile(0.38, 0.6, accent)}><Icon name="music-2" size={18} color="rgba(255,255,255,.85)" /></span>
    <span class="bars">
      <i class="b1"></i><i class="b2"></i>
      <i class="b3"><i style:width="{prog * 100}%"></i></i>
    </span>
    <span class="disc"><Icon name="pause" size={16} color="rgba(0,0,0,.9)" /></span>
  </div>
  <div class="sw">
    {#each THEMES as th, i (th.id)}
      {@const e = backOut(k(s, 0.15 + 0.05 * i, 0.4))}
      <button class="s" class:on={th.id === current} onclick={() => pick(th.id)} aria-label={t(th.name)}>
        <span style:width="{26 * e}px" style:height="{26 * e}px" style:background={css(th.accent)}></span>
      </button>
    {/each}
  </div>
  <div class="nm mono">{#if shown}[ {shown} ]{/if}</div>
</div>

<style>
  .p { width: 100%; height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 18px; }
  .card { height: 64px; padding: 0 10px; border-radius: 12px; background: rgba(255, 255, 255, .05); display: flex; align-items: center; gap: 12px; }
  .tile { flex: none; width: 44px; height: 44px; border-radius: 8px; display: grid; place-items: center; transition: background-color .3s ease-out; }
  .bars { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; }
  .bars i { display: block; border-radius: 999px; font-style: normal; }
  .b1 { width: 110px; height: 8px; background: rgba(255, 255, 255, .75); }
  .b2 { width: 70px; height: 6px; background: rgba(255, 255, 255, .3); }
  .b3 { width: 100%; height: 3px; background: rgba(255, 255, 255, .1); overflow: hidden; }
  .b3 i { height: 100%; background: var(--accent); }
  .disc { flex: none; width: 36px; height: 36px; border-radius: 50%; background: var(--accent); display: grid; place-items: center; transition: background-color .3s ease-out; }
  .sw { display: flex; align-items: center; gap: 6px; }
  .s { width: 38px; height: 38px; border-radius: 50%; border: 2px solid transparent; display: grid; place-items: center; transition: border-color .2s ease-out; }
  .s.on { border-color: rgba(255, 255, 255, .9); }
  .s span { display: block; border-radius: 50%; }
  .nm { height: 14px; font-size: 11px; color: rgba(255, 255, 255, .55); }
</style>
