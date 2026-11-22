<script lang="ts">
  // player_bar.rs: 80 px, radius 12, px 12, hsla(0 0 6% / .94). Left 300 px
  // (cover 48, title / artist + source badge, like), transport (shuffle, prev,
  // play, next, repeat) + seek row centred on the whole bar, right pill
  // (connect, volume, sleep, mini, queue). Right click on a control opens its menu.
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import ToggleButton from "../components/ToggleButton.svelte";
  import ThinBar from "../components/ThinBar.svelte";
  import Cover from "../components/Cover.svelte";
  import SongTitle from "../components/SongTitle.svelte";
  import Badges from "../components/Badges.svelte";
  import Decode from "../components/Decode.svelte";
  import PlayGlyph from "../components/PlayGlyph.svelte";
  import StatusStrip from "./StatusStrip.svelte";
  import { app, type ControlMenu } from "../state.svelte";
  import { saveMenu } from "../pages/lists/actions";
  import { jam } from "../jam.svelte";
  import { onWeb } from "../web";
  import { toggleWebMini } from "../web-mini.svelte";
  import { t } from "../i18n.svelte";
  import { fmtTime } from "../fmt";
  import { press } from "../ui.svelte";
  import { rating, trackKey } from "../api/types";

  let barW = $state(1200);
  let volHover = $state(false);
  const SEEK_W = $derived(Math.min(340, Math.max(160, barW - 460)));
  const folded = $derived(barW < 1072);
  const cur = $derived(app.player.current);
  const pos = $derived(app.position);
  const dur = $derived(app.player.duration);
  const frac = $derived(dur ? pos / dur : 0);
  const key = $derived(cur ? trackKey(cur) : "");
  const elsewhere = $derived(app.devices.some((d) => d.active && d.kind !== "this") || app.jamOpen);

  function menu(m: ControlMenu) {
    return (e: MouseEvent) => { e.preventDefault(); e.stopPropagation(); app.closeMenus(); app.controlMenu = { menu: m, x: e.clientX, y: e.clientY }; };
  }
  function wheelSeek(n: number) { const s = app.config.ui.wheel_seek_s; if (s && dur) app.seekRelative(n * s); }
  function wheelVol(n: number) {
    const s = app.config.ui.wheel_volume_pct / 100;
    if (!s) return;
    app.setVolume(Math.min(1, Math.max(0, Math.round((app.volume + n * s) / s) * s)));
  }
  // A-B loop ends: drag along the seek bar.
  let seekEl: HTMLDivElement | undefined = $state();
  function dragLoop(start: boolean) {
    return (e: PointerEvent) => {
      e.stopPropagation();
      const el = e.currentTarget as HTMLElement;
      el.setPointerCapture(e.pointerId);
      const move = (ev: PointerEvent) => {
        if (!seekEl || !dur) return;
        const r = seekEl.getBoundingClientRect(), f = Math.min(1, Math.max(0, (ev.clientX - r.left) / r.width)) * dur;
        if (start) app.setLoop(Math.min(f, app.player.loop_b ?? dur), app.player.loop_b ?? null);
        else app.setLoop(app.player.loop_a ?? null, Math.max(f, app.player.loop_a ?? 0));
      };
      el.addEventListener("pointermove", move);
      el.addEventListener("pointerup", () => el.removeEventListener("pointermove", move), { once: true });
    };
  }
  const la = $derived(dur && app.player.loop_a != null ? (app.player.loop_a / dur) * SEEK_W : null);
  const lb = $derived(dur && app.player.loop_b != null ? (app.player.loop_b / dur) * SEEK_W : null);
</script>

