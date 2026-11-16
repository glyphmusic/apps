<script lang="ts">
  // now_playing.rs lyrics: the lyrics frame. Synced lines are direct
  // children of the scroll frame so it can glide to the active line (a
  // critically damped spring, w 9 rad/s, ~0.45 s) and keep it in the middle;
  // a hand scroll pauses that for 4 s. Lines: 28 / lh 38 / py 10 bold; the
  // sung line grows to 36 / 47 / py 12 and its sung part fills white with a
  // soft edge that glides across (eased 0.35 per frame); opacity: active 1,
  // upcoming .5 - .06 per line (min .22), sung .62 - .08 per line (min .3),
  // all easing; lines fade out towards the frame's top and bottom edge (96
  // px, smoothstep). Click a line = seek there. Plain lyrics: 18 px dim.
  // Top: view switch left, options button right (h 34, px 11, radius 10,
  // sliders icon + provider + offset; white when open) with the options
  // panel. Loading: skeleton lines (gap 26). None: empty state + find.
  import Button from "../../components/Button.svelte";
  import EmptyState from "../../components/EmptyState.svelte";
  import Skeleton from "../../components/Skeleton.svelte";
  import LyricsSource from "./LyricsSource.svelte";
  import { app } from "../../state.svelte";
  import { t, tf } from "../../i18n.svelte";
  import { fadeEdges } from "../../ui.svelte";
  import { lyr, syncKey, loadAlternatives, pick, offset } from "./lyrics-state.svelte";

  let { compact = false }: { compact?: boolean } = $props();
  const centered = $derived(app.lyricsOnly);
  const l = $derived(app.lyrics.lyrics);
  const off = $derived(offset());
  let frame: HTMLDivElement | undefined = $state();
  let frameH = $state(600);
  let lineEls: HTMLElement[] = [];
  let sweepEl: HTMLSpanElement | undefined = $state();
  let userAt = -1e9;
  let fill = $state(0);

  $effect(() => { void app.lyrics.key; syncKey(); });

  // ---- active line, sweep ----
  const posMs = $derived(app.position * 1000 + off);
  const lines = $derived(l?.lines ?? []);
  const synced = $derived(!!l?.synced && lines.some((x) => x.time_ms != null));
  const nextMs = $derived.by(() => {
    const out = new Array<number>(lines.length);
    let nextTime = 0;
    for (let i = lines.length - 1; i >= 0; i--) {
      const tm = lines[i].time_ms;
      out[i] = nextTime > 0 ? nextTime : (tm != null ? tm + 4000 : 4000);
      if (tm != null) nextTime = tm;
    }
    return out;
  });
  const active = $derived.by(() => {
    if (!synced) return null;
    let a: number | null = null;
    for (let i = 0; i < lines.length; i++) { const tm = lines[i].time_ms; if (tm != null && tm <= posMs) a = i; else if (tm != null) break; }
    return a;
  });

  // ---- unified, on-demand rAF for sweep + scroll spring (zero work when paused/settled/hidden) ----
  let fillV = 0, fillLine = -1, lastPct = -1;
  let vel = 0, lastT = 0;
  let cachedLine = -1, cachedFrameH = -1, cachedTarget = 0, retargetUntil = 0, curScroll = 0;

  $effect(() => {
    const f = frame;
    const isSynced = synced;
    const a = active;
    const isPlaying = app.playing;
    const curLines = lines;
    const curOff = off;
    const curNextMs = nextMs;
    const fh = frameH;
    if (!f || !isSynced || a == null) {
      fillV = 0;
      fillLine = -1;
      lastPct = -1;
      vel = 0;
      fill = 0;
      return;
    }
    if (a !== cachedLine || fh !== cachedFrameH) {
      cachedLine = a;
      cachedFrameH = fh;
      retargetUntil = performance.now() + 340;
      lastT = 0;
    }
    let raf = 0;
    let alive = true;
    const measureTarget = (now: number) => {
      if (now <= retargetUntil || lastT === 0) {
        const el = lineEls[a];
        if (el) {
          const vh = f.clientHeight, max = Math.max(0, f.scrollHeight - vh);
          cachedTarget = Math.min(max, Math.max(0, el.offsetTop + el.offsetHeight / 2 - vh * 0.5));
        }
      }
      curScroll = f.scrollTop;
    };
    const step = () => {
      raf = 0;
      if (!alive || document.hidden) return;
      const now = performance.now();
      const t0 = curLines[a]?.time_ms;
      let fillBusy = false;
      if (t0 != null) {
        const pNow = (app.positionMsNow() + curOff - t0) / Math.max(1, curNextMs[a] - t0);
        const p = Math.min(1, Math.max(0, pNow));
        const nextFill = fillLine !== a || p < fillV - 0.2 ? p : fillV + (p - fillV) * 0.35;
        fillV = nextFill;
        fillLine = a;
        // Quantize to 0.5% steps so Chromium/WebView2 doesn't re-rasterize clipped gradient text at 144 Hz.
        const pct = Math.round(fillV * 200) / 2;
        if (Math.abs(pct - lastPct) >= 0.5 || fillLine !== a) {
          lastPct = pct;
          if (sweepEl) sweepEl.style.setProperty("--p", String(pct));
          fill = pct / 100;
        }
        fillBusy = isPlaying || Math.abs(p - fillV) > 0.005;
      }

      let scrollBusy = false;
      if (now - userAt >= 4000) {
        measureTarget(now);
        const dt = Math.min(0.05, lastT ? (now - lastT) / 1000 : 0.016);
        lastT = now;
        const w = 9, x = curScroll - cachedTarget;
        vel += (-w * w * x - 2 * w * vel) * dt;
        let next = curScroll + vel * dt;
        if (Math.abs(next - cachedTarget) < 0.4 && Math.abs(vel) < 4) {
          next = cachedTarget;
          vel = 0;
        } else {
          scrollBusy = true;
        }
        if (Math.abs(next - curScroll) > 0.01) f.scrollTop = next;
        if (now <= retargetUntil) scrollBusy = true;
      } else {
        vel = 0;
        lastT = now;
      }

      if (fillBusy || scrollBusy) {
        raf = requestAnimationFrame(step);
      }
    };
    const onVis = () => { if (!document.hidden && !raf) raf = requestAnimationFrame(step); };
    document.addEventListener("visibilitychange", onVis);
    raf = requestAnimationFrame(step);
    return () => {
      alive = false;
      cancelAnimationFrame(raf);
      document.removeEventListener("visibilitychange", onVis);
    };
  });
  const touched = () => (userAt = performance.now());
  function seekTo(ms: number) { userAt = -1e9; app.seek(Math.max(0, (ms - off) / 1000)); }

  const alpha = (i: number) => {
    const a = active;
    if (a == null) return 0.42;
    if (i === a) return 1;
    return i > a ? Math.max(0.22, 0.5 - 0.06 * (i - a - 1)) : Math.max(0.3, 0.62 - 0.08 * (a - i - 1));
  };
  const SK = [0.62, 0.48, 0.7, 0.4, 0.56, 0.66, 0.44];
  const half = $derived(Math.max(120, frameH / 2 - 30));
  const n = $derived(Math.max(1, (lines[active ?? 0]?.text ?? "").length));
