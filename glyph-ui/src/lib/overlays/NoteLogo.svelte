<script lang="ts">
  // splash.rs note_logo / note_logo_flying / paint_logo: the note as a live
  // logo (onboarding welcome page, Settings > About). With `flyIn` the
  // characters stream in and assemble first, like the splash, inside the
  // box; then they react to the mouse: characters near the cursor are
  // pushed out of a bubble (4.5 rows), a spring (0.06) pulls them home and
  // they push each other apart; now and then one flickers in the accent.
  import { onMount } from "svelte";
  import { fx } from "../ui.svelte";
  import { CELL_ASPECT, COLS, easeInOut, makeParticles, pathAt, REST, ROWS, rnd, TRAIL, charColor } from "./splash-art";

  let { rowH, flyIn = false }: { rowH: number; flyIn?: boolean } = $props();
  const cellW = $derived(rowH / CELL_ASPECT);
  const pad = $derived(rowH * 2);
  const W = $derived(COLS * cellW + pad * 2), H = $derived(ROWS * rowH + pad * 2);
  let cv = $state<HTMLCanvasElement>();
  const parts = makeParticles();
  const settled = Math.max(...parts.map((p) => p.delay + p.dur));
  const FLICKER = [".", "-", "+", "#", "0"];

  onMount(() => {
    const c = cv!;
    const ctx = c.getContext("2d")!;
    const readAccent = () =>
      document.documentElement.style.getPropertyValue("--accent").trim() ||
      getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
    let accent = readAccent();
    const still = fx.reduce;
    const born = performance.now() - (flyIn && !still ? 0 : settled * 1000);
    const off = new Float32Array(parts.length * 4);
    const glitch = parts.map(() => ({ until: 0, ch: " " }));
    let seed = 1, raf = 0;
    let mouse: [number, number] | null = null;
    const move = (e: PointerEvent) => { const r = c.getBoundingClientRect(); mouse = [e.clientX - r.left, e.clientY - r.top]; };
    const leave = () => (mouse = null);
    c.addEventListener("pointermove", move);
    c.addEventListener("pointerleave", leave);

    const frame = () => {
      const rh = rowH, cw = rh / CELL_ASPECT, w = COLS * cw + rh * 4, h = ROWS * rh + rh * 4;
      const dpr = devicePixelRatio || 1;
      if (c.width !== Math.round(w * dpr)) { c.width = Math.round(w * dpr); c.height = Math.round(h * dpr); }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      const t = still ? settled : (performance.now() - born) / 1000;
      const o = { cx: w / 2, cy: h / 2, reach: Math.max(w, h) * 0.9, lane: 40, left: w / 2 - (COLS * cw) / 2, top: h / 2 - (ROWS * rh) / 2, cellW: cw, cellH: rh };

      if (!still) {
        // Flicker: now and then one character turns into another for a moment.
        seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
        if (t > settled && ((seed >>> 8) % 100) < 18) {
          const i = (seed >>> 3) % parts.length;
          glitch[i] = { until: t + 0.12 + rnd(seed, 9) * 0.2, ch: FLICKER[(seed >>> 13) % FLICKER.length] };
        }
        // Spring home, cursor bubble, characters pushing each other apart.
        const bubble = rh * 4.5, gap = cw * 0.95;
        const pos = parts.map((p, i) => {
          const e = easeInOut(Math.min(1, Math.max(0, (t - p.delay) / p.dur)));
          const [x, y] = pathAt(p, e, o);
          return [x + off[i * 4] + cw / 2, y + off[i * 4 + 1] + rh / 2] as const;
        });
        for (let i = 0; i < parts.length; i++) {
          const k = i * 4;
          off[k + 2] -= off[k] * 0.06; off[k + 3] -= off[k + 1] * 0.06;
          const [x, y] = pos[i];
          if (mouse) {
            const dx = x - mouse[0], dy = y - mouse[1], d = Math.max(0.01, Math.hypot(dx, dy));
            if (d < bubble) { const f = ((bubble - d) / bubble) * 2.2; off[k + 2] += (dx / d) * f; off[k + 3] += (dy / d) * f; }
          }
          if (t >= settled && (Math.abs(off[k]) > 0.05 || Math.abs(off[k + 1]) > 0.05 || mouse)) {
            for (let j = 0; j < parts.length; j++) {
              const dx = x - pos[j][0], dy = y - pos[j][1];
              if (j === i || Math.abs(dx) >= gap || Math.abs(dy) >= gap) continue;
              const d = Math.hypot(dx, dy);
              if (d < gap && d > 0.001) { const f = ((gap - d) / gap) * 0.5; off[k + 2] += (dx / d) * f; off[k + 3] += (dy / d) * f; }
            }
          }
        }
        for (let i = 0; i < parts.length; i++) {
          const k = i * 4;
          off[k + 2] *= 0.78; off[k + 3] *= 0.78;
          off[k] += off[k + 2]; off[k + 1] += off[k + 3];
        }
      }

      ctx.font = `700 ${rh * 0.95}px "JetBrains Mono", Consolas, monospace`;
      ctx.textBaseline = "top";
      parts.forEach((p, i) => {
        const raw = Math.min(1, Math.max(0, (t - p.delay) / p.dur));
        if (raw <= 0) return;
        const e = easeInOut(raw), glitching = glitch[i].until > t;
        const ch = glitching ? glitch[i].ch : p.ch, pink = p.pink || glitching;
        for (const [back, a] of raw < 1 ? TRAIL : REST) {
          const col = charColor(pink, a, accent);
          if (!col) continue;
          const [x, y] = pathAt(p, Math.max(0, e - back), o);
          ctx.fillStyle = col;
          ctx.fillText(ch, x + off[i * 4], y + off[i * 4 + 1]);
        }
      });
      if (!still) raf = requestAnimationFrame(frame);
    };
    const obs = new MutationObserver(() => {
      accent = readAccent();
      if (still) frame();
    });
    obs.observe(document.documentElement, { attributes: true, attributeFilter: ["style"] });
    frame();
    if (!still) raf = requestAnimationFrame(frame);
    return () => {
      obs.disconnect();
      cancelAnimationFrame(raf);
      c.removeEventListener("pointermove", move);
      c.removeEventListener("pointerleave", leave);
    };
  });
</script>

<canvas bind:this={cv} style:width="{W}px" style:height="{H}px"></canvas>

<style>
  canvas { display: block; flex: none; }
</style>
