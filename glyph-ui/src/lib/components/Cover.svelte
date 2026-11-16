<script lang="ts" module>
  // Shared observer so offscreen covers do not decode until near the viewport,
  // and scrolled-far-away thumbnails unload cleanly to bound memory.
  const callbacks = new WeakMap<Element, (visible: boolean) => void>();
  let observer: IntersectionObserver | null = null;
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
        { rootMargin: "220px 260px" },
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
  import { backend } from "../api/backend";
  import { trackKey, type Track } from "../api/types";
  import { fx } from "../ui.svelte";
  let { track, size = 40, radius = 6, round = false }: { track: Track | null | undefined; size?: number; radius?: number; round?: boolean } = $props();
  let url = $state<string | null>(null);
  let phase = $state<"loading" | "ready" | "none">("loading");
  let unveiling = $state(false);
  let loadedKey = "";
  let box: HTMLDivElement | undefined = $state();
  let near = $state(false);
  const r = $derived(round ? size / 2 : radius);
  const key = $derived(track ? trackKey(track) : "");
  const loadSig = $derived(track ? `${key}|${track.source}|${track.id}|${track.cover_url ?? ""}` : "");

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
    if (!sameTrack) {
      loadedKey = k;
      url = null;
      phase = "loading";
      timedOut = false;
      unveiling = false;
    }
    let alive = true;
    const late = setTimeout(() => { if (alive && phase === "loading") timedOut = true; }, 4000);
    backend().cover(t).then((u) => {
      if (!alive) return;
      if (!u) { clearTimeout(late); phase = "none"; return; }
      url = u;
    });
    return () => { alive = false; clearTimeout(late); clearTimeout(veilTimer); };
  });

  let veilTimer: ReturnType<typeof setTimeout> | undefined;
  function onImgLoad() {
    phase = "ready";
    timedOut = false;
    clearTimeout(veilTimer);
    if (fx.ascii && !fx.reduce) {
      unveiling = true;
      veilTimer = setTimeout(() => { unveiling = false; }, 950);
    } else {
      unveiling = false;
    }
  }
  function onImgError() {
    url = null;
    phase = "none";
    timedOut = false;
  }
</script>

<div class="cv" bind:this={box} style:width="{size}px" style:height="{size}px" style:border-radius="{r}px">
  {#if phase === "loading" && !timedOut}
    <Skeleton w={size} h={size} radius={r} id={key} />
  {:else if phase === "none" || (phase === "loading" && timedOut)}
    <div class="ph"><Icon name="music-2" size={Math.min(64, Math.max(14, size * 0.36))} color="var(--faint)" /></div>
  {/if}
  {#if url && phase !== "none"}
    <img src={url} alt="" draggable="false" decoding="async" class:hidden={phase !== "ready"} class:reveal={phase === "ready" && fx.ascii && !fx.reduce && unveiling} onload={onImgLoad} onerror={onImgError} />
  {/if}
</div>

<style>
  .cv { position: relative; flex: none; overflow: hidden; background: var(--card); }
  img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }
  img.hidden { opacity: 0; }
  img.reveal { animation: cv-in .35s ease-out both; }
  @keyframes cv-in { from { opacity: .15; } to { opacity: 1; } }
  .ph { width: 100%; height: 100%; display: grid; place-items: center; }
</style>
