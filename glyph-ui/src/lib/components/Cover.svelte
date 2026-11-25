<script lang="ts" module>
  import { backend } from "../api/backend";
  import { trackKey, type Track } from "../api/types";
  // Shared observer so offscreen covers do not decode until near the viewport,
  // and scrolled-far-away thumbnails unload cleanly to bound memory.
  const callbacks = new WeakMap<Element, (visible: boolean) => void>();
  let observer: IntersectionObserver | null = null;
  /**
   * Covers already shown once (song -> picture address): shown again at
   * once, without the shimmer and the fade. Rows that come back (a sheet
   * opens, a list scrolls back) then draw nothing new while they move.
   */
  const known = new Map<string, string>();
  function remember(sig: string, url: string) {
    known.delete(sig);
    known.set(sig, url);
    if (known.size > 1500) known.delete(known.keys().next().value!);
  }
  const sigOf = (t: Track) => `${trackKey(t)}|${t.source}|${t.id}|${t.cover_url ?? ""}`;
  const warming = new Set<string>();
  /**
   * Fetches and decodes these songs' covers ahead of time (e.g. the next
   * songs of the queue), so a sheet that shows them later has them at once.
   */
  export function warmCovers(tracks: Track[]) {
    for (const t of tracks) {
      const sig = sigOf(t);
      if (known.has(sig) || warming.has(sig)) continue;
      warming.add(sig);
      void backend().cover(t).then((u) => {
        if (!u) return;
        const im = new Image();
        im.decoding = "async";
        im.src = u;
        return im.decode().then(() => remember(sig, u), () => {});
      }).catch(() => {}).finally(() => warming.delete(sig));
    }
  }
  function observeVisibility(el: Element, cb: (visible: boolean) => void): () => void {
    if (typeof IntersectionObserver === "undefined") {
      cb(true);
      return () => {};
    }
    if (!observer) {
      observer = new IntersectionObserver(
        (entries) => {
          for (const e of entries) {
            callbacks.get(e.target)?.(e.isIntersecting);
          }
        },
        { rootMargin: "120px 160px" },
      );
    }
    callbacks.set(el, cb);
    observer.observe(el);
    return () => {
      callbacks.delete(el);
      observer?.unobserve(el);
    };
  }
</script>

<script lang="ts">
  // widgets/cover.rs: the song's cover; shimmer while it loads, then fade in.
  // No cover after 4 s, or none at all: a flat card with a music note.
  import { untrack } from "svelte";
  import Icon from "./Icon.svelte";
  import Skeleton from "./Skeleton.svelte";
  import { fx } from "../ui.svelte";
  import { cornerPx } from "../corners";
  let { track, size = 40, radius = 6, round = false }: { track: Track | null | undefined; size?: number; radius?: number; round?: boolean } = $props();
  const r = $derived(round ? size / 2 : radius);
  const key = $derived(track ? trackKey(track) : "");
  const loadSig = $derived(track ? sigOf(track) : "");
  // Known cover: ready from the first picture on.
  const hit = untrack(() => (loadSig ? known.get(loadSig) : undefined));
  let url = $state<string | null>(hit ?? null);
  let phase = $state<"loading" | "ready" | "none">(hit ? "ready" : "loading");
  let unveiling = $state(false);
  let loadedKey = hit ? untrack(() => key) : "";
  let box: HTMLDivElement | undefined = $state();
  let near = $state(!!hit);

  let timedOut = $state(false);
  $effect(() => {
    if (!track) { near = false; return; }
    if (size >= 180) { near = true; return; }
    if (!box) return;
    return observeVisibility(box, (visible) => {
      near = visible;
      if (!visible) {
        loadedKey = "";
        url = null;
        phase = "loading";
        timedOut = false;
        unveiling = false;
      }
    });
  });

  $effect(() => {
    void loadSig;
    const t = untrack(() => track);
    const k = key;
    if (!t) {
      loadedKey = "";
      url = null;
      phase = "none";
      timedOut = false;
      unveiling = false;
      return;
    }
    if (!near) return;
    const sameTrack = k === loadedKey;
    if (sameTrack && untrack(() => phase === "ready" && url !== null)) return;
    const sig = untrack(() => loadSig);
    const seen = known.get(sig);
    if (seen) {
      loadedKey = k;
      url = seen;
      phase = "ready";
      timedOut = false;
      unveiling = false;
      return;
    }
    if (!sameTrack) {
      loadedKey = k;
      url = null;
      phase = "loading";
      timedOut = false;
      unveiling = false;
    }
    let alive = true;
    const ac = new AbortController();
    const late = setTimeout(() => { if (alive && phase === "loading") timedOut = true; }, 7000);
    let again: ReturnType<typeof setTimeout> | undefined;
    const ask = (retry: boolean) => backend().cover(t, ac.signal).then((u) => {
      if (!alive) return;
      if (!u) {
        clearTimeout(late);
        phase = "none";
        // One more try a bit later (after the "no cover" answer expires):
        // a slow network or a busy server should not leave a note for good.
        if (!retry) again = setTimeout(() => { if (alive) ask(true); }, 16_000);
        return;
      }
      // Found on the second try: keep the note until the picture is decoded.
      if (phase === "none") { phase = "loading"; timedOut = true; }
      url = u;
      remember(sig, u);
    });
    ask(false);
    return () => { alive = false; ac.abort(); clearTimeout(late); clearTimeout(again); clearTimeout(veilTimer); };
  });

  let veilTimer: ReturnType<typeof setTimeout> | undefined;
  function onImgLoad() {
    // A known cover was ready from the start: no shimmer, no fade.
    if (phase === "ready") return;
    phase = "ready";
    timedOut = false;
    clearTimeout(veilTimer);
    if (!fx.still && !fx.reduce) {
      unveiling = true;
      veilTimer = setTimeout(() => { unveiling = false; }, 300);
    } else {
      unveiling = false;
    }
  }
  function onImgError() {
    known.delete(untrack(() => loadSig));
    url = null;
    phase = "none";
    timedOut = false;
    unveiling = false;
  }
</script>

<div class="cv" bind:this={box} style:width="{size}px" style:height="{size}px" style:border-radius={round ? `${r}px` : cornerPx(r)}>
  {#if (phase === "loading" && !timedOut) || unveiling}
    <Skeleton w={size} h={size} radius={r} id={key} />
  {:else if phase === "none" || (phase === "loading" && timedOut)}
    <div class="ph"><Icon name="music-2" size={Math.min(64, Math.max(14, size * 0.36))} color="var(--faint)" /></div>
  {/if}
  {#if url && phase !== "none"}
    <img src={url} alt="" draggable="false" decoding="async" class:hidden={phase !== "ready"} class:reveal={phase === "ready" && !fx.still && !fx.reduce && unveiling} onload={onImgLoad} onerror={onImgError} />
  {/if}
</div>

<style>
  .cv { position: relative; flex: none; overflow: hidden; background: var(--card); }
  img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }
  img.hidden { opacity: 0; }
  img.reveal { animation: cv-in .26s cubic-bezier(.22, 1, .36, 1) both; }
  @keyframes cv-in { from { opacity: 0; } to { opacity: 1; } }
  .ph { width: 100%; height: 100%; display: grid; place-items: center; }
</style>
