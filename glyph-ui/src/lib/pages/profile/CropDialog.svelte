<script lang="ts">
  // profile.rs crop_dialog: crop dialog for a picked profile picture or
  // banner. Panel w 528 (stage 480 + 2 x 24), p 24, radius 12, bg
  // hsla(0,0,.09,.99), gap 18. Title 20 bold white 95 % + close icon (18,
  // white 60 %); stage 480 x 320, radius 8, white 4 %, the picture under
  // four 60 % black shades and a 2 px white 95 % frame (round for the
  // picture, radius 8 for the banner); drag moves, wheel zooms; zoom slider
  // (220 px) between two image icons; reset (text button, h 36), cancel
  // (h 36 px 18), apply (primary, h 36 px 18). One instance renders: the
  // first one mounted (Profile page, or the overlay layer if the lead adds it).
  import { onMount } from "svelte";
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import ModalLayer from "../../overlays/ModalLayer.svelte";
  import Button from "../../components/Button.svelte";
  import IconButton from "../../components/IconButton.svelte";
  import Icon from "../../components/Icon.svelte";
  import ThinBar from "../../components/ThinBar.svelte";
  import { t } from "../../i18n.svelte";
  import { applyCrop, cancelCrop, clampCrop, crop, CROP_MAX_ZOOM, CROP_STAGE, cropRect, cropScale, cropWindow } from "./crop.svelte";

  const me = Symbol("crop");
  onMount(() => {
    if (!crop.host) crop.host = me;
    return () => { if (crop.host === me) crop.host = null; };
  });
  // A second mounted instance takes over when the first goes away.
  $effect(() => { if (!crop.host) crop.host = me; });

  const d = $derived(crop.dlg);
  const [sw, sh] = CROP_STAGE;
  const win = $derived(d ? cropWindow(d.banner) : [0, 0]);
  const vx = $derived((sw - win[0]) / 2), vy = $derived((sh - win[1]) / 2);
  const s = $derived(d ? cropScale(d) : 1);
  const rect = $derived(d ? cropRect(d.w, d.h, d.banner, d) : [0, 0, 1, 1]);
  const frac = $derived(d ? (d.zoom - 1) / (CROP_MAX_ZOOM - 1) : 0);

  let drag: { mx: number; my: number; ox: number; oy: number } | null = null;
  function down(e: PointerEvent) {
    if (!d || e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = { mx: e.clientX, my: e.clientY, ox: d.cx, oy: d.cy };
  }
  function move(e: PointerEvent) {
    if (!d || !drag) return;
    const sc = cropScale(d);
    d.cx = drag.ox - (e.clientX - drag.mx) / Math.max(1, d.w * sc);
    d.cy = drag.oy - (e.clientY - drag.my) / Math.max(1, d.h * sc);
    clampCrop(d);
  }
  function up() { drag = null; }
  function wheel(e: WheelEvent) {
    if (!d) return;
    e.preventDefault();
    d.zoom = d.zoom * (1 + e.deltaY / 400);
    clampCrop(d);
  }
  function key(e: KeyboardEvent) {
    if (crop.host === me && d && e.key === "Escape") { e.stopPropagation(); cancelCrop(); }
  }
  function zoomTo(f: number) { if (d) { d.zoom = 1 + f * (CROP_MAX_ZOOM - 1); clampCrop(d); } }
  function reset() { if (d) { d.zoom = 1; d.cx = 0.5; d.cy = 0.5; } }
</script>

<svelte:window onkeydown={key} />

{#if d && crop.host === me}
  <div transition:fade={{ duration: 180 }}>
    <ModalLayer dim={0.55} onbackdrop={cancelCrop}>
      <div class="panel" style:width="{sw + 48}px" transition:fly|global={{ y: 8, duration: 180, easing: cubicOut }} role="dialog">
        <div class="head">
          <div class="title">{t(d.banner ? "profile.crop.banner" : "profile.crop.avatar")}</div>
          <IconButton name="x" size={18} color="rgba(255,255,255,.6)" onclick={cancelCrop} />
        </div>
        <div class="stage" style:width="{sw}px" style:height="{sh}px" onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up} onwheel={wheel} role="presentation">
          <img src={d.url} alt="" draggable="false" style:left="{vx - rect[0] * s}px" style:top="{vy - rect[1] * s}px" style:width="{d.w * s}px" style:height="{d.h * s}px" />
          <div class="shade" style:left="0" style:top="0" style:width="{sw}px" style:height="{vy}px"></div>
          <div class="shade" style:left="0" style:top="{vy + win[1]}px" style:width="{sw}px" style:height="{Math.max(0, sh - vy - win[1])}px"></div>
          <div class="shade" style:left="0" style:top="{vy}px" style:width="{vx}px" style:height="{win[1]}px"></div>
          <div class="shade" style:left="{vx + win[0]}px" style:top="{vy}px" style:width="{Math.max(0, sw - vx - win[0])}px" style:height="{win[1]}px"></div>
          <div class="frame" class:round={!d.banner} style:left="{vx}px" style:top="{vy}px" style:width="{win[0]}px" style:height="{win[1]}px"></div>
        </div>
        <div class="slider">
          <Icon name="image" size={14} color="rgba(255,255,255,.6)" />
          <ThinBar width={220} value={frac} onpick={zoomTo} />
          <Icon name="image" size={20} color="rgba(255,255,255,.6)" />
        </div>
        <div class="foot">
          <Button kind="text" class="b36" onclick={reset}>{t("profile.crop.reset")}</Button>
          <span class="sp"></span>
          <Button class="b36" onclick={cancelCrop}>{t("dialog.cancel")}</Button>
          <Button kind="primary" class="b36" onclick={applyCrop}>{t("profile.crop.apply")}</Button>
        </div>
      </div>
    </ModalLayer>
  </div>
{/if}

<style>
  .panel { padding: 24px; border-radius: 12px; background: rgba(23, 23, 23, .99); display: flex; flex-direction: column; gap: 18px; font-family: var(--font); }
  .head { display: flex; align-items: center; }
  .title { flex: 1; font-size: 20px; font-weight: 700; color: rgba(255, 255, 255, .95); }
  .stage { position: relative; border-radius: 8px; overflow: hidden; background: rgba(255, 255, 255, .04); cursor: grab; touch-action: none; }
  .stage:active { cursor: grabbing; }
  .stage img { position: absolute; max-width: none; pointer-events: none; }
  .shade { position: absolute; background: rgba(0, 0, 0, .6); pointer-events: none; }
  .frame { position: absolute; border: 2px solid rgba(255, 255, 255, .95); border-radius: 8px; pointer-events: none; }
  .frame.round { border-radius: 50%; }
  .slider { display: flex; align-items: center; justify-content: center; gap: 12px; }
  .foot { display: flex; align-items: center; gap: 8px; }
  .sp { flex: 1; }
  .foot :global(.b36) { height: 36px; }
  .foot :global(.text.b36) { font-size: var(--sm); }
  .foot :global(.default.b36), .foot :global(.primary.b36) { padding: 0 18px; }
</style>
