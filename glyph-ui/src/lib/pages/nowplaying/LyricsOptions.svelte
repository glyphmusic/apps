<script lang="ts">
  // now_playing.rs options_panel: w 340, max h 440, px 14, py 10, radius 12,
  // hsla 0 0 7 % .97, 1 px white 10 % border, grows in over 180 ms. Rows
  // (min h 34, label sm white 55 % left, value right): Source (provider 13
  // semibold + mono 11 "92% match · line-synced"), Version (find button /
  // "◀ 1/3 ▶" / none), Timing (− mono +0.0s + Reset). Below: a separator
  // and the found versions (mono score, Plain text / Synced, check, 3-line
  // preview) to pick the right one.
  import { onDestroy } from "svelte";
  import Icon from "../../components/Icon.svelte";
  import Button from "../../components/Button.svelte";
  import { fadeEdges } from "../../ui.svelte";
  import { t, tf } from "../../i18n.svelte";
  import { lyr, loadAlternatives, altIndex, pick, cycle, offset, shiftOffset, setOffset } from "./lyrics-state.svelte";
  import type { Lyrics } from "../../api/types";

  let { l }: { l: Lyrics } = $props();
  const off = $derived(offset());
  const idx = $derived(altIndex() ?? 0);
  const n = $derived(Math.max(1, lyr.alts.length));
  const pct = $derived(Math.round((l.score ?? 0) * 100));
  const preview = (a: Lyrics) => a.lines.filter((x) => x.text.trim()).slice(0, 3).map((x) => x.text).join(" / ");
  let spinTick = $state(0);
  $effect(() => {
    if (!lyr.loading) return;
    const id = setInterval(() => { spinTick = (spinTick + 1) % 10; }, 80);
    return () => clearInterval(id);
  });
  const spin = $derived("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"[spinTick]);
  onDestroy(() => { spinTick = 0; });
</script>

<div class="op" onpointerdown={(e) => e.stopPropagation()} onmousedown={(e) => e.stopPropagation()} onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
  <div class="op-in scroll" use:fadeEdges={{ top: 10, bottom: 10, minOverflow: 8 }}>
    <div class="row">
      <span class="lb">{t("lyrics.source")}</span>
      <span class="src">
        <span class="pv">{l.source ?? ""}</span>
        <span class="mt mono trunc">{tf(l.synced ? "lyrics.match_synced" : "lyrics.match_plain", { p: pct })}</span>
      </span>
    </div>
    <div class="row">
      <span class="lb">{t("lyrics.version")}</span>
      {#if lyr.loading}
        <span class="note"><span class="mono">{spin}</span> {t("lyrics.alts.loading")}</span>
      {:else if !lyr.loaded}
        <Button onclick={loadAlternatives}>{t("lyrics.alts.find")}</Button>
      {:else if lyr.alts.length < 2}
        <span class="note">{t("lyrics.alts.none")}</span>
      {:else}
        <span class="val">
          <button class="tb" onclick={() => cycle(-1)}><Icon name="chevron-left" size={13} /></button>
          <span class="mono s12">{idx + 1}/{n}</span>
          <button class="tb" onclick={() => cycle(1)}><Icon name="chevron-right" size={13} /></button>
        </span>
      {/if}
    </div>
    <div class="row">
      <span class="lb">{t("lyrics.timing")}</span>
      <span class="val">
        <button class="tb" onclick={() => shiftOffset(-100)}><Icon name="minus" size={13} /></button>
        <span class="mono s12 off">{off >= 0 ? "+" : "-"}{(Math.abs(off) / 1000).toFixed(1)}s</span>
        <button class="tb" onclick={() => shiftOffset(100)}><Icon name="plus" size={13} /></button>
        {#if off !== 0}<Button kind="text" onclick={() => setOffset(0)}>{t("lyrics.timing.reset")}</Button>{/if}
      </span>
    </div>
    {#if lyr.alts.length >= 2}
      <div class="sep"></div>
      <div class="pick">{t("lyrics.alts.pick")}</div>
      <div class="alts">
        {#each lyr.alts as a, i}
          <button class="alt" class:on={i === idx} onclick={() => pick(i)}>
            <span class="ah">
              <span class="mono">{Math.round((a.score ?? 0) * 100)}%</span>
              <span class="kind">{a.synced ? t("lyrics.synced") : t("lyrics.plain")}</span>
              <span class="fl"></span>
              {#if i === idx}<Icon name="check" size={13} color="var(--accent)" />{/if}
            </span>
            <span class="pr">{preview(a)}</span>
          </button>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .op { width: 340px; max-width: calc(100vw - 32px); border-radius: 10px; background: #1f1f1f; box-shadow: 0 12px 32px rgba(0, 0, 0, .55); overflow: hidden; animation: op-in .18s var(--ease); }
  .op-in { max-height: 440px; padding: 10px 12px; display: flex; flex-direction: column; gap: 2px; }
  @keyframes op-in { from { opacity: 0; transform: translateY(-6px); } to { opacity: 1; transform: none; } }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 12px; min-height: 34px; }
  .lb { flex: none; font-size: 12.5px; color: rgba(255, 255, 255, .55); }
  .src { min-width: 0; display: flex; flex-direction: column; align-items: flex-end; }
  .pv { font-size: 13px; font-weight: 600; }
  .mt { font-size: 11px; color: rgba(255, 255, 255, .5); max-width: 100%; }
  .note { font-size: 12.5px; color: rgba(255, 255, 255, .6); }
  .val { display: flex; align-items: center; gap: 6px; }
  .s12 { font-size: 12px; }
  .off { width: 48px; display: flex; justify-content: center; }
  .tb { width: 26px; height: 26px; display: grid; place-items: center; border-radius: 7px; background: rgba(255, 255, 255, .06); color: rgba(255, 255, 255, .85); transition: background-color .12s; }
  .tb:hover { background: rgba(255, 255, 255, .10); color: #fff; }
  .tb:active { background: rgba(255, 255, 255, .14); }
  .sep { height: 1px; margin: 8px 0; background: rgba(255, 255, 255, .08); }
  .pick { padding: 0 2px 6px; font-size: 12px; line-height: 17px; color: rgba(255, 255, 255, .5); }
  .alts { display: flex; flex-direction: column; gap: 2px; }
  .alt { width: 100%; min-width: 0; overflow: hidden; display: flex; flex-direction: column; gap: 3px; padding: 8px 10px; border-radius: 7px; transition: background-color .12s; }
  .alt:hover { background: rgba(255, 255, 255, .05); }
  .alt.on { background: rgba(255, 255, 255, .08); }
  .ah { display: flex; align-items: center; gap: 6px; font-size: 12px; color: rgba(255, 255, 255, .85); }
  .kind { color: rgba(255, 255, 255, .5); }
  .fl { flex: 1; }
  .pr { width: 100%; min-width: 0; font-size: 12px; line-height: 17px; color: rgba(255, 255, 255, .6); display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
</style>
