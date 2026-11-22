<script lang="ts">
  // compact/now_playing.rs bottom_sheet: queue or Connect as a sheet from
  // the bottom (86 % of the height), a grab handle, dim behind; pull down or
  // tap outside to close. Queue sheet supports edit mode (multi-select
  // move-to-top / remove), drag-to-reorder handles, pinned current song with
  // play/pause, and bottom Shuffle / Repeat / Sleep Timer bar. Swipe a row
  // right to add the song to the queue again, left to take it out; songs
  // added to the queue by hand carry an accent queue mark.
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Icon from "../components/Icon.svelte";
  import Cover from "../components/Cover.svelte";
  import SongTitle from "../components/SongTitle.svelte";
  import IconButton from "../components/IconButton.svelte";
  import PlayGlyph from "../components/PlayGlyph.svelte";
  import { app } from "../state.svelte";
  import { thisIcon, thisName } from "../connect";
  import { t, tf } from "../i18n.svelte";
  import { rating, type Track } from "../api/types";
  import { jam } from "../jam.svelte";
  import { fadeEdges } from "../ui.svelte";
  import { SLEEP_MINUTES, setSleep, sleepCountdown, sleepLabel } from "../overlays/sleep.svelte";
  import ConnectView from "../shell/ConnectView.svelte";
  import { drag, sheetIn, sheetOut, shouldClose, SWIPE_ACT } from "./cstate.svelte";

  let pull = $state(0);
  let editing = $state(false);
  let selected = $state<number[]>([]);
  let timerOpen = $state(false);
  let listEl: HTMLDivElement | undefined = $state();
  let dragRow = $state<{ from: number; slot: number; dy: number; h: number } | null>(null);
  let rowDragActive = false;
  let queueCap = $state(30);
  /** The row being swiped sideways (its key) and how far. */
  let swipe = $state<{ key: string; dx: number } | null>(null);
  /** Rows swiped out to the left, gone once the new queue arrives. */
  let leaving = $state<string[]>([]);
  $effect(() => { void app.queue; leaving = []; });

  const kind = $derived(app.showDevices ? "devices" : "queue");
  const close = () => { app.showQueue = false; app.showDevices = false; app.jamOpen = false; jam.page = "main"; };
  const up = $derived(app.upNext);
  const upShown = $derived(editing || dragRow ? up : up.slice(0, queueCap));
  function onListScroll() {
    if (!listEl || queueCap >= up.length) return;
    if (listEl.scrollHeight - listEl.scrollTop - listEl.clientHeight < 260) queueCap += 30;
  }
  const manual = $derived(Math.min(app.queue.user, up.length));
  const base = $derived((app.queue.index ?? -1) + 1);
  const remote = $derived(app.devices.find((d) => d.active && d.kind !== "this"));
  const devName = $derived(remote?.name ?? app.config.audio.output_device ?? thisName());
  const sleepLabelText = $derived.by(() => {
    void app.clock;
    const s = app.sleep;
    if (!s) return t("sleep.short");
    if (s.endOfSong) return t("sleep.end_of_song");
    return s.until ? sleepCountdown(s.until - Date.now()) : t("sleep.short");
  });
  const DEV_ICON: Record<string, string> = { this: "smartphone", glyph: "monitor", cast: "cast", airplay: "airplay", upnp: "speaker", account: "laptop" };

  $effect(() => { if (!up.length && editing) { editing = false; selected = []; } });

  function toggleSel(qIdx: number) {
    selected = selected.includes(qIdx) ? selected.filter((x) => x !== qIdx) : [...selected, qIdx];
  }

  async function moveSelectedTop() {
    if (!selected.length) return;
    const sorted = [...selected].sort((a, b) => a - b);
    let dest = base;
    for (const idx of sorted) {
      if (idx !== dest) await app.queueMove(idx, dest);
      dest++;
    }
    selected = [];
    editing = false;
  }

  async function removeSelected() {
    if (!selected.length) return;
    const sorted = [...selected].sort((a, b) => b - a);
    for (const idx of sorted) await app.queueRemove(idx);
    selected = [];
    if (!app.upNext.length) editing = false;
  }

  function startRowDrag(e: PointerEvent, from: number, u: { track: Track; index: number }) {
    if (e.pointerType === "mouse" && e.button !== 0) return;
    e.stopPropagation();
    const grip = e.currentTarget as HTMLElement;
    const row = grip.closest<HTMLElement>("[data-qrow]");
    const container = listEl;
    if (!row || !container) return;
    const rowEls = [...container.querySelectorAll<HTMLElement>("[data-qrow]")];
    const startScroll = container.scrollTop;
    const centers = rowEls.map((el) => {
      const r = el.getBoundingClientRect();
      return r.top + r.height / 2 + startScroll;
    });
    const startCenter = centers[from] ?? (row.getBoundingClientRect().top + 30 + startScroll);
    const h = row.getBoundingClientRect().height || 60;
    const startY = e.clientY;
    let started = false;
    rowDragActive = true;
    pull = 0;
    try { grip.setPointerCapture(e.pointerId); } catch { /* ignore */ }
    const blockTouch = (ev: TouchEvent) => { ev.preventDefault(); ev.stopPropagation(); };
    const onMove = (ev: PointerEvent) => {
      const scrollDelta = container.scrollTop - startScroll;
      const dy = ev.clientY - startY + scrollDelta;
      if (!started) {
        if (Math.abs(ev.clientY - startY) < 4) return;
        started = true;
        swipe = null;
      }
      const draggedCenter = startCenter + dy;
      let slot = 0;
      for (let i = 0; i < centers.length; i++) {
        if (i !== from && draggedCenter > centers[i]) slot++;
      }
      slot = Math.max(0, Math.min(up.length - 1, slot));
      dragRow = { from, slot, dy, h };
    };
    const cleanup = () => {
      rowDragActive = false;
      removeEventListener("pointermove", onMove);
      removeEventListener("pointerup", onEnd);
      removeEventListener("pointercancel", onCancel);
      grip.removeEventListener("touchmove", blockTouch);
    };
    const onCancel = () => { cleanup(); dragRow = null; };
    const onEnd = () => {
      cleanup();
      const d = dragRow;
      dragRow = null;
      if (d && d.slot !== d.from && up[d.from] && up[d.slot]) {
        app.queueMove(u.index, up[d.slot].index);
      }
    };
    grip.addEventListener("touchmove", blockTouch, { passive: false });
    addEventListener("pointermove", onMove);
    addEventListener("pointerup", onEnd);
    addEventListener("pointercancel", onCancel);
  }

  /** Rubber-band damping past the swipe threshold so swiping feels physical. */
  function dampSwipe(dx: number): number {
    const sign = dx < 0 ? -1 : 1;
    const abs = Math.abs(dx);
    const max = dx < 0 ? 132 : 124;
    if (abs <= SWIPE_ACT) return dx;
    return sign * Math.min(max, SWIPE_ACT + (abs - SWIPE_ACT) * 0.35);
  }

  const rowKey = (u: { track: Track; index: number }) => `${u.index}:${u.track.source}:${u.track.id}`;
  /** Sideways swipe on a queue row: right adds it again, left removes it. */
  function swipeEnd(u: { track: Track; index: number }, dx: number, vx: number) {
    const key = rowKey(u);
    swipe = null;
    if (dx >= SWIPE_ACT || (vx > 0.5 && dx > 28)) app.queueAdd([u.track], false);
    else if (dx <= -SWIPE_ACT || (vx < -0.5 && dx < -28)) { leaving = [...leaving, key]; app.queueRemove(u.index); }
  }

  function rowShift(i: number): number {
    const d = dragRow;
    if (!d) return 0;
    if (i === d.from) return d.dy;
    if (d.from < d.slot && i > d.from && i <= d.slot) return -d.h;
    if (d.from > d.slot && i >= d.slot && i < d.from) return d.h;
    return 0;
  }
