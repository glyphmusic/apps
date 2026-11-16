<script lang="ts" module>
  // Card size is kept while the app runs (mini_card_w / mini_card_h).
  const card = $state({ w: 300, h: 384 });
  let cardLoaded = false;
  const pos = $state({ x: -1, y: -1 });
</script>

<script lang="ts">
  // mini.rs: the mini player. In the app it is an always-on-top window; in
  // the browser an in-page floating card (fixed, bottom right, drag it by the
  // cover / title / grip). Two looks, switched in place (the bottom right
  // corner stays): the slim bar (300 x 52, radius 12, hsla 0 0 6 % .98,
  // pl 9 pr 8: cover 34 radius 6 with an expand button on hover, title 13
  // semibold over artist 11 dim, like 15, prev / next 29 with 15 icons, play
  // 28 white circle with an 11 icon, 2 px accent progress line along the
  // bottom edge) and the card (300 x 384 default, 240..1200 x 220..1200,
  // resizable at its corner; padding 12, top 32; square cover as big as the
  // room allows, radius 8; hover shows a black 55 % layer with volume pop-up,
  // shuffle, prev, play 38, next, repeat, share and the seek bar; title 17
  // bold + artist 12.5 + like 18 below; hover bar on top: smaller / grip /
  // close). Cover colour background needs the palette; not available here.
  import { untrack } from "svelte";
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import ToggleButton from "../components/ToggleButton.svelte";
  import PlayGlyph from "../components/PlayGlyph.svelte";
  import Cover from "../components/Cover.svelte";
  import FracBar from "./mini/FracBar.svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { fmtTime } from "../fmt";
  import { press, tip } from "../ui.svelte";
  import { tipFor } from "../i18n.svelte";
  import { inMiniWindow, setWindowSize, startDrag, startResize, syncMiniWindowMode } from "../window";
  import { isTauri, isWeb } from "../api/backend";

  let { pipWindow }: { pipWindow?: Window } = $props();
  const windowed = $derived(inMiniWindow() || !!pipWindow);
  const showTopbar = $derived(!(windowed && isWeb()));
  // The saved card size (config.player.mini_card_w / _h) is the first size.
  if (!cardLoaded) {
    cardLoaded = true;
    const p = app.config.player;
    if (p.mini_card_w && p.mini_card_h) { card.w = Math.min(1200, Math.max(240, p.mini_card_w)); card.h = Math.min(1200, Math.max(220, p.mini_card_h)); }
  }
  /** Writes the card size into the settings (kept over a restart); only when it changed. */
  function saveCard() {
    const w = Math.round(card.w), h = Math.round(card.h), p = app.config.player;
    if (Math.round(p.mini_card_w ?? 0) === w && Math.round(p.mini_card_h ?? 0) === h) return;
    app.setConfig((c) => { c.player.mini_card_w = w; c.player.mini_card_h = h; });
  }
  // In its own window the size changes with the window: save when it settles.
  $effect(() => {
    if (!(windowed && big)) return;
    void [card.w, card.h];
    const id = setTimeout(saveCard, 500);
    return () => clearTimeout(id);
  });

  const MINI_W = 300, MINI_H = 52, CARD_PAD = 12, TOP_INSET = 4, TOP_BTN = 24, CARD_TOP = TOP_INSET * 2 + TOP_BTN, INFO_H = 40, INFO_GAP = 10;
  const big = $derived(app.config.player.mini_large);
  const cardTop = $derived(showTopbar ? CARD_TOP : CARD_PAD);
  const cur = $derived(app.player.current);
  const dur = $derived(app.player.duration);
  const frac = $derived(dur ? app.position / dur : 0);
  const W = $derived(big ? card.w : MINI_W);
  const H = $derived(big ? card.h : MINI_H);
  const liked = $derived(cur ? app.isLiked(cur) : false);
  let vw = $state(untrack(() => pipWindow?.innerWidth) || (typeof window !== "undefined" ? window.innerWidth : 300));
  let vh = $state(untrack(() => pipWindow?.innerHeight) || (typeof window !== "undefined" ? window.innerHeight : 384));
  let volPinned = $state(false), volHover = $state(false);
  let resizing = $state(false);
  const volOpen = $derived(volPinned || volHover);

  function resizePip(target: Window, w: number, h: number) {
    try {
      const diffW = target.outerWidth > target.innerWidth ? target.outerWidth - target.innerWidth : 0;
      const diffH = target.outerHeight > target.innerHeight ? target.outerHeight - target.innerHeight : 0;
      target.resizeTo(Math.round(w + diffW), Math.round(h + diffH));
    } catch {}
  }

  $effect(() => {
    const win = pipWindow ?? (typeof window !== "undefined" ? window : null);
    if (!win) return;
    let raf = 0;
    const syncSize = () => {
      raf = 0;
      const nw = win.innerWidth || vw;
      const nh = win.innerHeight || vh;
      vw = nw;
      vh = nh;
      // Record user-driven window resizes in card mode without echoing setSize back to the OS.
      if (windowed && big && nw >= 240 && nh >= 200) {
        card.w = Math.min(1200, Math.max(240, nw));
        card.h = Math.min(1200, Math.max(220, nh));
      }
    };
    const onResize = () => {
      if (!raf) raf = requestAnimationFrame(syncSize);
    };
    syncSize();
    win.addEventListener("resize", onResize);
    return () => {
      cancelAnimationFrame(raf);
      win.removeEventListener("resize", onResize);
    };
  });

  // First place: bottom right, 24 px from the edges.
  $effect.pre(() => { if (pos.x < 0) { pos.x = Math.max(8, vw - MINI_W - 24); pos.y = Math.max(8, vh - (big ? card.h : MINI_H) - 24); } });
  // Bar <-> card in place: keep the bottom right corner.
  let prev = { w: 0, h: 0 };
  let prevSet = false;
  $effect(() => {
    const w = W, h = H;
    if (!prevSet) { prev = { w, h }; prevSet = true; return; }
    if (!resizing) { pos.x = Math.max(0, pos.x + prev.w - w); pos.y = Math.max(0, pos.y + prev.h - h); }
    prev = { w, h };
  });
  // Tauri / Document PiP: the mini player is a window of its own and fills it.
  const left = $derived(windowed ? 0 : Math.min(Math.max(0, pos.x), Math.max(0, vw - W)));
  const top = $derived(windowed ? 0 : Math.min(Math.max(0, pos.y), Math.max(0, vh - H)));
  // Only resize the native/PiP window when switching between bar and card modes.
  let prevBig: boolean | undefined;
  $effect(() => {
    const b = big;
    if (!windowed) { prevBig = b; return; }
    if (prevBig === b) return;
    const initial = prevBig === undefined;
    prevBig = b;
    if (initial) return;
    const w = b ? untrack(() => card.w) : MINI_W;
    const h = b ? untrack(() => card.h) : MINI_H;
    if (pipWindow) resizePip(pipWindow, w, h);
    else void syncMiniWindowMode(b, w, h);
  });

  /** Drag the window by this element (not by its buttons). */
  function grab(e: PointerEvent) {
    if (e.button !== 0) return;
    if (windowed) {
      if (isTauri()) void startDrag();
      return;
    }
    const dx = e.clientX - left, dy = e.clientY - top;
    const move = (ev: PointerEvent) => { pos.x = ev.clientX - dx; pos.y = ev.clientY - dy; };
    const up = () => { removeEventListener("pointermove", move); removeEventListener("pointerup", up); };
    addEventListener("pointermove", move);
    addEventListener("pointerup", up);
  }
  function resize(e: PointerEvent) {
    e.stopPropagation(); e.preventDefault();
    if (pipWindow) return;
    if (isTauri() && windowed) { startResize(); return; }
    const x0 = e.clientX, y0 = e.clientY, w0 = card.w, h0 = card.h;
    resizing = true;
    const move = (ev: PointerEvent) => {
      card.w = Math.min(1200, Math.max(240, w0 + ev.clientX - x0));
      card.h = Math.min(1200, Math.max(220, h0 + ev.clientY - y0));
      if (windowed) void setWindowSize(card.w, card.h);
    };
    const up = () => { resizing = false; saveCard(); removeEventListener("pointermove", move); removeEventListener("pointerup", up); };
    addEventListener("pointermove", move);
    addEventListener("pointerup", up);
  }
  const setBig = (b: boolean) => {
    app.setConfig((c) => { c.player.mini_large = b; });
    if (pipWindow) resizePip(pipWindow, b ? card.w : MINI_W, b ? card.h : MINI_H);
  };
  const closeMini = () => {
    app.miniShown = false;
    if (pipWindow && !pipWindow.closed) {
      try { pipWindow.close(); } catch {}
    }
  };
  const liveW = $derived(windowed ? vw : card.w);
  const liveH = $derived(windowed ? vh : card.h);
  const side = $derived(Math.max(60, Math.min(liveW - CARD_PAD * 2, liveH - cardTop - CARD_PAD - INFO_GAP - INFO_H)));
  const wheelVol = $derived(app.config.ui.wheel_volume_pct / 100);
  const wheelSeek = $derived(dur ? app.config.ui.wheel_seek_s / dur : 0);
  const title = $derived(cur?.title ?? t("mini.idle"));