</script>

<div class="ly">
  {#if compact}
    <div class="info">
      <span class="fl"></span>
      <LyricsSource />
    </div>
  {/if}

  {#if app.lyrics.loading}
    <div class="sk">
      {#each SK as w, i}<div style:width="{w * 100}%"><Skeleton w="100%" h={24} radius={6} id="lyr-sk-{i}" /></div>{/each}
    </div>
  {:else if !l}
    <div class="none">
      <EmptyState title={t("lyrics.none.title")} sub={lyr.error ?? t("lyrics.none.sub")} />
      {#if !lyr.loaded || lyr.alts.length > 0}
        <div class="find">
          {#if lyr.alts.length}<Button onclick={() => pick(0)}>{tf("lyrics.alts.found", { n: lyr.alts.length })}</Button>
          {:else if lyr.loading}<span class="note">{t("lyrics.alts.loading")}</span>
          {:else}<Button onclick={loadAlternatives}>{t("lyrics.alts.find")}</Button>{/if}
        </div>
      {/if}
    </div>
  {:else}
    <div class="frame scroll" class:centered class:synced bind:this={frame} bind:clientHeight={frameH}
      use:fadeEdges={{ top: 32, bottom: 32, active: !synced }}
      style:padding-top="{synced ? half : 12}px" style:padding-bottom="{synced ? half : 24}px"
      onwheel={touched} ontouchmove={touched} onpointerdown={touched} role="presentation">
      {#if synced}
        {#each lines as ln, i (i)}
          {@const on = i === active}
          <div class="line" class:on class:compact bind:this={lineEls[i]} style:--a={alpha(i)} onclick={() => ln.time_ms != null && seekTo(ln.time_ms)} onkeydown={(e) => e.key === "Enter" && ln.time_ms != null && seekTo(ln.time_ms)} role="button" tabindex="-1">
            {#if on}<span class="sweep" bind:this={sweepEl} style:--p={fill * 100} style:--e={(3 / n) * 50} style:--rest={0.42 * Math.max(alpha(i), 0.5)}>{ln.text.trim() ? ln.text : "♪"}</span>{:else}{ln.text.trim() ? ln.text : "♪"}{/if}
          </div>
        {/each}
      {:else}
        <div class="plain" class:centered>
          {#each lines as ln}<div>{ln.text}</div>{/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .ly { position: relative; flex: 1; min-width: 0; height: 100%; display: flex; flex-direction: column; }
  .info { flex: none; z-index: 20; padding: 12px 0 8px; display: flex; align-items: center; gap: 8px; }
  .fl { flex: 1; }
  .frame { position: relative; flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .frame.synced { -webkit-mask-image: linear-gradient(transparent, #000 96px, #000 calc(100% - 96px), transparent); mask-image: linear-gradient(transparent, #000 96px, #000 calc(100% - 96px), transparent); }
  .frame.centered { align-items: center; }
  .line { flex: none; width: 100%; padding: 10px 0; cursor: pointer; font-weight: 700; font-size: 28px; line-height: 38px; contain: layout style; color: rgba(var(--ink), var(--a)); transition: color .32s ease-out, font-size .28s ease-out, line-height .28s ease-out, padding .28s ease-out; }
  .line.compact { font-size: 24px; line-height: 32px; }
  .centered .line { text-align: center; }
  .line:not(.on):hover { color: rgba(var(--ink), .8); }
  .line.on { padding: 12px 0; font-size: 36px; line-height: 47px; }
  .line.on.compact { font-size: 28px; line-height: 37px; }
  .sweep { color: transparent; -webkit-background-clip: text; background-clip: text; background-image: linear-gradient(90deg, rgb(var(--ink)) 0, rgb(var(--ink)) calc(var(--p) * 1% - var(--e) * 1%), rgba(var(--ink), var(--rest)) calc(var(--p) * 1% + var(--e) * 1%), rgba(var(--ink), var(--rest)) 100%); }
  .plain { display: flex; flex-direction: column; gap: 8px; font-size: 18px; color: var(--dim); }
  .plain.centered { align-items: center; text-align: center; }
  .sk { padding-top: 24px; display: flex; flex-direction: column; gap: 26px; }
  .none { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; padding-top: 0; }
  .none :global(.es) { align-self: center; }
  .find { padding-left: 0; display: flex; justify-content: center; }
  .note { font-size: var(--sm); color: var(--dim); }
</style>
