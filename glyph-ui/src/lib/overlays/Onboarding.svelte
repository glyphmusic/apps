<script lang="ts">
  // ui/onboarding.rs layer(): first start, a short tour of what Glyph does,
  // one idea per page (welcome, search, library, sound, lyrics, devices,
  // theme), each with a small live illustration to play with. It follows the
  // splash on a fresh install; Settings > About opens it again
  // (`app.onboarding = true`). Arrow keys / Enter go on, Escape skips; on
  // touch screens a sideways swipe turns the page and the sheet is full
  // screen. Closing (skip, finish) fades out over 240 ms and sets
  // `app.onboarding = false`; "Bring my music" opens Settings > Transfer.
  // Card: w min(vw - 48, 640), p 30 (touch: full width, p 20), radius 18,
  // hsla(0,0,.075,.985), gap 18; stage h clamp(vh - 400, 200, 250) (touch:
  // 36 % of vh, 220..330), radius 14, white 3.5 %. Title 22 bold (24 touch),
  // body 14 / 21 white 66 % (15 / 22), min-h 63 (88). Backdrop dim 78 %.
  import { onMount } from "svelte";
  import { backend } from "../api/backend";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { decode, fx, revealK, revealRestart } from "../ui.svelte";
  import Icon from "../components/Icon.svelte";
  import Button from "../components/Button.svelte";
  import ModalLayer from "./ModalLayer.svelte";
  import Welcome from "../pages/onboarding/Welcome.svelte";
  import SearchPage from "../pages/onboarding/SearchPage.svelte";
  import LibraryPage from "../pages/onboarding/LibraryPage.svelte";
  import SoundPage from "../pages/onboarding/SoundPage.svelte";
  import LyricsPage from "../pages/onboarding/LyricsPage.svelte";
  import DevicesPage from "../pages/onboarding/DevicesPage.svelte";
  import ThemePage from "../pages/onboarding/ThemePage.svelte";
  import { easeOut, k, ob, resetTour } from "../pages/onboarding/anim.svelte";

  const PAGES = ["welcome", "search", "library", "sound", "lyrics", "devices", "theme"] as const;
  const CLOSE = 240;

  let page = $state(0), prev = $state(0), dir = $state(0);
  let now = $state(performance.now());
  let pageAt = $state(performance.now()), shownAt = $state<number | null>(null);
  let closing = $state(0), thenMigrate = $state(false);
  let vw = $state(innerWidth), vh = $state(innerHeight);
  let swipe: number | null = null;

  const still = $derived(fx.reduce);
  const touch = $derived(app.compact);
  const n = PAGES.length;
  const last = $derived(page + 1 === n);
  const titleKey = (p: number) => `onb-title-${p}`;
  const s = $derived(still ? 100 : Math.max(0, (now - pageAt) / 1000));
  const cardW = $derived(touch ? vw : Math.min(vw - 48, 640));
  const pad = $derived(touch ? 20 : 30);
  const stageW = $derived(cardW - pad * 2);
  const stageH = $derived(touch ? Math.min(330, Math.max(220, vh * 0.36)) : Math.min(250, Math.max(200, vh - 400)));
  const open = $derived(still ? 1 : shownAt == null ? 0 : easeOut((now - shownAt) / 1000 / 0.32));
  const close = $derived(closing ? (still ? 0 : 1 - easeOut((now - closing) / CLOSE)) : 1);
  const vis = $derived(Math.min(open, close));
  const switchK = $derived(still ? 1 : easeOut(s / 0.3));
  const key = $derived(PAGES[page]);
  const title = $derived(t(`onboarding.${key}.title`));

  onMount(() => {
    resetTour(performance.now());
    let raf = 0;
    const tick = () => { now = performance.now(); raf = requestAnimationFrame(tick); };
    raf = requestAnimationFrame(tick);
    const down = (e: KeyboardEvent) => {
      if (closing) return;
      const hit = () => { e.preventDefault(); e.stopImmediatePropagation(); };
      if (e.key === "ArrowRight" || e.key === "Enter") { hit(); next(); }
      else if (e.key === "ArrowLeft") { hit(); back(); }
      else if (e.key === "Escape") { hit(); escape(); }
    };
    addEventListener("keydown", down, true);
    return () => { cancelAnimationFrame(raf); removeEventListener("keydown", down, true); };
  });

  // The tour comes up once the splash is over.
  $effect(() => {
    if (!app.splash && shownAt == null) {
      const t0 = performance.now();
      shownAt = t0; pageAt = t0; ob.lyricsAt = t0;
      revealRestart(titleKey(page));
    }
  });

  // Closed: the tour is over, then what the last page asked for.
  $effect(() => {
    if (closing && now - closing >= (still ? 0 : CLOSE)) {
      const migrate = thenMigrate;
      app.onboarding = false;
      if (migrate) app.openSettings("migrate");
    }
  });

  function go(to: number) {
    if (to === page || to < 0 || to >= n || closing) return;
    dir = to > page ? 1 : -1;
    prev = page; page = to; pageAt = performance.now();
    // Coming back replays the page's intro.
    if (PAGES[to] === "search") ob.resultsAt = null;
    revealRestart(titleKey(to));
  }
  function next() { if (last) finish(false); else go(page + 1); }
  function back() { if (page > 0) go(page - 1); }
  /** Escape skips the tour; the phones' back gesture goes one page back. */
  function escape() { if (touch && page > 0) back(); else finish(false); }
  function finish(migrate: boolean) {
    if (closing) return;
    closing = performance.now(); thenMigrate = migrate;
    backend().call("onboarding_seen").catch(() => {});
  }

  // Phones: a sideways swipe turns the page (a swipe never finishes the tour).
  function pdown(e: PointerEvent) { if (touch) swipe = e.clientX; }
  function pup(e: PointerEvent) {
    if (!touch || swipe == null) return;
    const d = e.clientX - swipe;
    swipe = null;
    if (d < -56 && page + 1 < n) go(page + 1);
    else if (d > 56) back();
  }
