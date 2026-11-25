<script lang="ts">
  // ui/splash.rs Splash::element / paint: ASCII characters stream in from
  // beyond the window edges in six curved currents and assemble the Glyph
  // note in the centre; then the black lifts off (2.45 s in, 0.45 s out).
  // Every character is real text on a canvas (bold mono, size row_h x 0.95,
  // row_h = 26 % of the smaller window side / 29 rows, clamped 5..11 px;
  // white 96 %, pink in the accent). A click skips; when done
  // `app.splash = false`. With reduced motion it does not play at all.
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { fx } from "../ui.svelte";
  import { ART, CELL_ASPECT, currentSplash, easeInOut, makeParticles, pathAt, REST, smooth, T_OUT, T_OUT_START, TOTAL, TRAIL } from "./splash-art";

  let cv: HTMLCanvasElement;
  // Glyph's note, or a plugin look's picture (remembered from the last start).
  const art = currentSplash();
  const COLS = art.cols, ROWS = art.rows;
  const parts = makeParticles(art);

  onMount(() => {
    if (fx.reduce) { app.splash = false; return; }
    const ctx = cv.getContext("2d");
    if (!ctx) { app.splash = false; return; }
    let start: number | null = null;
    let raf = 0, done = false;

    const finish = () => {
      if (done) return;
      done = true;
      clearTimeout(safetyTimer);
      cancelAnimationFrame(raf);
      app.splash = false;
    };
    const safetyTimer = window.setTimeout(finish, (TOTAL / art.speed + 1.5) * 1000);
    skip = finish;

    // Pre-render the 5 unique characters in white and accent once per scale
    // so each frame uses fast drawImage blits instead of 1,200+ fillText calls.
    let cacheKey = "";
    let cachePad = 0;
    let cacheSw = 0;
    let cacheSh = 0;
    const sprites = new Map<string, HTMLCanvasElement>();
    const buildSprites = (rowH: number, cellW: number, dpr: number) => {
      const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim() || "#ff5fd2";
      const fontSpec = `700 ${rowH * 0.95}px "JetBrains Mono", Consolas, monospace`;
      const fontReady = document.fonts ? document.fonts.check(fontSpec) : true;
      const key = `${rowH.toFixed(2)}:${dpr}:${accent}:${fontReady}`;
      if (key === cacheKey) return true;
      cacheKey = key;
      sprites.clear();
      cachePad = Math.ceil(rowH * 0.2);
      cacheSw = Math.ceil(cellW) + cachePad * 2;
      cacheSh = Math.ceil(rowH) + cachePad * 2;
      for (const p of parts) {
        const k = `${p.ch}:${p.pink ? 1 : 0}`;
        if (sprites.has(k)) continue;
        const sc = document.createElement("canvas");
        sc.width = Math.max(1, Math.round(cacheSw * dpr));
        sc.height = Math.max(1, Math.round(cacheSh * dpr));
        const sctx = sc.getContext("2d");
        if (!sctx) return false;
        sctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        sctx.font = fontSpec;
        sctx.textBaseline = "top";
        sctx.fillStyle = p.pink ? accent : "#f5f5f5";
        sctx.fillText(p.ch, cachePad, cachePad);
        sprites.set(k, sc);
      }
      return true;
    };

    const draw = (t: number) => {
      try {
        const dpr = devicePixelRatio || 1, w = Math.max(1, innerWidth), h = Math.max(1, innerHeight);
        if (cv.width !== Math.round(w * dpr) || cv.height !== Math.round(h * dpr)) { cv.width = Math.round(w * dpr); cv.height = Math.round(h * dpr); }
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.globalAlpha = 1;
        ctx.clearRect(0, 0, w, h);
        const fade = 1 - smooth((t - T_OUT_START) / T_OUT);
        if (fade <= 0 || t >= TOTAL) { finish(); return; }
        ctx.fillStyle = `rgba(0,0,0,${fade})`;
        ctx.fillRect(0, 0, w, h);

        // The note is about a quarter of the smaller window side tall.
        // A look's smaller picture may draw bigger characters than the note.
        const rowH = Math.min(art.art === ART ? 11 : 28, Math.max(5, (Math.min(w, h) * 0.26) / ROWS));
        const cellW = rowH / CELL_ASPECT, cellH = rowH;
        if (!buildSprites(rowH, cellW, dpr)) { finish(); return; }
        const cx = w / 2, cy = h / 2;
        const o = { cx, cy, reach: Math.max(w, h) * 0.78, lane: 120, left: cx - (COLS * cellW) / 2, top: cy - (ROWS * cellH) / 2, cellW, cellH };
        for (const p of parts) {
          const raw = Math.min(1, Math.max(0, (t - p.delay) / p.dur));
          if (raw <= 0) continue;
          const e = easeInOut(raw);
          const sp = sprites.get(`${p.ch}:${p.pink ? 1 : 0}`);
          if (!sp) continue;
          for (const [back, a] of raw < 1 ? TRAIL : REST) {
            const step = Math.round(Math.min(1, Math.max(0, a * fade)) * 12) / 12;
            if (step <= 0) continue;
            const [x, y] = pathAt(p, Math.max(0, e - back), o);
            ctx.globalAlpha = step;
            ctx.drawImage(sp, x - cachePad, y - cachePad, cacheSw, cacheSh);
          }
        }
        ctx.globalAlpha = 1;
      } catch {
        finish();
      }
    };
    const frame = (now: number) => {
      if (start === null) start = now;
      draw(((now - start) / 1000) * art.speed);
      if (!done) raf = requestAnimationFrame(frame);
    };
    // Paint t = 0 immediately on mount so the background never flashes before the first rAF.
    draw(0);
    if (!done) raf = requestAnimationFrame(frame);
    return () => { done = true; clearTimeout(safetyTimer); cancelAnimationFrame(raf); };
  });

  let skip: () => void = () => { app.splash = false; };
</script>

<div class="sp" onpointerdown={() => skip()} role="presentation">
  <canvas bind:this={cv}></canvas>
</div>

<style>
  .sp { position: fixed; inset: 0; z-index: 80; cursor: default; }
  canvas { width: 100%; height: 100%; display: block; }
</style>
