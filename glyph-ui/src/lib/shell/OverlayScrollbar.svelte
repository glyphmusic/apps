<script lang="ts">
  // Overlay vertical scrollbar rendered on top of the page frame so hero
  // gradients, banners and full-bleed rows extend seamlessly behind the thumb.
  let {
    el,
    top = 16,
    bottom = 12,
  }: {
    el: HTMLElement | undefined;
    top?: number;
    bottom?: number;
  } = $props();

  let trackEl: HTMLDivElement | undefined = $state();
  let canScroll = $state(false);
  let thumbH = $state(0);
  let thumbTop = $state(0);
  let dragging = $state(false);
  let scrolling = $state(false);
  let hover = $state(false);

  let raf = 0;
  let scrollIdleTimer = 0;
  let maxScroll = 0;
  let maxThumbTop = 0;

  function sync() {
    raf = 0;
    if (!el || !trackEl) {
      canScroll = false;
      return;
    }
    const ch = el.clientHeight;
    const sh = el.scrollHeight;
    const st = el.scrollTop;
    const trackH = trackEl.clientHeight;
    maxScroll = Math.max(0, sh - ch);
    if (ch <= 0 || trackH <= 0 || maxScroll <= 1) {
      canScroll = false;
      thumbH = 0;
      thumbTop = 0;
      maxThumbTop = 0;
      return;
    }
    const h = Math.max(32, Math.min(trackH, Math.round((ch / sh) * trackH)));
    maxThumbTop = Math.max(0, trackH - h);
    thumbH = h;
    thumbTop = maxThumbTop > 0 ? Math.min(maxThumbTop, Math.max(0, (st / maxScroll) * maxThumbTop)) : 0;
    canScroll = true;
  }

  function schedule() {
    if (!raf) raf = requestAnimationFrame(sync);
  }

  function onScroll() {
    sync();
    scrolling = true;
    clearTimeout(scrollIdleTimer);
    scrollIdleTimer = window.setTimeout(() => {
      scrolling = false;
    }, 700);
  }

  $effect(() => {
    const target = el;
    const track = trackEl;
    if (!target || !track) {
      canScroll = false;
      return;
    }
    const ro = new ResizeObserver(schedule);
    const observeAll = () => {
      ro.disconnect();
      ro.observe(target);
      ro.observe(track);
      for (const child of target.children) ro.observe(child);
    };
    observeAll();

    const mo = new MutationObserver(() => {
      observeAll();
      schedule();
    });
    mo.observe(target, { childList: true });

    target.addEventListener("scroll", onScroll, { passive: true });
    sync();
    schedule();

    return () => {
      cancelAnimationFrame(raf);
      raf = 0;
      clearTimeout(scrollIdleTimer);
      scrolling = false;
      target.removeEventListener("scroll", onScroll);
      ro.disconnect();
      mo.disconnect();
    };
  });

  function beginDrag(e: PointerEvent, startScrollTop: number) {
    if (!el) return;
    const target = el;
    const startY = e.clientY;
    dragging = true;
    const move = (ev: PointerEvent) => {
      if (maxThumbTop <= 0 || maxScroll <= 0) return;
      const dy = ev.clientY - startY;
      target.scrollTop = Math.min(maxScroll, Math.max(0, startScrollTop + (dy / maxThumbTop) * maxScroll));
    };
    const up = () => {
      dragging = false;
      removeEventListener("pointermove", move);
      removeEventListener("pointerup", up);
      removeEventListener("pointercancel", up);
    };
    addEventListener("pointermove", move);
    addEventListener("pointerup", up);
    addEventListener("pointercancel", up);
  }

  function onThumbDown(e: PointerEvent) {
    if (e.button !== 0 || !el || !canScroll) return;
    e.preventDefault();
    e.stopPropagation();
    beginDrag(e, el.scrollTop);
  }

  function onTrackDown(e: PointerEvent) {
    if (e.button !== 0 || !el || !trackEl || !canScroll) return;
    e.preventDefault();
    e.stopPropagation();
    const rect = trackEl.getBoundingClientRect();
    const clickY = e.clientY - rect.top;
    const nextTop = Math.min(maxThumbTop, Math.max(0, clickY - thumbH / 2));
    const nextScroll = maxThumbTop > 0 ? (nextTop / maxThumbTop) * maxScroll : 0;
    el.scrollTop = nextScroll;
    sync();
    beginDrag(e, nextScroll);
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="osb"
  class:show={canScroll}
  class:active={hover || dragging}
  class:scrolling
  class:drag={dragging}
  style:top="{top}px"
  style:bottom="{bottom}px"
  bind:this={trackEl}
  onpointerenter={() => (hover = true)}
  onpointerleave={() => (hover = false)}
  onpointerdown={onTrackDown}
  aria-hidden="true"
>
  <div
    class="thumb"
    style:height="{thumbH}px"
    style:transform="translateY({thumbTop}px)"
    onpointerdown={onThumbDown}
  ></div>
</div>

<style>
  .osb {
    position: absolute;
    right: 2px;
    width: 12px;
    z-index: 30;
    opacity: 0;
    pointer-events: none;
    touch-action: none;
    transition: opacity .18s ease-out;
  }
  .osb.show {
    opacity: 1;
    pointer-events: auto;
  }
  .thumb {
    position: absolute;
    top: 0;
    right: 3px;
    width: 4px;
    border-radius: 99px;
    background: rgba(255, 255, 255, .20);
    will-change: transform;
    transition: width .14s ease-out, background-color .14s ease-out;
  }
  .osb.scrolling .thumb {
    background: rgba(255, 255, 255, .30);
  }
  .osb.active .thumb {
    width: 6px;
    background: rgba(255, 255, 255, .38);
  }
  .osb.drag .thumb {
    width: 6px;
    background: rgba(255, 255, 255, .52);
  }
</style>
