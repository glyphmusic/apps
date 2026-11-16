<script lang="ts">
  // compact.rs mini_bar: h 64, radius 12, hsla(0 0 6% / .94), cover 48,
  // title md medium / artist sm dim, like toggle + plain play icon,
  // and a 2 px progress line along the bottom. Swipe sideways to reveal &
  // skip to prev/next track, up (or tap) to open Now Playing.
  import Cover from "../components/Cover.svelte";
  import SongTitle from "../components/SongTitle.svelte";
  import PlayGlyph from "../components/PlayGlyph.svelte";
  import ToggleButton from "../components/ToggleButton.svelte";
  import { app } from "../state.svelte";
  import { press } from "../ui.svelte";
  import { rating, trackKey, type Track } from "../api/types";
  import { phone, drag, SWIPE_ACT } from "./cstate.svelte";

  type Slides = { prev: Track | null; cur: Track; next: Track | null };
  let sx = $state(0);
  let ow = $state(240);
  let settling = $state(false);
  let frozen = $state<(Slides & { fromKey: string; doneAt: number }) | null>(null);
  let dirLock: "x" | "y" | null = null;
  let timer = 0;

  const cur = $derived(app.player.current!);
  const idx = $derived(app.queue.index);
  const prev = $derived(idx != null && idx > 0 ? app.queue.items[idx - 1] ?? null : null);
  const next = $derived(app.upNext[0]?.track ?? (app.player.repeat === "all" && app.queue.items.length > 1 ? app.queue.items[0] : null));
  const frac = $derived(app.player.duration ? Math.min(1, app.position / app.player.duration) : 0);
  const viewPrev = $derived(frozen ? frozen.prev : prev);
  const viewCur = $derived(frozen ? frozen.cur : cur);
  const viewNext = $derived(frozen ? frozen.next : next);

  $effect(() => {
    const f = frozen;
    if (!f) return;
    if (trackKey(cur) !== f.fromKey) {
      const wait = Math.max(0, f.doneAt - performance.now());
      const t = setTimeout(() => { settling = false; sx = 0; frozen = null; }, wait);
      return () => clearTimeout(t);
    }
  });

  function move(dx: number, dy: number) {
    if (!dirLock) dirLock = Math.abs(dx) > Math.abs(dy) ? "x" : "y";
    if (dirLock !== "x") return;
    if (frozen) { clearTimeout(timer); frozen = null; }
    settling = false;
    const damp = (dx > 0 && !prev) || (dx < 0 && !next);
    sx = damp ? dx * 0.35 : dx;
  }

  function end(dx: number, dy: number, vx: number, vy: number) {
    const lock = dirLock;
    dirLock = null;
    if (lock === "x" || Math.abs(dx) > Math.abs(dy)) {
      const goNext = !!next && (dx <= -SWIPE_ACT || (vx < -0.4 && dx < -18));
      const goPrev = !!prev && (dx >= SWIPE_ACT || (vx > 0.4 && dx > 18));
      if (goNext || goPrev) {
        const w = Math.max(120, ow);
        frozen = { prev, cur, next, fromKey: trackKey(cur), doneAt: performance.now() + 200 };
        settling = true;
        sx = goNext ? -w : w;
        clearTimeout(timer);
        timer = window.setTimeout(() => { settling = false; sx = 0; frozen = null; }, 650);
        if (goNext) app.next();
        else if (idx != null && idx > 0) app.queueJump(idx - 1);
        else app.previous();
        return;
      }
      settling = true;
      sx = 0;
      clearTimeout(timer);
      timer = window.setTimeout(() => (settling = false), 220);
    } else if (dy < -SWIPE_ACT * 0.5 || (vy < -0.35 && dy < -14)) {
      phone.npOpen = true;
    }
  }
</script>

{#snippet slide(tr: Track, offset: string)}
  <span class="in" style:transform="translateX({offset})">
    <Cover track={tr} size={48} radius={6} />
    <span class="tx">
      <span class="t"><SongTitle rating={rating(tr)}>{tr.title}</SongTitle></span>
      <span class="a trunc">{tr.artist}</span>
    </span>
  </span>
{/snippet}

<div class="mini" role="group" use:drag={{ axis: "any", move, end }}
  oncontextmenu={(e) => { e.preventDefault(); e.stopPropagation(); app.openCtx({ x: e.clientX, y: e.clientY, track: cur }); }}>
  <button class="open" bind:clientWidth={ow} onclick={() => (phone.npOpen = true)}>
    <span class="strip" class:settling style:transform="translateX({sx}px)">
      {#if viewPrev}{@render slide(viewPrev, "-100%")}{/if}
      {@render slide(viewCur, "0")}
      {#if viewNext}{@render slide(viewNext, "100%")}{/if}
    </span>
  </button>
  <div class="acts">
    <ToggleButton name="heart" size={20} on={app.isLiked(cur)} accent="var(--accent)" notip onclick={() => app.toggleLike(cur)} />
    <button class="play" use:press onclick={() => app.togglePlay()} aria-label="Play">
      <i><PlayGlyph loading={app.loading} playing={app.playing} size={20} button={34} color="var(--text)" /></i>
    </button>
  </div>
  <span class="prog"><i style:width="{frac * 100}%"></i></span>
</div>

<style>
  .mini { position: relative; height: 64px; padding: 0 8px; display: flex; align-items: center; gap: 8px; border-radius: 12px; background: #141414; overflow: hidden; touch-action: pan-y; transition: transform .16s var(--ease); }
  .open { flex: 1; min-width: 0; height: 100%; position: relative; overflow: hidden; text-align: left; }
  .strip { position: relative; display: block; width: 100%; height: 100%; }
  .strip.settling { transition: transform .2s var(--ease); }
  .in { position: absolute; inset: 0; display: flex; align-items: center; gap: 12px; padding-right: 8px; min-width: 0; }
  .tx { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .t { font-size: var(--md); font-weight: 500; min-width: 0; }
  .a { font-size: var(--sm); color: var(--dim); }
  .acts { flex: none; display: flex; align-items: center; gap: 2px; }
  .play { flex: none; width: 40px; height: 44px; display: grid; place-items: center; }
  .play i { display: grid; place-items: center; }
  .play:global(.pressing) i { animation: press .25s var(--ease); }
  .prog { position: absolute; left: 8px; right: 8px; bottom: 0; height: 2px; border-radius: 1px; background: rgba(255, 255, 255, .12); }
  .prog i { display: block; height: 100%; border-radius: inherit; background: #fff; }
</style>