</script>

<svelte:window bind:innerWidth={vw} bind:innerHeight={vh} />

{#if !app.splash}
  {#snippet art()}
    {#if key === "welcome"}<Welcome {s} h={stageH} />
    {:else if key === "search"}<SearchPage {s} {now} {still} h={stageH} />
    {:else if key === "library"}<LibraryPage {s} {now} w={stageW} />
    {:else if key === "sound"}<SoundPage {s} {now} {still} w={stageW} h={stageH} />
    {:else if key === "lyrics"}<LyricsPage {now} {still} w={stageW} h={stageH} />
    {:else if key === "devices"}<DevicesPage {s} {now} {still} w={stageW} h={stageH} />
    {:else}<ThemePage {s} {still} w={stageW} />{/if}
  {/snippet}

  {#snippet content()}
    <div class="top">
      <div class="step">
        <span class="no mono">{String(page + 1).padStart(2, "0")} / {String(n).padStart(2, "0")}</span>
        {#if key !== "welcome"}
          <span class="try" style:opacity={easeOut(k(s, 0.9, 0.4))}><Icon name="sparkles" size={12} color="var(--accent)" />{t("onboarding.try")}</span>
        {/if}
      </div>
      {#if last}<span class="spacer"></span>{:else}<Button kind="text" class="skip" onclick={() => finish(false)}>{t("onboarding.skip")}</Button>{/if}
    </div>

    {#key page}
      <div class="block" style:--dir={dir} class:still>
        <div class="stage" style:width="{stageW}px" style:height="{stageH}px">{@render art()}</div>
        <div class="text">
          <div class="title">{decode(title, revealK(titleKey(page), 0.5))}</div>
          <div class="body">{t(`onboarding.${key}.body`)}</div>
        </div>
      </div>
    {/key}

    <div class="foot">
      <div class="dots">
        {#each PAGES as _, i (i)}
          {@const a = i === page ? switchK : i === prev ? 1 - switchK : 0}
          <button class="dotw" aria-label={String(i + 1)} onclick={() => go(i)}>
            <span class="dot" style:width="{6 + 16 * a}px" style:background="color-mix(in srgb, var(--accent) {a * 100}%, rgba(255,255,255,.2))"></span>
          </button>
        {/each}
      </div>
      <div class="btns">
        {#if page > 0}<Button onclick={back}>{t("onboarding.back")}</Button>{/if}
        {#if last}<Button icon="arrow-right-left" onclick={() => finish(true)}>{t("onboarding.migrate")}</Button>{/if}
        <Button kind="primary" onclick={next}>{t(last ? "onboarding.start" : "onboarding.next")}<Icon name={last ? "check" : "arrow-right"} size={14} color="rgba(0,0,0,.9)" /></Button>
      </div>
    </div>
  {/snippet}

  {#if touch}
    <div class="sheet" style:background="rgba(5,5,5,{vis})" onpointerdown={pdown} onpointerup={pup} role="presentation">
      <div class="inner" style:opacity={vis} style:padding="14px {pad}px 16px">{@render content()}</div>
    </div>
  {:else}
    <ModalLayer dim={0.45} blur={16} {vis} z={85} onbackdrop={() => {}}>
      <div class="card" style:width="{cardW}px" style:padding="{pad}px" style:opacity={vis} style:transform="translateY({16 * (1 - vis)}px)" role="dialog">{@render content()}</div>
    </ModalLayer>
  {/if}
{/if}

<style>
  .card { border-radius: 18px; background: rgba(19, 19, 19, .985); display: flex; flex-direction: column; gap: 18px; font-family: var(--font); }
  .sheet { position: fixed; inset: 0; z-index: 85; overflow: hidden; touch-action: pan-y; }
  .inner { height: 100%; display: flex; flex-direction: column; gap: 18px; font-family: var(--font); }
  .top { display: flex; align-items: center; justify-content: space-between; }
  .step { display: flex; align-items: center; gap: 10px; }
  .no { font-size: 11.5px; color: rgba(255, 255, 255, .45); }
  .try { display: flex; align-items: center; gap: 5px; font-size: 12px; color: rgba(255, 255, 255, .55); }
  .spacer { height: 24px; }
  .top :global(.skip) { font-size: 12.5px; }
  .block { display: flex; flex-direction: column; gap: 22px; animation: onb-in .36s var(--ease) both; }
  .block.still { animation: none; }
  @keyframes onb-in { from { opacity: 0; transform: translateX(calc(var(--dir) * 36px)); } to { opacity: 1; transform: none; } }
  .stage { position: relative; border-radius: 14px; background: rgba(255, 255, 255, .035); overflow: hidden; max-width: 100%; }
  .text { display: flex; flex-direction: column; gap: 8px; }
  .title { font-size: 22px; font-weight: 700; color: rgba(255, 255, 255, .96); }
  .body { min-height: 63px; font-size: 14px; line-height: 21px; color: rgba(255, 255, 255, .66); }
  .foot { display: flex; align-items: center; justify-content: space-between; }
  .dots { display: flex; align-items: center; }
  .dotw { padding: 8px 3px; }
  .dot { display: block; height: 6px; border-radius: 3px; }
  .btns { display: flex; align-items: center; gap: 8px; }
  .sheet .foot { margin-top: auto; flex-direction: column; gap: 12px; }
  .sheet .title { font-size: 24px; }
  .sheet .body { min-height: 88px; font-size: 15px; line-height: 22px; }
</style>