</script>

<div class="dim" transition:fade|global={{ duration: 200 }} onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")} onpointerdown={close} role="presentation">
  <div class="panel" style:transform="translateY({pull}px)" class:settle={pull === 0} in:sheetIn|global={{ duration: 260 }} out:sheetOut|global={{ duration: 210 }} onpointerdown={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
    {#if kind === "devices"}
      <ConnectView
        sheet
        onpull={(dy) => (pull = dy)}
        ondrop={(dy, vy) => { if (shouldClose(dy, vy)) { close(); return; } pull = 0; }}
        onclose={close}
      />
    {:else}
    <div class="grab" use:drag={{ axis: "y", move: (_, dy) => (pull = Math.max(0, dy)), end: (_, dy, _vx, vy) => { if (shouldClose(dy, vy)) { close(); return; } pull = 0; } }}>
      <span class="hdl"></span>
      {#if timerOpen}
        <button class="t-back" onclick={() => (timerOpen = false)}>
          <span class="bk-ic"><Icon name="chevron-left" size={20} color="rgba(255,255,255,.85)" /></span>
          <span class="t-hd">
            <b>{t("tip.timer")}</b>
            <span>{app.sleep?.endOfSong ? t("sleep.status.song") : app.sleep?.until ? tf("sleep.status.left", { time: sleepCountdown(app.sleep.until - Date.now()) }) : t("sleep.title")}</span>
          </span>
        </button>
      {:else}
        <div class="q-top">
          <div class="q-ht">
            <div class="q-title-row">
              <h2>{editing ? t("queue.edit") : t("queue.title")}</h2>
              {#if editing && selected.length}<span class="ecnt">({selected.length})</span>{/if}
            </div>
            {#if !editing}
              {#if app.queue.context}<span class="sub trunc">{t("np.playing_from")} „{app.queue.context}“</span>{/if}
              <button class="dev-line" onclick={() => { app.showQueue = false; app.showDevices = true; }}>
                <Icon name={remote ? (DEV_ICON[remote.kind] ?? "cast") : thisIcon()} size={13} color="var(--accent)" />
                <span class="trunc">{devName}</span>
              </button>
            {/if}
          </div>
          {#if editing}
            <IconButton name="x" size={20} box={36} notip onclick={() => { editing = false; selected = []; }} />
          {:else if up.length}
            <button class="edit-pill" onclick={() => { editing = true; selected = []; }}>{t("list.edit")}</button>
          {/if}
        </div>
        {#if app.player.current}
          {@const cur = app.player.current}
          <div class="now-row" role="button" tabindex="-1" oncontextmenu={(e) => { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: cur }); }}>
            <Cover track={cur} size={46} radius={4} />
            <span class="tx">
              <span class="t on">
                <span class="now-ic"><Icon name="audio-lines" size={14} color="var(--accent)" /></span>
                <SongTitle rating={rating(cur)}>{cur.title}</SongTitle>
              </span>
              <span class="a trunc">{cur.artist}</span>
            </span>
            <button class="now-play" onclick={(e) => { e.stopPropagation(); app.togglePlay(); }} aria-label={t(app.playing ? "common.pause" : "common.play")}>
              <PlayGlyph loading={app.loading} playing={app.playing} size={15} button={38} />
            </button>
          </div>
        {/if}
      {/if}
    </div>

    <div class="scroll list" bind:this={listEl} onscroll={onListScroll} use:fadeEdges={{ top: 14, bottom: 14, minOverflow: 8 }}
      use:drag={{ axis: "y", onlyAtTop: () => !rowDragActive && !dragRow && !swipe && (listEl?.scrollTop ?? 0) <= 0, move: (_, dy) => (pull = Math.max(0, dy)), end: (_, dy, _vx, vy) => { if (shouldClose(dy, vy)) { close(); return; } pull = 0; } }}>
      {#if timerOpen}
        {@const onSong = !!app.sleep?.endOfSong}
        <div class="t-list" in:fly|local={{ x: 24, duration: 180, easing: cubicOut }}>
          {#each SLEEP_MINUTES as mins}
            {@const on = app.sleep?.mins === mins}
            <button class="t-opt" class:on onclick={() => { setSleep(mins); timerOpen = false; }}>
              <Icon name="timer" size={18} color={on ? "var(--accent)" : "rgba(255,255,255,.72)"} />
              <span class="t-lb">{sleepLabel(mins)}</span>
              {#if on}<Icon name="check" size={18} color="var(--accent)" />{/if}
            </button>
          {/each}
          <button class="t-opt" class:on={onSong} onclick={() => { setSleep("song"); timerOpen = false; }}>
            <Icon name="music-2" size={18} color={onSong ? "var(--accent)" : "rgba(255,255,255,.72)"} />
            <span class="t-lb">{t("sleep.end_of_song")}</span>
            {#if onSong}<Icon name="check" size={18} color="var(--accent)" />{/if}
          </button>
          {#if app.sleep}
            <div class="t-sep"></div>
            <button class="t-opt off" onclick={() => { setSleep(null); timerOpen = false; }}>
              <Icon name="x" size={18} color="hsla(0, 75%, 68%, .9)" />
              <span class="t-lb">{t("sleep.off")}</span>
            </button>
          {/if}
        </div>
      {:else}
        {#if !editing && manual > 0 && up.length > manual}
          <div class="sec">{tf("queue.next_n", { n: manual })}</div>
        {/if}
        {#each upShown as u, i (rowKey(u))}
          {@const checked = selected.includes(u.index)}
          {@const dragging = dragRow?.from === i}
          {@const key = rowKey(u)}
          {@const sx = swipe?.key === key ? swipe.dx : 0}
          {@const gone = leaving.includes(key)}
          {@const queued = i < manual}
          {#if !editing && i === manual && manual > 0 && up.length > manual}
            <div class="sec">{app.queue.context ? tf("queue.from_n", { name: app.queue.context, n: up.length - manual }) : tf("queue.later_n", { n: up.length - manual })}</div>
          {/if}
          {@const shiftY = rowShift(i)}
          {@const armedAdd = sx >= SWIPE_ACT}
          {@const armedRm = sx <= -SWIPE_ACT}
          <div
            class="q-wrap"
            class:gone
            class:dragging
            class:reordering={!!dragRow && !dragging}
            class:swiping-wrap={sx !== 0}
            style:transform={shiftY ? `translateY(${shiftY}px)` : undefined}
            data-qrow
          >
          {#if sx > 0}
            <div class="under add" class:armed={armedAdd} style:opacity={Math.min(1, sx / 22)}>
              <span class="u-ic" class:pop={armedAdd}>
                <Icon name="list-plus" size={21} color={armedAdd ? "#0d0d0d" : "var(--accent)"} />
              </span>
            </div>
          {/if}
          {#if sx < 0}
            <div class="under rm" class:armed={armedRm} style:opacity={Math.min(1, -sx / 22)}>
              <span class="u-ic" class:pop={armedRm}>
                <Icon name="trash-2" size={21} color={armedRm ? "#fff" : "hsla(0, 78%, 68%, .95)"} />
              </span>
            </div>
          {/if}
          <div
            class="r tap q-row"
            class:dragging
            class:swiping={sx !== 0}
            style:transform={gone ? "translateX(-110%)" : sx ? `translateX(${sx}px)` : undefined}
            use:drag={{
              axis: "x",
              move: (dx) => { if (!editing && !rowDragActive && !dragRow && !gone) swipe = { key, dx: dampSwipe(dx) }; },
              end: (dx, _dy, vx) => { if (!editing && !rowDragActive && !dragRow && !gone) swipeEnd(u, dx, vx); else swipe = null; },
            }}
            onclick={() => (editing ? toggleSel(u.index) : app.queueJump(u.index))}
            oncontextmenu={(e) => { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: u.track }); }}
            onkeydown={() => {}}
            role="button"
            tabindex="-1"
          >
            {#if editing}
              <span class="chk" class:on={checked}>
                {#if checked}<Icon name="check" size={13} color="#000" />{/if}
              </span>
            {/if}
            <Cover track={u.track} size={46} radius={4} />
            <span class="tx">
              <span class="t"><SongTitle rating={rating(u.track)}>{u.track.title}</SongTitle></span>
              <span class="a qa">
                {#if queued}<span class="qd" aria-label={t("queue.added")}><Icon name="list-plus" size={12} color="var(--accent)" /></span>{/if}
                <span class="trunc">{u.track.artist}</span>
              </span>
            </span>
            <button class="grip" data-drag-own aria-label="Reorder" onpointerdown={(e) => startRowDrag(e, i, u)} onclick={(e) => e.stopPropagation()}>
              <svg width="18" height="14" viewBox="0 0 18 14" fill="none" aria-hidden="true">
                <line x1="2" y1="2.5" x2="16" y2="2.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
                <line x1="2" y1="7" x2="16" y2="7" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
                <line x1="2" y1="11.5" x2="16" y2="11.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
              </svg>
            </button>
          </div>
          </div>
        {/each}
        {#if !up.length}<p class="empty">{t("queue.empty")}</p>{/if}
      {/if}
    </div>

    {#if !timerOpen}
      <div class="foot">
        {#if editing}
          <div class="edit-bar">
            <button class="x-circ" onclick={() => { editing = false; selected = []; }} aria-label={t("common.back")}>
              <Icon name="x" size={18} color="#fff" />
            </button>
            <button class="act-pill" disabled={!selected.length} onclick={moveSelectedTop}>{t("queue.move_top")}</button>
            <button class="act-pill" disabled={!selected.length} onclick={removeSelected}>{t("queue.remove")}</button>
          </div>
        {:else}
          <div class="ctl-bar">
            <button class="ctl-btn" class:on={app.player.shuffle} onclick={() => app.toggleShuffle()}>
              <Icon name="shuffle" size={20} color={app.player.shuffle ? "var(--accent)" : "rgba(255,255,255,.88)"} />
              <span>{t("tip.shuffle")}</span>
            </button>
            <button class="ctl-btn" class:on={app.player.repeat !== "off"} onclick={() => app.cycleRepeat()}>
              <Icon name={app.player.repeat === "one" ? "repeat-1" : "repeat"} size={20} color={app.player.repeat !== "off" ? "var(--accent)" : "rgba(255,255,255,.88)"} />
              <span>{t("ctl.repeat")}</span>
            </button>
            <button class="ctl-btn" class:on={app.sleep != null} onclick={() => (timerOpen = true)}>
              <Icon name="timer" size={20} color={app.sleep != null ? "var(--accent)" : "rgba(255,255,255,.88)"} />
              <span class="trunc">{sleepLabelText}</span>
            </button>
          </div>
        {/if}
      </div>
    {/if}
    {/if}
  </div>
</div>

<style>
  .dim { position: fixed; inset: 0; z-index: 55; background: rgba(0, 0, 0, .55); }
  .panel { position: absolute; left: 0; right: 0; bottom: 0; height: min(86vh, calc(100vh - var(--safe-top, env(safe-area-inset-top)) - 12px)); height: min(86dvh, calc(100dvh - var(--safe-top, env(safe-area-inset-top)) - 12px)); border-radius: 16px 16px 0 0; background: hsl(0 0% 8%); padding-bottom: max(var(--safe-bottom, env(safe-area-inset-bottom)), var(--ime-bottom, 0px)); display: flex; flex-direction: column; overflow: hidden; }
  .panel.settle { transition: transform .24s var(--ease); }
  .grab { flex: none; touch-action: none; }
  .hdl { display: block; width: 36px; height: 4px; margin: 10px auto 8px; border-radius: 2px; background: rgba(255, 255, 255, .25); }
  .q-top { padding: 0 16px 10px; display: flex; align-items: flex-start; gap: 12px; }
  .q-ht { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .q-title-row { display: flex; align-items: baseline; gap: 8px; }
  h2 { margin: 0; font-size: 21px; font-weight: 700; color: #fff; }
  .ecnt { font-size: 13px; font-weight: 600; color: var(--accent); }
  .sub { font-size: 13px; color: var(--dim); }
  .dev-line { align-self: flex-start; max-width: 100%; margin-top: 2px; display: flex; align-items: center; gap: 5px; font-size: 12px; font-weight: 500; color: var(--accent); }
  .edit-pill { flex: none; height: 32px; padding: 0 14px; border-radius: 999px; background: rgba(255, 255, 255, .1); font-size: 13px; font-weight: 600; color: #fff; transition: background-color .14s ease-out, transform .14s var(--ease); }
  .edit-pill:active { background: rgba(255, 255, 255, .18); transform: scale(.96); }

  .now-row { padding: 6px 16px 10px; display: flex; align-items: center; gap: 12px; border-bottom: 1px solid rgba(255, 255, 255, .07); }
  .now-ic { display: inline-flex; vertical-align: -2px; margin-right: 4px; }
  .now-play { flex: none; width: 38px; height: 38px; border-radius: 50%; background: #fff; display: grid; place-items: center; transition: transform .14s var(--ease); }
  .now-play:active { transform: scale(.92); }

  .t-back { width: 100%; padding: 2px 16px 12px; display: flex; align-items: center; gap: 10px; text-align: left; border-bottom: 1px solid rgba(255, 255, 255, .07); }
  .bk-ic { flex: none; width: 34px; height: 34px; border-radius: 50%; background: rgba(255, 255, 255, .07); display: grid; place-items: center; }
  .t-hd { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .t-hd b { font-size: 16px; color: #fff; }
  .t-hd span { font-size: 12.5px; color: var(--dim); }
  .t-list { padding: 8px 10px; display: flex; flex-direction: column; gap: 2px; }
  .t-opt { width: 100%; height: 48px; padding: 0 12px; border-radius: 10px; display: flex; align-items: center; gap: 12px; font-size: 15px; color: rgba(255, 255, 255, .9); text-align: left; transition: background-color .12s ease-out; }
  .t-opt:active { background: rgba(255, 255, 255, .08); }
  .t-opt.on { color: var(--accent); font-weight: 600; }
  .t-opt.off { color: hsla(0, 75%, 68%, .9); }
  .t-lb { flex: 1; }
  .t-sep { height: 1px; margin: 6px 8px; background: rgba(255, 255, 255, .07); }

  .list { flex: 1; min-height: 0; position: relative; }
  .sec { padding: 12px 16px 4px; font-size: 13px; font-weight: 700; color: var(--dim); }
  .r { width: 100%; height: 60px; padding: 0 4px 0 10px; display: flex; align-items: center; gap: 12px; text-align: left; }
  .q-wrap { position: relative; border-radius: 10px; margin: 0 6px; transition: opacity .2s ease-out; }
  .q-wrap.reordering { transition: transform .18s var(--ease), opacity .2s ease-out; }
  .q-wrap.dragging { z-index: 10; transition: none; }
  .q-wrap.swiping-wrap, .q-wrap.gone { overflow: hidden; }
  .q-wrap.gone { opacity: 0; pointer-events: none; }
  .under {
    position: absolute;
    inset: 0;
    border-radius: 10px;
    padding: 0 18px;
    display: flex;
    align-items: center;
    gap: 8px;
    pointer-events: none;
    transition: background-color .16s ease-out, color .16s ease-out;
  }
  .under.add { justify-content: flex-start; background: color-mix(in srgb, var(--accent) 16%, #141416); color: var(--accent); }
  .under.add.armed { background: var(--accent); color: #0d0d0d; }
  .under.rm { justify-content: flex-end; background: hsla(0, 72%, 52%, .18); color: hsla(0, 78%, 70%, .95); }
  .under.rm.armed { background: hsl(0 72% 52%); color: #fff; }
  .u-ic { width: 38px; height: 38px; border-radius: 50%; display: grid; place-items: center; transition: transform .18s var(--ease); }
  .u-ic.pop { transform: scale(1.18); }
  .q-row { position: relative; border-radius: 10px; background: hsl(0 0% 8%); touch-action: pan-y; transition: transform .22s var(--ease), background-color .12s ease-out; }
  .q-row.swiping { transition: background-color .12s ease-out; background: #161618; }
  /* Solid while pressed, so the swipe icons underneath never show through. */
  .q-row.tap:active { background: hsl(0 0% 11%); }
  .q-row.dragging { transition: none; background: #1e1e22; border-radius: 10px; }
  .tap:active { background: rgba(255, 255, 255, .06); }
  .chk { flex: none; width: 19px; height: 19px; border-radius: 4px; background: rgba(255, 255, 255, .16); display: grid; place-items: center; transition: background-color .12s ease-out; }
  .chk.on { background: var(--accent); }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .t { font-size: 15px; color: var(--text); min-width: 0; display: flex; align-items: center; }
  .t.on { color: var(--accent); font-weight: 500; }
  .a { font-size: 12.5px; color: var(--dim); }
  .qa { display: flex; align-items: center; gap: 5px; min-width: 0; }
  .qd { flex: none; display: inline-flex; }
  .grip { flex: none; width: 42px; height: 48px; display: grid; place-items: center; color: rgba(255, 255, 255, .45); touch-action: none; cursor: grab; }
  .grip:active { color: #fff; cursor: grabbing; }
  .empty { padding: 16px; color: var(--dim); }

  .foot { flex: none; padding: 10px 14px 12px; border-top: 1px solid rgba(255, 255, 255, .07); background: hsl(0 0% 8%); }
  .ctl-bar { display: flex; gap: 8px; }
  .ctl-btn { flex: 1; min-width: 0; height: 60px; padding: 8px 6px; border-radius: 10px; background: rgba(255, 255, 255, .07); display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 5px; font-size: 11.5px; font-weight: 500; color: rgba(255, 255, 255, .88); transition: background-color .14s ease-out, transform .14s var(--ease), color .14s ease-out; }
  .ctl-btn.on { color: var(--accent); background: rgba(255, 255, 255, .1); }
  .ctl-btn:active { transform: scale(.96); }
  .edit-bar { display: flex; align-items: center; justify-content: center; gap: 10px; }
  .x-circ { flex: none; width: 40px; height: 40px; border-radius: 50%; background: rgba(255, 255, 255, .1); display: grid; place-items: center; transition: transform .14s var(--ease); }
  .x-circ:active { transform: scale(.92); }
  .act-pill { height: 40px; padding: 0 18px; border-radius: 999px; background: rgba(255, 255, 255, .14); font-size: 13.5px; font-weight: 600; color: #fff; transition: opacity .14s, background-color .14s, transform .14s var(--ease); }
  .act-pill:disabled { opacity: .4; pointer-events: none; }
  .act-pill:active { transform: scale(.96); }
</style>
