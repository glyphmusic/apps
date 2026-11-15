<script lang="ts">
  // compact/now_playing.rs lyrics_view: full screen on the accent tile;
  // header (chevron, title / artist), the synced lyrics following the song
  // (tap a line to jump there), seek and transport at the bottom. Pull down
  // to close.
  import ThinBar from "../components/ThinBar.svelte";
  import IconButton from "../components/IconButton.svelte";
  import PlayGlyph from "../components/PlayGlyph.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import IconAction from "./IconAction.svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { fmtTime } from "../fmt";
  import { accentTile, themeColors } from "../theme";
  import { fadeEdges } from "../ui.svelte";
  import { phone, drag, sheetIn, sheetOut, shouldClose } from "./cstate.svelte";
  let pull = $state(0);
  let box: HTMLDivElement | undefined = $state();
  let userScroll = 0;
  const cur = $derived(app.player.current!);
  const ly = $derived(app.lyrics.lyrics);
  const dur = $derived(app.player.duration ?? 0);
  const active = $derived.by(() => {
    if (!ly?.synced) return -1;
    const ms = app.position * 1000 + (ly.offset_ms ?? 0);
    let a = -1;
    for (let i = 0; i < ly.lines.length; i++) if ((ly.lines[i].time_ms ?? 0) <= ms) a = i; else break;
    return a;
  });
  $effect(() => {
    const i = active;
    if (i < 0 || !box || performance.now() - userScroll < 3000) return;
    const el = box.children[i] as HTMLElement | undefined;
    el?.scrollIntoView({ block: "center", behavior: "smooth" });
  });
  const close = () => (phone.lyricsOpen = false);
</script>

<div class="lv" style:background={accentTile(0.24, 0.5, themeColors(app.config.visuals.theme).accent)} style:transform="translateY({pull}px)" style:border-radius={pull > 0 ? `${Math.min(22, 10 + pull * 0.3).toFixed(1)}px ${Math.min(22, 10 + pull * 0.3).toFixed(1)}px 0 0` : undefined} class:settle={pull === 0}
  in:sheetIn|global={{ duration: 280 }} out:sheetOut|global={{ duration: 230, radius: true }}
  onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")}>
  <div class="hd" use:drag={{ axis: "y", move: (_, dy) => (pull = Math.max(0, dy)), end: (_, dy, _vx, vy) => { if (shouldClose(dy, vy)) { close(); return; } pull = 0; } }}>
    <IconAction name="chevron-down" onclick={close} />
    <div class="c"><b class="trunc">{cur.title}</b><span class="trunc">{cur.artist}</span></div>
    <span class="sp"></span>
  </div>
  <div class="scroll lines" bind:this={box} use:fadeEdges={{ top: 32, bottom: 32 }} onwheel={() => (userScroll = performance.now())} ontouchmove={() => (userScroll = performance.now())} role="presentation">
    {#if app.lyrics.loading}
      {#each [0.62, 0.48, 0.7, 0.4, 0.56, 0.66, 0.44] as w}<div class="sk"><Skeleton w="{w * 100}%" h={24} radius={6} /></div>{/each}
    {:else if ly?.lines.length}
      {#each ly.lines as l, i}
        <button class="l" class:on={i === active} class:past={ly.synced && i < active} onclick={() => l.time_ms != null && app.seek(l.time_ms / 1000)}>{l.text.trim() || "♪"}</button>
      {/each}
    {:else}
      <p class="none">{t("lyrics.none.title")}</p>
    {/if}
  </div>
  <div class="ctl">
    <ThinBar width="100%" value={dur ? app.position / dur : 0} live={false} onpick={(f) => dur && app.seek(f * dur)} />
    <div class="times mono"><span>{fmtTime(app.position)}</span><span>{dur ? fmtTime(dur) : ""}</span></div>
    <div class="tr">
      <IconButton name="skip-back" size={24} box={48} color="var(--text)" notip onclick={() => app.previous()} />
      <button class="play" onclick={() => app.togglePlay()} aria-label={t("common.play")}><PlayGlyph loading={app.loading} playing={app.playing} size={21} button={56} /></button>
      <IconButton name="skip-forward" size={24} box={48} color="var(--text)" notip onclick={() => app.next()} />
    </div>
  </div>
</div>

<style>
  .lv { position: fixed; inset: 0; z-index: 45; padding: var(--safe-top, env(safe-area-inset-top)) 0 calc(var(--safe-bottom, env(safe-area-inset-bottom)) + 16px); display: flex; flex-direction: column; overflow: hidden; }
  .lv.settle { transition: transform .24s var(--ease), border-radius .22s var(--ease); }
  .hd { flex: none; height: 52px; padding: 0 4px; display: flex; align-items: center; gap: 4px; touch-action: none; }
  .c { flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: center; }
  .c b { max-width: 100%; font-size: 14px; color: #fff; }
  .c span { max-width: 100%; font-size: 12.5px; color: rgba(255, 255, 255, .7); }
  .sp { width: 40px; flex: none; }
  .lines { flex: 1; min-height: 0; padding: 12px 20px 40vh; display: flex; flex-direction: column; gap: 14px; }
  .l { font-size: 24px; line-height: 1.25; font-weight: 700; color: rgba(255, 255, 255, .45); text-align: left; transition: color .3s; }
  .l.past { color: rgba(255, 255, 255, .7); }
  .l.on { color: #fff; }
  .sk { height: 24px; }
  .none { color: rgba(255, 255, 255, .7); }
  .ctl { flex: none; padding: 8px 24px 0; display: flex; flex-direction: column; gap: 6px; }
  .times { display: flex; justify-content: space-between; font-size: 11.5px; color: rgba(255, 255, 255, .6); }
  .tr { display: flex; align-items: center; justify-content: center; gap: 28px; }
  .play { width: 56px; height: 56px; border-radius: 50%; background: #fff; display: grid; place-items: center; }
</style>
