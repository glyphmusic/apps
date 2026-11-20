<script lang="ts">
  // The glow field behind everything (visuals/glow.rs): theme glow colours,
  // the playing song's palette (from its cover), a new shape per song unless
  // one is fixed in Settings > Appearance. A background picture (own or a
  // plugin's, backgrounds.svelte.ts) replaces the glow: darkened and
  // optionally blurred, text ink from its brightness.
  import { untrack } from "svelte";
  import { Glow, paletteFromImage, paletteFromSeed, presetFromSeed, hex, solidIsDarkInk, type GlowShape, type Palette, type RenderMode } from "../glow";
  import { app } from "../state.svelte";
  import { THEMES, solidBase, themeColors } from "../theme";
  import { backend } from "../api/backend";
  import { trackKey } from "../api/types";
  import { fx, setGlow } from "../ui.svelte";
  import { t } from "../i18n.svelte";
  import { bgUrl, bgs, loadBackgrounds, pictureLuma } from "../backgrounds.svelte";

  let { blurred = false }: { blurred?: boolean } = $props();

  let glowEl = $state<HTMLCanvasElement | undefined>();
  let asciiEl = $state<HTMLCanvasElement | undefined>();
  let glow = $state<Glow | null>(null);

  // The picture picked in Settings > Appearance (null while unknown or gone).
  const picId = $derived(app.config?.visuals.bg_image || "");
  $effect(() => { if (picId && !bgs.loaded) void loadBackgrounds(); });
  const picUrl = $derived(picId ? bgUrl(picId) : null);
  const picDim = $derived(Math.min(0.9, Math.max(0, app.config?.visuals.bg_dim ?? 0.35)));
  const picBlur = $derived(Math.min(1, Math.max(0, app.config?.visuals.bg_blur ?? 0)));
  const solid = $derived(!app.config || app.config.visuals.glow_shape === "solid" || !!picUrl);
  // Settings > Appearance > Animations. A canvas keeps the kind of drawing it
  // started with, so a switch brings new canvases (the {#key} below). A
  // graphics card that failed here stays off (`gpu_blocked`).
  const renderMode = $derived<RenderMode>(app.config?.visuals.render_mode === "cpu" || app.config?.visuals.gpu_blocked ? "cpu" : "gpu");

  /** The graphics card failed: stay on the processor on this device and say so once. */
  function gpuFailed() {
    if (!app.config || app.config.visuals.gpu_blocked) return;
    void app.setConfig((c) => { c.visuals.render_mode = "cpu"; c.visuals.gpu_blocked = true; });
    app.toast(t("toast.gpu_fallback"), "info");
  }
  const baseColor = $derived(solidBase(app.config?.visuals));
  const baseCss = $derived(`#${baseColor.toString(16).padStart(6, "0")}`);

  $effect(() => {
    if (solid || !glowEl || !asciiEl) {
      glow = null;
      return;
    }
    const g = new Glow(glowEl, asciiEl, untrack(() => renderMode));
    g.onGpuFail = () => untrack(gpuFailed);
    if (g.failReason) queueMicrotask(gpuFailed);
    glow = g;
    setGlow(g);
    return () => {
      g.destroy();
      glow = null;
    };
  });

  // Text ink on a picture: dark only when it stays bright after darkening.
  let picLuma = $state<number | null>(null);
  $effect(() => {
    const u = picUrl;
    picLuma = null;
    if (!u) return;
    let alive = true;
    void pictureLuma(u).then((l) => { if (alive) picLuma = l; });
    return () => { alive = false; };
  });
  const staticInk = $derived(picUrl ? (picLuma ?? 0) * (1 - picDim) > 0.62 : solidIsDarkInk(baseColor));

  $effect(() => {
    if (solid) setGlow(null, staticInk);
  });

  $effect(() => {
    if (solid) {
      setGlow(null, staticInk);
      return;
    }
    if (!glow) return;
    const compact = app.compact;
    const v = app.config!.visuals;
    const c = themeColors(v.theme);
    glow.opts = { intensity: v.intensity, reactivity: v.music_reactivity, throttleUnfocused: v.throttle_unfocused, reduce: fx.reduce, titlebarDots: !compact, solid: false, base: c.base, perf: !!v.performance_mode };
    glow.refreshLayout();
    glow.setTheme(c.glow ? (c.glow.map(hex) as Palette) : null);
  });

  $effect(() => { glow?.setPaused(app.splash || app.settingsOpen || app.onboarding); });
  $effect(() => { if (glow) glow.playing = app.playing; });

  // Shape and colours follow the song (skipped completely in solid mode).
  let lastTrackKey = "";
  $effect(() => {
    if (solid || !glow || !app.config) {
      lastTrackKey = "";
      return;
    }
    const g = glow;
    const cur = app.player.current, fixed = app.config.visuals.glow_shape as GlowShape;
    const key = cur ? trackKey(cur) : "";
    g.setPreset((fixed as Exclude<GlowShape, "" | "solid">) || (cur ? presetFromSeed(key) : "strahl"));
    if (key === lastTrackKey) return;
    lastTrackKey = key;
    if (!cur) { g.setPalette(null); return; }
    g.kick();
    let alive = true;
    backend().cover(cur).then(async (u) => {
      const p = (u && (await paletteFromImage(u))) || paletteFromSeed(key);
      if (alive && glow === g) g.setPalette(p);
    });
    return () => { alive = false; };
  });
  void THEMES;
</script>

{#if picUrl}
  <div class="bg solid" style:background={baseCss}></div>
  <div class="bg pic" class:blurred style:background-image={`url("${picUrl}")`} style:--pic-blur="{Math.round(picBlur * 40)}px" style:--pic-scale={picBlur > 0 ? 1 + picBlur * 0.08 : 1}></div>
  <div class="bg pic-dim" style:opacity={picDim}></div>
{:else if solid}
  <div class="bg solid" style:background={baseCss}></div>
{:else}
  {#key renderMode}
    <canvas class="bg glow" class:blurred bind:this={glowEl}></canvas>
    <canvas class="bg ascii" class:blurred bind:this={asciiEl}></canvas>
  {/key}
{/if}

<style>
  .bg { position: fixed; inset: 0; width: 100%; height: 100%; pointer-events: none; contain: strict; transition: filter .18s ease-out; }
  .bg.solid { z-index: 0; transition: none; }
  /* The picture fills the window; blur reaches past the edges so they stay sharp-free. */
  .bg.pic { z-index: 0; background-size: cover; background-position: center; filter: blur(var(--pic-blur, 0px)); transform: scale(var(--pic-scale, 1)); }
  .bg.pic.blurred { filter: blur(calc(var(--pic-blur, 0px) + 14px)) saturate(1.1); }
  .bg.pic-dim { z-index: 0; background: #000; transition: opacity .18s ease-out; }
  .bg.blurred { filter: blur(14px) saturate(1.1); }
  .glow { z-index: 0; image-rendering: auto; }
  .ascii { z-index: 0; image-rendering: pixelated; }
</style>
