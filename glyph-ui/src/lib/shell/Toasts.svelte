<script lang="ts">
  // toasts.rs: top 40, centred, 460 wide, h 40, radius 10, hsla(0 0 8.5% /
  // .98), 1 px white 7 %. Newest in front; older ones peek 9 px below as a
  // deck (each 30 px narrower, fainter); hovering fans them out. Fade in
  // 200 ms, lift 8 px while fading out; swipe up or sideways / click to dismiss.
  import Icon from "../components/Icon.svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  const H = 40, GAP = 8, PEEK = 9, SHRINK = 30, DECK = 3;
  const SWIPE_Y = 24;
  let open = $state(false);
  let drag = $state<{ id: number; dx: number; dy: number } | null>(null);
  let exitMap = $state<Record<number, { top: number; dx: number; dy: number }>>({});
  let enteredMap = $state<Record<number, boolean>>({});
  let scrollMap = $state<Record<number, { dist: number; dur: number }>>({});
  const expanded = $derived(open || drag !== null);
  const live = $derived(app.toasts.filter((x) => !x.gone));
  const height = $derived(expanded ? live.length * (H + GAP) : H + Math.min(DECK, Math.max(0, live.length - 1)) * PEEK);

  function rubberY(dy: number): number {
    return dy < 0 ? dy : Math.min(12, dy * 0.25);
  }

  function slotTop(id: number): number {
    const slot = live.findIndex((t) => t.id === id);
    if (slot < 0) return 0;
    return expanded ? slot * (H + GAP) : Math.min(slot, DECK) * PEEK;
  }

  function toastDrag(el: HTMLElement, id: number) {
    let toastId = id;
    let start: { x: number; y: number; pid: number; lx: number; ly: number; lt: number; vx: number; vy: number } | null = null;
    let claimed = false;
    let suppressClick = 0;

    const down = (e: PointerEvent) => {
      if (e.pointerType === "mouse" && e.button !== 0) return;
      if ((e.target as HTMLElement | null)?.closest(".undo")) return;
      suppressClick = 0;
      const now = performance.now();
      start = { x: e.clientX, y: e.clientY, pid: e.pointerId, lx: e.clientX, ly: e.clientY, lt: now, vx: 0, vy: 0 };
      claimed = false;
    };

    const move = (e: PointerEvent) => {
      if (!start || e.pointerId !== start.pid) return;
      const rawDx = e.clientX - start.x;
      const rawDy = e.clientY - start.y;
      const now = performance.now();
      const dt = Math.max(1, now - start.lt);
      const ivx = (e.clientX - start.lx) / dt;
      const ivy = (e.clientY - start.ly) / dt;
      start.vx = start.vx * 0.35 + ivx * 0.65;
      start.vy = start.vy * 0.35 + ivy * 0.65;
      start.lx = e.clientX;
      start.ly = e.clientY;
      start.lt = now;

      if (!claimed) {
        if (Math.hypot(rawDx, rawDy) < 5) return;
        claimed = true;
        enteredMap[toastId] = true;
        try { el.setPointerCapture(e.pointerId); } catch { /* ignore */ }
        drag = { id: toastId, dx: rawDx, dy: rubberY(rawDy) };
        app.pauseToasts(true);
      } else {
        drag = { id: toastId, dx: rawDx, dy: rubberY(rawDy) };
      }
      e.preventDefault();
    };

    const up = (e: PointerEvent) => {
      if (!start || e.pointerId !== start.pid) return;
      try {
        if (el.hasPointerCapture(e.pointerId)) el.releasePointerCapture(e.pointerId);
      } catch { /* ignore */ }

      const rawDx = e.clientX - start.x;
      const rawDy = e.clientY - start.y;
      const stale = performance.now() - start.lt > 85;
      const vx = stale ? 0 : start.vx;
      const vy = stale ? 0 : start.vy;
      const wasClaimed = claimed;
      start = null;
      claimed = false;
      if (!wasClaimed) return;

      suppressClick = performance.now();
      e.preventDefault();

      if (e.type !== "pointercancel") {
        const w = el.offsetWidth || 460;
        const dy = rubberY(rawDy);
        const xThresh = Math.max(64, Math.min(110, w * 0.26));
        const xReversed = rawDx * vx < 0 && Math.abs(vx) > 0.3;
        const xFlick = rawDx * vx > 0 && Math.abs(vx) > 0.4 && Math.abs(rawDx) > 22;
        const dismissX = !xReversed && (Math.abs(rawDx) > xThresh || xFlick);

        const yReversed = vy > 0.3;
        const yFlick = vy < -0.35 && rawDy < -10;
        const dismissY = !yReversed && (rawDy < -SWIPE_Y || yFlick);

        if (dismissX || dismissY) {
          const top = slotTop(toastId);
          const preferX = dismissX && (!dismissY || Math.abs(rawDx) / xThresh >= Math.max(0, -rawDy) / SWIPE_Y);
          if (preferX) {
            const dir = rawDx >= 0 ? 1 : -1;
            exitMap[toastId] = { top, dx: rawDx + dir * Math.max(140, w * 0.45), dy };
          } else {
            exitMap[toastId] = { top, dx: rawDx, dy: Math.min(dy - 40, -52) };
          }
          app.dismissToast(toastId);
        }
      }

      if (drag?.id === toastId) drag = null;
      if (!open) app.pauseToasts(false);
    };

    const click = (e: MouseEvent) => {
      if (claimed || performance.now() - suppressClick < 350) {
        suppressClick = 0;
        e.stopPropagation();
        e.preventDefault();
      }
    };

    el.addEventListener("pointerdown", down);
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
    el.addEventListener("click", click, true);
    return {
      update(nextId: number) { toastId = nextId; },
      destroy() {
        el.removeEventListener("pointerdown", down);
        el.removeEventListener("pointermove", move);
        el.removeEventListener("pointerup", up);
        el.removeEventListener("pointercancel", up);
        el.removeEventListener("click", click, true);
        if (drag?.id === toastId) {
          drag = null;
          if (!open) app.pauseToasts(false);
        }
        delete exitMap[toastId];
        delete enteredMap[toastId];
      },
    };
  }

  function measure(clip: HTMLElement, params: { id: number; open: boolean; key: string }) {
    let raf = 0;
    const sync = (p: { id: number; open: boolean; key: string }) => {
      cancelAnimationFrame(raf);
      if (!p.open) {
        delete scrollMap[p.id];
        return;
      }
      raf = requestAnimationFrame(() => {
        const track = clip.firstElementChild as HTMLElement | null;
        if (!track) return;
        const overflow = track.scrollWidth - clip.clientWidth;
        if (overflow > 4) {
          scrollMap[p.id] = { dist: overflow + 8, dur: Number(Math.max(2.4, overflow / 42 + 1.4).toFixed(2)) };
        } else {
          delete scrollMap[p.id];
        }
      });
    };
    sync(params);
    return {
      update: sync,
      destroy: () => { cancelAnimationFrame(raf); delete scrollMap[params.id]; },
    };
  }