<div class="bar" bind:clientWidth={barW} data-part="player-bar">
  {#if jam.bursts.length}
    <div class="bursts" aria-hidden="true">
      {#each jam.bursts as b (b.id)}
        <span
          class="burst"
          class:ac={b.accent}
          style="left:{b.x}%;--dx:{b.dx}px;--rise:{b.rise}px;--r0:{b.rot0}deg;--r1:{b.rot1}deg;--s:{b.scale};animation-duration:{b.dur}ms;animation-delay:{b.delay}ms"
        ><Icon name={b.icon} size={b.size} /></span>
      {/each}
    </div>
  {/if}
  <!-- left: the song -->
  <div class="side l">
    {#if cur}
      <div class="now">
        <button class="open" onclick={() => app.navigate("now_playing")}>
          <Cover track={cur} size={48} radius={6} />
          <span class="nt">
            <span class="t"><SongTitle rating={rating(cur)}><Decode text={cur.title} key="bar-meta:{key}" /></SongTitle></span>
            <span class="a">
              <span class="trunc artist" role="link" tabindex="-1" onclick={(e) => { e.stopPropagation(); app.openArtistPage(cur.artist); }} onkeydown={() => {}}><Decode text={cur.artist} key="bar-meta:{key}" /></span>
              {#if app.player.source}<Badges source={app.player.source} />{/if}
              {#if app.player.preview}<span class="pv mono">[PREVIEW 0:30]</span>{/if}
            </span>
          </span>
        </button>
        <ToggleButton name="heart" size={16} on={app.isLiked(cur)} accent="var(--accent)" onclick={() => app.toggleLike(cur)} oncontextmenu={saveMenu(cur)} />
      </div>
    {:else}
      <div class="now">
        <span class="ph"><Icon name="music-2" size={18} color="var(--faint)" /></span>
        <span class="nt"><span class="t">{t("player.idle.title")}</span><span class="a trunc faint">{t("player.idle.sub")}</span></span>
      </div>
    {/if}
  </div>

  <!-- centre: transport + seek, centred on the bar itself -->
  <div class="center">
    <div class="tr">
      <ToggleButton name="shuffle" on={app.player.shuffle || app.controlMenu?.menu === "shuffle"} accent="var(--accent)" onclick={() => app.toggleShuffle()} oncontextmenu={menu("shuffle")} />
      <span use:press><IconButton name="skip-back" size={17} box={31} color="var(--text)" onclick={() => app.previous()} oncontextmenu={menu("back")} /></span>
      <button class="play" use:press aria-label={app.playing ? t("common.pause") : t("common.play")} onclick={() => app.togglePlay()}>
        <i><PlayGlyph loading={app.loading} playing={app.playing} size={15} button={34} /></i>
      </button>
      <span use:press><IconButton name="skip-forward" size={17} box={31} color="var(--text)" onclick={() => app.next()} oncontextmenu={menu("forward")} /></span>
      <ToggleButton name={app.player.repeat === "one" ? "repeat-1" : "repeat"} on={app.player.repeat !== "off" || app.controlMenu?.menu === "repeat"} accent="var(--accent)" onclick={() => app.cycleRepeat()} oncontextmenu={menu("repeat")} />
    </div>
    <div class="seekrow">
      <span class="tm r mono">{fmtTime(pos)}</span>
      <div class="seek" bind:this={seekEl}>
        <ThinBar width={SEEK_W} value={frac} live={false} onpick={(f) => dur && app.seek(f * dur)} label={dur ? (f) => fmtTime(f * dur) : undefined} onwheelstep={wheelSeek} />
        {#if la != null && lb != null}<span class="loop" style:left="{la}px" style:width="{Math.max(0, lb - la)}px"></span>{/if}
        {#if la != null}<span class="lh" style:left="{la - 7}px" onpointerdown={dragLoop(true)} role="slider" aria-valuenow={la} tabindex="-1"><i></i></span>{/if}
        {#if lb != null}<span class="lh" style:left="{lb - 7}px" onpointerdown={dragLoop(false)} role="slider" aria-valuenow={lb} tabindex="-1"><i></i></span>{/if}
      </div>
      <span class="tm mono">{dur != null ? fmtTime(dur) : "-:--"}</span>
    </div>
  </div>

  <!-- right: the controls pill -->
  <div class="side r">
    <div class="pill">
      <ToggleButton name="tv" label={t("np.mode.video")} on={app.view === "now_playing" && app.videoMode} accent="var(--accent)" onclick={() => {
        if (!cur) return;
        if (app.view === "now_playing" && app.videoMode) { app.videoMode = false; }
        else { app.lyricsOnly = false; app.videoMode = true; app.navigate("now_playing"); }
      }} />
      <ToggleButton name="monitor-speaker" on={app.showDevices || elsewhere} accent="var(--accent)" onclick={() => { app.showDevices = !app.showDevices; if (app.showDevices) app.showQueue = false; }} />
      {#if folded}
        <div class="vfold" role="group" onpointerenter={() => (volHover = true)} onpointerleave={() => (volHover = false)}>
          <IconButton name={app.volume <= 0.001 ? "volume-x" : "volume-2"} size={15} color={app.controlMenu?.menu === "volume" ? "var(--accent)" : undefined} notip onclick={() => app.toggleMute()} oncontextmenu={menu("volume")} />
          {#if volHover}
            <div class="vpop"><ThinBar vertical width={84} value={app.volume} onpick={(f) => app.setVolume(f)} onwheelstep={wheelVol} /></div>
          {/if}
        </div>
      {:else}
        <div class="vgroup">
          <IconButton name={app.volume <= 0.001 ? "volume-x" : "volume-2"} size={15} color={app.controlMenu?.menu === "volume" ? "var(--accent)" : undefined} onclick={() => app.toggleMute()} oncontextmenu={menu("volume")} />
          <ThinBar width={80} value={app.volume} onpick={(f) => app.setVolume(f)} label={(f) => `${Math.round(f * 100)}%`} onwheelstep={wheelVol} />
        </div>
      {/if}
      <ToggleButton name="timer" on={app.sleep != null || app.controlMenu?.menu === "sleep"} accent="var(--accent)" onclick={menu("sleep")} oncontextmenu={menu("sleep")} />
      <ToggleButton name="picture-in-picture-2" on={app.miniShown || app.controlMenu?.menu === "mini"} accent="var(--accent)" onclick={() => (onWeb() ? toggleWebMini(app) : (app.miniShown = !app.miniShown))} oncontextmenu={menu("mini")} />
      <ToggleButton name="list-music" on={app.showQueue} accent="var(--accent)" onclick={() => { app.showQueue = !app.showQueue; if (app.showQueue) app.showDevices = false; }} />
    </div>
    <StatusStrip />
  </div>
</div>

<style>
  .bar { position: relative; flex: none; height: var(--player-h); width: 100%; padding: 0 12px; display: grid; grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr); align-items: center; gap: 12px; border-radius: 12px; background: rgba(15, 15, 15, .94); --ink: 242, 242, 242; }
  .center { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 6px; min-width: 0; }
  .tr { display: flex; align-items: center; gap: 14px; }
  .play { width: 34px; height: 34px; display: grid; place-items: center; }
  .play i { width: 34px; height: 34px; border-radius: 50%; background: #fff; display: grid; place-items: center; transition: opacity .12s, transform .12s; }
  .play i:hover { opacity: .88; }
  .play:global(.pressing) i { animation: press .25s var(--ease); }
  .seekrow { display: flex; align-items: center; gap: 8px; }
  .tm { flex: none; width: 38px; font-size: 11px; color: var(--faint); }
  .tm.r { text-align: right; }
  .seek { position: relative; }
  .loop { position: absolute; top: 0; bottom: 0; background: rgba(var(--accent-rgb, 255, 95, 210), .25); pointer-events: none; }
  .lh { position: absolute; top: -8px; width: 14px; height: 20px; display: grid; place-items: center; cursor: ew-resize; }
  .lh i { width: 2px; height: 12px; border-radius: 2px; background: var(--accent); transition: width .12s, height .12s; }
  .lh:hover i { width: 4px; height: 18px; }
  .side { position: relative; width: 100%; max-width: 300px; min-width: 0; display: flex; }
  .side.l { justify-self: start; }
  .side.r { justify-self: end; flex-direction: column; align-items: flex-end; justify-content: center; gap: 5px; }
  .now { display: flex; align-items: center; gap: 10px; min-width: 0; max-width: 100%; }
  .open { display: flex; align-items: center; gap: 12px; min-width: 0; flex: 1; }
  .ph { flex: none; width: 48px; height: 48px; border-radius: 6px; background: rgba(255, 255, 255, .06); display: grid; place-items: center; }
  .nt { display: flex; flex-direction: column; justify-content: center; gap: 2px; min-width: 0; }
  .t { font-size: var(--md); font-weight: 500; min-width: 0; }
  .a { display: flex; align-items: center; gap: 6px; min-width: 0; font-size: var(--sm); color: var(--dim); }
  .artist:hover { color: var(--text); text-decoration: underline; cursor: pointer; }
  .pv { flex: none; font-size: var(--xs); color: var(--warn); }
  .pill { flex: none; display: flex; align-items: center; gap: 2px; height: 36px; padding: 0 4px; border-radius: 10px; background: rgba(255, 255, 255, .04); }
  .vgroup { display: flex; align-items: center; gap: 6px; padding: 0 4px; }
  .vfold { position: relative; }
  .vpop { position: absolute; bottom: 29px; left: 1px; width: 28px; height: 108px; padding: 10px 0 18px; display: flex; justify-content: center; z-index: 30; animation: fade-in .12s ease-out; }
  .vpop::before { content: ""; position: absolute; inset: 0 0 4px; border-radius: 8px; background: rgba(26, 26, 26, .98); }
  .vpop :global(.thin) { position: relative; }
  .bursts { position: absolute; left: 0; right: 0; bottom: calc(100% + 4px); height: 0; pointer-events: none; overflow: visible; z-index: 25; }
  .burst { position: absolute; bottom: 0; display: inline-flex; color: rgba(255, 255, 255, .92); user-select: none; will-change: transform, opacity; animation: jam-float 1.6s cubic-bezier(.2, .8, .25, 1) both; }
  .burst.ac { color: var(--accent); }
  @keyframes jam-float {
    0% { opacity: 0; transform: translate3d(-50%, 8px, 0) rotate(var(--r0)) scale(calc(var(--s) * .72)); }
    14% { opacity: 1; }
    74% { opacity: 1; }
    100% { opacity: 0; transform: translate3d(calc(-50% + var(--dx)), calc(var(--rise) * -1), 0) rotate(var(--r1)) scale(var(--s)); }
  }
</style>
