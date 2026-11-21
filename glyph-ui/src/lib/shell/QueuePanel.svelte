<script lang="ts" module>
  // The queue's radio switch lives while the app runs (app.radio in Rust).
  const ui = $state({ radio: false });
</script>

<script lang="ts">
  // Settings-styled queue side panel (320 px). Sections use clean uppercase
  // labels (no bracketed monospace headers) and surface cards; tracks show
  // explicit ratings via SongTitle, and the radio toggle matches Settings rows.
  import Cover from "../components/Cover.svelte";
  import IconButton from "../components/IconButton.svelte";
  import ToggleButton from "../components/ToggleButton.svelte";
  import EmptyState from "../components/EmptyState.svelte";
  import SongTitle from "../components/SongTitle.svelte";
  import Tog from "../settings/Tog.svelte";
  import FadeScroll from "./panel/FadeScroll.svelte";
  import QueueRow from "./panel/QueueRow.svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import { rating, trackKey, type Track } from "../api/types";

  const ROW_H = 44;

  const up = $derived(app.upNext);
  const n = $derived(up.length);
  const manual = $derived(Math.min(app.queue.user, n));
  const base = $derived((app.queue.index ?? -1) + 1);
  const cur = $derived(app.player.current);
  const offline = $derived(app.backendState.kind === "offline");

  type GroupItem = { i: number; tr: Track; gapKey: number };
  type Group = { head: string; items: GroupItem[] };

  /** Drop slot (index among the upcoming songs) for a cursor viewport y. */
  function slotAt(clientY: number): number {
    const rows = [...(listEl?.querySelectorAll<HTMLElement>("[data-qslot]") ?? [])];
    if (!rows.length) return 0;
    let slot = 0;
    for (let idx = 0; idx < rows.length; idx++) {
      const rr = rows[idx].getBoundingClientRect();
      if (clientY > rr.top + rr.height / 2) slot = idx + 1;
    }
    return Math.min(Math.max(0, slot), Math.max(0, n - 1));
  }

  // ---- drag state ----
  let drag = $state<{ from: number; slot: number; y: number; grab: number; left: number; w: number; inside: boolean; settle: boolean } | null>(null);
  let gaps = $state<number[]>([]);
  let listEl: HTMLDivElement | undefined = $state();
  let scrollEl: HTMLDivElement | undefined = $state();
  let panelEl: HTMLDivElement | undefined = $state();
  let didDrag = false;

  const groups = $derived.by<Group[]>(() => {
    const out: Group[] = [];
    let slot = 0;
    if (manual > 0) {
      const items: GroupItem[] = [];
      for (let i = 0; i < manual; i++) {
        if (drag && drag.from === i) continue;
        items.push({ i, tr: up[i].track, gapKey: slot++ });
      }
      out.push({ head: tf("queue.next_n", { n: manual }), items });
    }
    if (n > manual) {
      const rest = n - manual;
      const head = app.queue.context
        ? tf("queue.from_n", { name: app.queue.context, n: rest })
        : manual > 0
          ? tf("queue.later_n", { n: rest })
          : tf("queue.next_n", { n: rest });
      const items: GroupItem[] = [];
      for (let i = manual; i < n; i++) {
        if (drag && drag.from === i) continue;
        items.push({ i, tr: up[i].track, gapKey: slot++ });
      }
      out.push({ head, items });
    }
    return out;
  });

  function down(e: PointerEvent, i: number) {
    if (e.button !== 0) return;
    const row = e.currentTarget as HTMLElement, r = row.getBoundingClientRect(), startY = e.clientY;
    didDrag = false;
    let started = false;
    const move = (ev: PointerEvent) => {
      if (!started) {
        if (Math.abs(ev.clientY - startY) < 4) return;
        started = didDrag = true;
        drag = { from: i, slot: i, y: ev.clientY, grab: startY - r.top, left: r.left, w: r.width, inside: true, settle: false };
        loop();
      }
      if (!drag || drag.settle) return;
      const pr = panelEl!.getBoundingClientRect();
      drag.y = ev.clientY;
      drag.slot = slotAt(ev.clientY);
      drag.inside = ev.clientX >= pr.left - 40 && ev.clientX <= pr.right + 40 && ev.clientY >= pr.top && ev.clientY <= pr.bottom;
    };
    const stop = () => { removeEventListener("pointermove", move); removeEventListener("pointerup", end); removeEventListener("pointercancel", cancel); };
    const cancel = () => { stop(); drag = null; };
    const end = () => {
      stop();
      const d = drag;
      if (!d) return;
      if (!d.inside || d.slot === d.from) { drag = null; return; }
      // The lifted row settles into the gap, then the move is committed.
      const gap = listEl?.querySelector<HTMLElement>(`[data-slot="${d.slot}"]`);
      if (gap) d.y = gap.getBoundingClientRect().top + d.grab;
      d.settle = true;
      setTimeout(() => { app.queueMove(base + d.from, base + d.slot); drag = null; }, 150);
    };
    addEventListener("pointermove", move);
    addEventListener("pointerup", end);
    addEventListener("pointercancel", cancel);
  }

  // Gaps ease towards a full row at the target slot; the list scrolls
  // while the lifted row is near its top or bottom edge.
  function loop() {
    let last = performance.now();
    const tick = (now: number) => {
      const d = drag;
      if (!d) { gaps = []; return; }
      const slots = n - 1, dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      if (gaps.length !== slots + 1) { const v = new Array(slots + 1).fill(0); v[Math.min(d.slot, slots)] = ROW_H; gaps = v; }
      else { const k = 1 - Math.exp(-dt / 0.06); gaps = gaps.map((h, i) => h + ((i === d.slot ? ROW_H : 0) - h) * k); }
      if (scrollEl && !d.settle) {
        const r = scrollEl.getBoundingClientRect();
        if (d.y < r.top + 28) scrollEl.scrollTop -= 8;
        else if (d.y > r.bottom - 28) scrollEl.scrollTop += 8;
      }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }

  function jump(abs: number) { if (!didDrag) app.queueJump(abs); didDrag = false; }
  function setRadio(v: boolean) { ui.radio = v; backend().call("set_radio", { on: v }); }
  function saveQueueAsPlaylist() {
    const tracks: Track[] = [];
    if (cur) tracks.push(cur);
    for (const item of up) tracks.push(item.track);
    if (!tracks.length) return;
    app.nameDialog = { tracks, rename: null, initial: app.queue.context ?? "" };
  }
  // The switch shows what Rust has (radio is on by default there).
  $effect(() => { backend().call<{ on: boolean } | undefined>("radio_state").then((r) => { if (r) ui.radio = r.on; }); });
</script>

<div class="qp" bind:this={panelEl}>
  <div class="head">
    <div class="ttl">{t("queue.title")}</div>
    {#if cur || n > 0}
      <IconButton name="list-plus" size={15} label={t("queue.save_playlist")} onclick={saveQueueAsPlaylist} />
    {/if}
    <ToggleButton name="shuffle" size={15} on={app.player.shuffle} accent="var(--accent)" onclick={() => app.toggleShuffle()} />
    <IconButton name="x" size={16} onclick={() => (app.showQueue = false)} />
  </div>

  {#if cur}
    <div class="lbl">{t("queue.now")}</div>
    <div class="card now">
      <Cover track={cur} size={44} radius={6} />
      <div class="nt">
        <div class="n1"><SongTitle rating={rating(cur)}>{cur.title}</SongTitle></div>
        <div class="n2 trunc">{cur.artist}</div>
      </div>
    </div>
  {/if}
  {#if n === 0}<div class="lbl">{t("queue.next")}</div>{/if}

  <FadeScroll bind:el={scrollEl} pad="0 12px 12px">
    <div class="list" bind:this={listEl}>
      {#if offline}
        <div class="card es"><EmptyState icon="refresh-cw" title={t("status.queue_offline")} /></div>
      {:else if n === 0}
        <div class="card es"><EmptyState title={t("queue.empty")} sub={ui.radio ? t("queue.radio_next") : ""} /></div>
      {/if}
      {#each groups as g, gi (g.head)}
        <div class="sec"><span class="trunc">{g.head}</span></div>
        <div class="card grp">
          {#each g.items as en, ri (`${en.i}${trackKey(en.tr)}`)}
            {@const ggap = gaps[en.gapKey] ?? 0}
            {#if drag && ggap > 0.5}<div class="gap" data-slot={en.gapKey} style:height="{ggap}px"></div>{/if}
            <div class="qitem" class:sep={ri > 0} data-qslot={en.gapKey}>
              <QueueRow track={en.tr} n={(en.i < manual ? en.i : en.i - manual) + 1} off={!app.allowed(en.tr)} queued={en.i < manual}
                onclick={() => jump(base + en.i)} onremove={() => app.queueRemove(base + en.i)} ondown={(e) => down(e, en.i)} />
            </div>
          {/each}
          {#if drag && gi === groups.length - 1}
            {@const endGap = gaps[n - 1] ?? 0}
            {#if endGap > 0.5}<div class="gap" data-slot={n - 1} style:height="{endGap}px"></div>{/if}
          {/if}
        </div>
      {/each}
    </div>
  </FadeScroll>

  <div class="foot">
    <div class="frow">
      <span class="flab">{t("queue.radio")}</span>
      <Tog on={ui.radio} onchange={setRadio} label={t("queue.radio")} />
    </div>
  </div>
</div>

{#if drag && up[drag.from]}
  <div class="lift" class:settle={drag.settle} style:left="{drag.left}px" style:top="{drag.y - drag.grab}px" style:width="{drag.w}px">
    <QueueRow track={up[drag.from].track} n={drag.from + 1} lifted />
  </div>
{/if}

<style>
  .qp { width: 100%; height: 100%; display: flex; flex-direction: column; background: rgba(12, 12, 12, .94); border-radius: 12px; overflow: hidden; }
  .head { flex: none; display: flex; align-items: center; padding: 16px 10px 8px 16px; }
  .ttl { flex: 1; font-size: var(--lg); font-weight: 600; color: #fff; }
  .lbl { flex: none; padding: 10px 16px 6px; font-size: 11.5px; font-weight: 600; color: rgba(255, 255, 255, .45); text-transform: uppercase; letter-spacing: .02em; }
  .card { border-radius: 10px; background: rgba(255, 255, 255, .035); }
  .now { flex: none; margin: 0 12px 4px; padding: 10px; display: flex; align-items: center; gap: 12px; }
  .nt { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .n1 { font-size: var(--md); font-weight: 500; color: var(--accent); min-width: 0; }
  .n2 { font-size: var(--sm); color: var(--dim); }
  .list { display: flex; flex-direction: column; min-height: 100%; }
  .es { padding: 8px 14px; }
  .sec { flex: none; padding: 12px 4px 6px; display: flex; align-items: center; font-size: 11.5px; font-weight: 600; color: rgba(255, 255, 255, .45); text-transform: uppercase; letter-spacing: .02em; }
  .grp { padding: 4px; display: flex; flex-direction: column; }
  .qitem { content-visibility: auto; contain-intrinsic-size: auto 44px; }
  .qitem.sep { border-top: 1px solid rgba(255, 255, 255, .06); }
  .gap { flex: none; }
  .foot { flex: none; padding: 12px 16px 14px; border-top: 1px solid rgba(255, 255, 255, .06); }
  .frow { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .flab { font-size: 13px; font-weight: 500; color: rgba(255, 255, 255, .88); min-width: 0; }
  .lift { position: fixed; z-index: 70; pointer-events: none; }
  .lift.settle { transition: top .15s var(--ease); }
</style>