</script>

<div
  class="deck"
  style:height="{height}px"
  role="status"
  onpointerenter={() => { open = true; app.pauseToasts(true); }}
  onpointerleave={() => { open = false; if (!drag) app.pauseToasts(false); }}
>
  {#each app.toasts as x (x.id)}
    {@const slot = live.indexOf(x)}
    {@const depth = expanded || slot < 0 ? 0 : slot}
    {@const exit = exitMap[x.id]}
    {@const isDragging = drag?.id === x.id}
    {@const y = exit ? exit.top : slot < 0 ? -36 : expanded ? slot * (H + GAP) : Math.min(slot, DECK) * PEEK}
    {@const dx = isDragging ? drag!.dx : exit ? exit.dx : 0}
    {@const dy = isDragging ? drag!.dy : exit ? exit.dy : 0}
    {@const baseOpacity = slot < 0 ? 0 : Math.max(0, 1 - depth * 0.22) * (depth > DECK ? 0 : 1)}
    {@const dragFade = isDragging ? Math.max(0.25, 1 - Math.max(Math.abs(dx) / 220, Math.max(0, -dy) / 60) * 0.65) : 1}
    {@const m = scrollMap[x.id]}
    <div
      class="toast"
      class:entering={!enteredMap[x.id]}
      class:dragging={isDragging}
      class:gone={x.gone}
      style:top="{y}px"
      style:--dx="{dx}px"
      style:--dy="{dy}px"
      style:width="calc(min(460px, 100vw - 32px) - {depth * SHRINK}px)"
      style:opacity={x.gone || slot < 0 ? 0 : baseOpacity * dragFade}
      style:z-index={100 - Math.max(0, slot)}
      use:toastDrag={x.id}
      onanimationend={() => (enteredMap[x.id] = true)}
      onclick={() => app.dismissToast(x.id)}
      onkeydown={() => {}}
      role="button"
      tabindex="-1"
    >
      <span class="body" style:opacity={Math.max(0, 1 - depth)}>
        <Icon name={x.kind === "ok" ? "check" : "info"} size={15} color={x.kind === "ok" ? "hsl(145 55% 52%)" : "rgba(255,255,255,.6)"} />
        <span
          class="tx"
          class:hover={open}
          class:animate={open && !!m}
          style:--scroll-dist={m ? `${-m.dist}px` : "0px"}
          style:--scroll-dur={m ? `${m.dur}s` : "4s"}
          use:measure={{ id: x.id, open, key: `${x.title}\0${x.sub ?? ""}` }}
        >
          <span class="tx-track">
            <b>{x.title}</b>
            {#if x.sub}<i>·</i><em>{x.sub.toUpperCase()}</em>{/if}
          </span>
        </span>
        {#if x.undo}
          <button class="undo" onclick={(e) => { e.stopPropagation(); x.undo?.run(); app.dismissToast(x.id); }}>{x.undo.label ?? t("toast.undo")}</button>
        {/if}
      </span>
    </div>
  {/each}
</div>

<style>
  .deck { position: fixed; top: 40px; left: 50%; transform: translateX(-50%); width: min(460px, calc(100vw - 32px)); z-index: 90; transition: height .25s var(--ease); pointer-events: none; }
  .toast { position: absolute; left: 50%; transform: translate(calc(-50% + var(--dx, 0px)), var(--dy, 0px)); height: 40px; padding: 0 16px; display: flex; align-items: center; border-radius: 10px; background: #1a1a1a; cursor: grab; touch-action: none; pointer-events: auto;
    transition: top .3s var(--ease), width .3s var(--ease), transform .22s var(--ease), opacity .25s ease-out; }
  .toast.entering { animation: toast-in .2s ease-out; }
  .toast.dragging { cursor: grabbing; transition: top .3s var(--ease), width .3s var(--ease); }
  .toast.gone { pointer-events: none; transition: top .18s ease-in, transform .18s ease-in, opacity .18s ease-in; }
  @keyframes toast-in { from { opacity: 0; transform: translate(-50%, -14px); } to { transform: translate(-50%, 0); } }
  .body { display: flex; align-items: center; gap: 10px; width: 100%; min-width: 0; transition: opacity .2s; }
  .tx { flex: 1; min-width: 0; overflow: hidden; }
  .tx-track { display: inline-flex; align-items: baseline; gap: 10px; max-width: 100%; white-space: nowrap; will-change: transform; }
  .tx.hover .tx-track { max-width: none; }
  .tx.animate { mask-image: linear-gradient(90deg, transparent 0, #000 8px, #000 calc(100% - 8px), transparent 100%); -webkit-mask-image: linear-gradient(90deg, transparent 0, #000 8px, #000 calc(100% - 8px), transparent 100%); }
  .tx.animate .tx-track { animation: toast-marquee var(--scroll-dur, 4s) ease-in-out infinite; }
  @keyframes toast-marquee {
    0%, 14% { transform: translateX(0); }
    72%, 88% { transform: translateX(var(--scroll-dist, 0px)); }
    100% { transform: translateX(0); }
  }
  b { font-size: 14px; font-weight: 600; color: rgba(255, 255, 255, .95); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  i { font-style: normal; font-size: 13px; color: rgba(255, 255, 255, .3); flex: none; }
  em { font-style: normal; font-size: 13px; font-weight: 500; color: rgba(255, 255, 255, .45); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tx.hover b, .tx.hover em { overflow: visible; text-overflow: clip; }
  .undo { flex: none; height: 26px; padding: 0 10px; border-radius: 6px; font-size: 13px; font-weight: 600; color: var(--accent); }
  .undo:hover { background: rgba(255, 255, 255, .08); }
</style>
