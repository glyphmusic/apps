<script lang="ts">
  // The glow field behind everything (visuals/glow.rs): theme glow colours,
  // the playing song's palette (from its cover), a new shape per song unless
  // one is fixed in Settings > Appearance.
  import { Glow, paletteFromImage, paletteFromSeed, presetFromSeed, hex, solidIsDarkInk, type GlowShape, type Palette } from "../glow";
  import { app } from "../state.svelte";
  import { THEMES, themeColors } from "../theme";
  import { backend } from "../api/backend";
  import { trackKey } from "../api/types";
  import { fx, setGlow } from "../ui.svelte";

  let { blurred = false }: { blurred?: boolean } = $props();

  let glowEl = $state<HTMLCanvasElement | undefined>();
  let asciiEl = $state<HTMLCanvasElement | undefined>();
  let glow = $state<Glow | null>(null);

  const solid = $derived(!app.config || app.config.visuals.glow_shape === "solid");
  const baseColor = $derived(themeColors(app.config?.visuals.theme ?? "glyph").base);
  const baseCss = $derived(`#${baseColor.toString(16).padStart(6, "0")}`);

  $effect(() => {
    if (solid || !glowEl || !asciiEl) {
      glow = null;
      return;
    }
    const g = new Glow(glowEl, asciiEl);
    glow = g;
    setGlow(g);
    return () => {
      g.destroy();
      glow = null;
    };
  });

  $effect(() => {
    if (solid) setGlow(null, solidIsDarkInk(baseColor));
  });

  $effect(() => {
    if (solid) {
      setGlow(null, solidIsDarkInk(baseColor));
      return;
    }
    if (!glow) return;
    const compact = app.compact;
    const v = app.config!.visuals;
    const c = themeColors(v.theme);
    glow.opts = { intensity: v.intensity, reactivity: v.music_reactivity, throttleUnfocused: v.throttle_unfocused, reduce: fx.reduce, titlebarDots: !compact, solid: false, base: c.base };
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

{#if solid}
  <div class="bg solid" style:background={baseCss}></div>
{:else}
  <canvas class="bg glow" class:blurred bind:this={glowEl}></canvas>
  <canvas class="bg ascii" class:blurred bind:this={asciiEl}></canvas>
{/if}

<style>
  .bg { position: fixed; inset: 0; width: 100%; height: 100%; pointer-events: none; contain: strict; transition: filter .18s ease-out; }
  .bg.solid { z-index: 0; transition: none; }
  .bg.blurred { filter: blur(14px) saturate(1.1); }
  .glow { z-index: 0; image-rendering: auto; }
  .ascii { z-index: 0; image-rendering: pixelated; }
</style>