</script>

{#if !big}
  <!-- the slim bar -->
  <div class="mini bar" class:pip={!!pipWindow} class:win={windowed} style:left="{left}px" style:top="{top}px" style:width="{windowed ? '100%' : `${MINI_W}px`}" style:height="{windowed ? '100%' : `${MINI_H}px`}">
    <div class="grp">
      <div class="art" onpointerdown={grab} role="presentation">
        {#if cur}<Cover track={cur} size={34} radius={6} />{:else}<span class="ph"><Icon name="music-2" size={16} color="var(--faint)" /></span>{/if}
        <button class="exp" aria-label={t("ctl.mini.card")} use:tip={t("ctl.mini.card")} onpointerdown={(e) => e.stopPropagation()} onclick={() => setBig(true)}><Icon name="maximize-2" size={14} color="#fff" /></button>
      </div>
      <div class="tx">
        <div class="t trunc" onpointerdown={grab} role="presentation">{title}</div>
        {#if cur}<button class="a trunc" onclick={() => app.openArtistPage(cur.artist)}>{cur.artist}</button>{/if}
      </div>
    </div>
    {#if cur}<ToggleButton name="heart" size={15} on={liked} accent="var(--accent)" onclick={() => app.toggleLike(cur)} />{/if}
    <span use:press><IconButton name="skip-back" size={15} box={29} color="var(--text)" onclick={() => app.previous()} /></span>
    <button class="play sm" use:press aria-label={app.playing ? t("common.pause") : t("common.play")} onclick={() => app.togglePlay()}>
      <PlayGlyph loading={app.loading} playing={app.playing} size={11} button={28} />
    </button>
    <span use:press><IconButton name="skip-forward" size={15} box={29} color="var(--text)" onclick={() => app.next()} /></span>
    <div class="prog" style:width="{100 * Math.min(1, Math.max(0, frac))}%"></div>
  </div>
{:else}
  <!-- the card -->
  <div class="mini cardw" class:pip={!!pipWindow} class:win={windowed} class:resizing style:left="{left}px" style:top="{top}px" style:width="{windowed ? '100%' : `${card.w}px`}" style:height="{windowed ? '100%' : `${card.h}px`}" style:padding="{cardTop}px {CARD_PAD}px {CARD_PAD}px" style:gap="{INFO_GAP}px">
    {#if showTopbar}
      <div class="topbar" style:height="{CARD_TOP}px" style:padding="{TOP_INSET}px">
        <button class="tbtn" aria-label={tipFor("minimize-2")} use:tip={tipFor("minimize-2")} onclick={() => setBig(false)}><Icon name="minimize-2" size={14} /></button>
        <div class="gripbar" onpointerdown={grab} role="presentation"><Icon name="grip-horizontal" size={16} color="var(--dim)" /></div>
        <button class="tbtn" aria-label={tipFor("x")} use:tip={tipFor("x")} onclick={closeMini}><Icon name="x" size={14} /></button>
      </div>
    {/if}

    <div class="cover" class:pinned={volPinned} style:width="{side}px" style:height="{side}px" onpointerleave={() => (volPinned = false)} role="presentation">
      {#if cur}<Cover track={cur} size={side} radius={8} />{:else}<div class="cph"><Icon name="music-2" size={48} color="var(--faint)" /></div>{/if}
      <div class="ov">
        <div class="fl"></div>
        <div class="ctl">
          <div class="volw" onpointerenter={() => (volHover = true)} onpointerleave={() => (volHover = false)} role="presentation">
            <IconButton name={app.volume <= 0.001 ? "volume-x" : "volume-2"} size={15} color="#fff" onclick={() => (volPinned = !volPinned)} />
            {#if volOpen}
              <div class="vpop"><FracBar vertical value={app.volume} track="rgba(255,255,255,.2)" fill="var(--accent)" wheelStep={wheelVol} onpick={(f) => app.setVolume(f)} /></div>
            {/if}
          </div>
          <IconButton name="shuffle" size={15} color={app.player.shuffle ? "var(--accent)" : "#fff"} onclick={() => app.toggleShuffle()} />
          <span use:press><IconButton name="skip-back" size={16} color="#fff" onclick={() => app.previous()} /></span>
          <button class="play lg" use:press aria-label={app.playing ? t("common.pause") : t("common.play")} onclick={() => app.togglePlay()}>
            <PlayGlyph loading={app.loading} playing={app.playing} size={14} button={38} />
          </button>
          <span use:press><IconButton name="skip-forward" size={16} color="#fff" onclick={() => app.next()} /></span>
          <IconButton name={app.player.repeat === "one" ? "repeat-1" : "repeat"} size={15} color={app.player.repeat !== "off" ? "var(--accent)" : "#fff"} onclick={() => app.cycleRepeat()} />
          <IconButton name="link-2" size={15} color="#fff" onclick={() => cur && app.share({ track: cur })} />
        </div>
        <div class="fl"></div>
        <div class="prog2">
          <div class="times mono"><span>{fmtTime(app.position)}</span><span>{dur != null ? fmtTime(dur) : "-:--"}</span></div>
          <FracBar value={frac} track="rgba(255,255,255,.25)" fill="#fff" wheelStep={wheelSeek} onpick={(f) => dur && app.seek(f * dur)} />
        </div>
      </div>
    </div>

    <div class="info" style:width="{side}px" style:height="{INFO_H}px">
      <div class="itx">
        <div class="it trunc" onpointerdown={grab} role="presentation">{title}</div>
        {#if cur}<button class="ia trunc" onclick={() => app.openArtistPage(cur.artist)}>{cur.artist}</button>{/if}
      </div>
      {#if cur}<ToggleButton name="heart" size={18} on={liked} accent="var(--accent)" onclick={() => app.toggleLike(cur)} />{/if}
    </div>
    {#if !pipWindow}<div class="rz" onpointerdown={resize} role="presentation"></div>{/if}
  </div>
{/if}

<style>
  .mini { position: fixed; z-index: 80; border-radius: 12px; overflow: hidden; font-family: var(--font); color: var(--text); animation: rise-in .25s var(--ease); --ink: 242, 242, 242; }
  .mini.win { animation: none; }
  .mini.pip { border-radius: 0; }
  /* ---- bar ---- */
  .bar { display: flex; align-items: center; gap: 2px; padding: 0 8px 0 9px; background: hsl(0 0% 6% / .98); }
  .grp { flex: 1; min-width: 0; height: 100%; display: flex; align-items: center; gap: 10px; }
  .art { position: relative; flex: none; width: 34px; height: 34px; cursor: grab; }
  .ph { width: 34px; height: 34px; border-radius: 6px; background: var(--card); display: grid; place-items: center; }
  .exp { position: absolute; inset: 0; border-radius: 6px; background: rgba(0, 0, 0, .6); display: grid; place-items: center; opacity: 0; transition: opacity .15s; }
  .art:hover .exp { opacity: 1; }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .t { font-size: 13px; line-height: 16px; font-weight: 600; cursor: grab; }
  .a { align-self: flex-start; max-width: 100%; font-size: 11px; line-height: 13px; color: var(--dim); }
  .a:hover, .ia:hover { color: var(--text); text-decoration: underline; }
  .play { flex: none; display: grid; place-items: center; border-radius: 50%; background: #fff; transition: opacity .12s; }
  .play:hover { opacity: .88; }
  .play.sm { width: 28px; height: 28px; }
  .play.lg { width: 38px; height: 38px; }
  .play:global(.pressing) { animation: press .25s var(--ease); }
  .prog { position: absolute; left: 0; bottom: 0; height: 2px; background: var(--accent); }
  /* ---- card ---- */
  .cardw { display: flex; flex-direction: column; align-items: center; justify-content: center; background: hsl(0 0% 7% / .99); }
  .cardw.resizing { transition: none; }
  .topbar { position: absolute; top: 0; left: 0; right: 0; display: flex; align-items: flex-start; opacity: 0; transition: opacity .2s ease-out; z-index: 3; }
  .cardw:hover .topbar { opacity: 1; }
  .tbtn { width: 24px; height: 24px; display: grid; place-items: center; border-radius: 8px; color: var(--dim); transition: background-color .12s; }
  .tbtn:hover { background: var(--hover); }
  .gripbar { flex: 1; height: 100%; display: flex; align-items: center; justify-content: center; cursor: grab; }
  .cover { position: relative; flex: none; border-radius: 8px; overflow: hidden; background: var(--card); }
  .cph { width: 100%; height: 100%; display: grid; place-items: center; }
  .ov { position: absolute; inset: 0; border-radius: 8px; background: rgba(0, 0, 0, .55); display: flex; flex-direction: column; opacity: 0; transition: opacity .2s ease-out; }
  .cover:hover .ov, .cover.pinned .ov { opacity: 1; }
  .fl { flex: 1; }
  .ctl { display: flex; align-items: center; justify-content: center; gap: 2px; }
  .volw { position: relative; }
  .vpop { position: absolute; bottom: 29px; left: 1px; width: 28px; height: 108px; padding: 10px 0 14px; display: flex; justify-content: center; z-index: 4; animation: fade-in .12s ease-out; }
  .vpop::before { content: ""; position: absolute; inset: 0 0 4px; border-radius: 8px; background: hsl(0 0% 10% / .98); }
  .vpop :global(.hit) { position: relative; }
  .prog2 { display: flex; flex-direction: column; gap: 4px; padding: 0 12px 12px; }
  .times { display: flex; justify-content: space-between; font-size: 11px; color: rgba(255, 255, 255, .8); }
  .info { flex: none; display: flex; align-items: center; gap: 10px; }
  .itx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .it { font-size: 17px; line-height: 22px; font-weight: 700; cursor: grab; }
  .ia { align-self: flex-start; max-width: 100%; font-size: 12.5px; color: var(--dim); }
  .rz { position: absolute; right: 4px; bottom: 4px; width: 18px; height: 18px; cursor: nwse-resize; z-index: 3; }
</style>
