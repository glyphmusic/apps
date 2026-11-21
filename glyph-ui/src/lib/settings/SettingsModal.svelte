<script lang="ts">
  // settings/modal.rs modal_inner: Settings as a modal (Liner layout). A
  // black 30 % backdrop (a click closes), the panel 920 x 660 (max 94 % x
  // 90 % of the window), p 8, gap 4, radius 16, #141414 98.5 %, border white
  // 7 %, rising 14 px while it fades in (180 ms). Left the nav (232), right
  // the page in its own inset card (radius 12, #0c0c0c, border white 8 %):
  // title 17 semibold (with the sub-tab after a chevron), subtitle 13 white
  // 55 %, the body below (pt 10); px 28, py 22, 22 px fade at the top and
  // bottom edge; close button 30 at the top right. Switching page or sub-tab
  // fades the new page in. Escape is handled globally (state.svelte keys).
  import { fade } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { app } from "../state.svelte";
  import Icon from "../components/Icon.svelte";
  import OverlayScrollbar from "../shell/OverlayScrollbar.svelte";
  import { t } from "../i18n.svelte";
  import { fx, tip, fadeEdges } from "../ui.svelte";
  import Nav from "./Nav.svelte";
  import { pageComponent, pageInfo, pageShown, subtabs } from "./pages";
  import { setTab, sui } from "./settings.svelte";
  import { subHidden } from "./advanced.svelte";
  import { closeDetail, findListing, loadPlugins, pl } from "./plugins-state.svelte";
  import { mg, serviceOf, toPick } from "./migrate-state.svelte";

  // Redirect away if the active settings page is not available (e.g. Integrations while signed out on /web).
  $effect(() => {
    if (!pageShown(app.settingsPage)) app.openSettings("overview");
  });

  // Advanced turned off while one of its sub-tabs is open: back to the first tab.
  $effect(() => {
    const k = subtabs(app.settingsPage)[sui.sub];
    if (k && subHidden(k)) sui.sub = 0;
  });

  // A page opened from elsewhere (app.openSettings(page)) starts at its first tab.
  $effect(() => {
    const p = app.settingsPage;
    if (p !== sui.lastPage) { sui.lastPage = p; sui.sub = 0; }
  });
  // Plugin pages and texts are known as soon as settings open.
  $effect(() => { if (!pl.loaded) loadPlugins(); });

  const page = $derived(app.settingsPage);
  const info = $derived(pageInfo(page));
  const tab = $derived(subtabs(page)[sui.sub]);
  const detailName = $derived(
    page === "plugins" && pl.detail && sui.sub < 2
      ? (findListing(pl.detail)?.name ?? pl.installed.find((p) => p.id === pl.detail)?.name ?? pl.detail)
      : null,
  );
  type Crumb = { label: string; onclick?: () => void };
  const crumbs = $derived.by((): Crumb[] => {
    if (page === "migrate" && mg.step !== "pick") {
      const root: Crumb = mg.step === "running" ? { label: info.name } : { label: info.name, onclick: toPick };
      if (mg.step === "link") return [root, { label: t("migrate.link") }];
      if (mg.service) {
        const svcName = serviceOf(mg.service).name;
        const acc = mg.overview?.account.trim() ?? "";
        if (mg.step === "choose") {
          const backToConnect = mg.returnStep === "connect" ? () => { mg.error = null; mg.step = "connect"; } : undefined;
          if (acc && acc.toLowerCase() !== svcName.toLowerCase()) {
            return [root, { label: svcName, onclick: backToConnect }, { label: acc }];
          }
          if (backToConnect) {
            return [root, { label: svcName, onclick: backToConnect }];
          }
        }
        return [root, { label: svcName }];
      }
    }
    const out: Crumb[] = [];
    if (tab || detailName) out.push({ label: info.name, onclick: () => setTab(page, 0) });
    else out.push({ label: info.name });
    if (tab) out.push(detailName ? { label: t(tab), onclick: closeDetail } : { label: t(tab) });
    if (detailName) out.push({ label: detailName });
    return out;
  });
  const pageSub = $derived(
    page !== "migrate"
      ? info.sub
      : mg.step === "connect" && mg.service
        ? t(`migrate.${mg.service}.short`)
        : mg.step === "link"
          ? t("migrate.link.hint")
          : mg.step === "choose"
            ? t("migrate.choose.hint")
            : info.sub,
  );
  const C = $derived(pageComponent(page));
  let scroller: HTMLDivElement | undefined = $state();
  let closing = $state(false);
  $effect(() => { void page; void sui.sub; void detailName; void mg.step; if (scroller) scroller.scrollTop = 0; });
  const close = () => (app.settingsOpen = false);

  // Dim the background without backdrop-filter: App.svelte blurs .root and
  // Background directly, which stays reliable in WebView2 after animations settle.
  function backdrop(_node: HTMLElement) {
    return {
      duration: fx.reduce ? 1 : 160,
      easing: cubicOut,
      css: (t: number) => `background: rgba(0, 0, 0, ${(0.45 * t).toFixed(3)});`,
    };
  }
  function panelMotion(_node: HTMLElement) {
    return {
      duration: fx.reduce ? 1 : 160,
      easing: cubicOut,
      css: (t: number) =>
        `opacity: ${t.toFixed(3)}; transform: translateY(${((1 - t) * 10).toFixed(2)}px) scale(${(0.985 + 0.015 * t).toFixed(4)});`,
    };
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="back" class:closing transition:backdrop onintrostart={() => (closing = false)} onoutrostart={() => (closing = true)} onclick={close}>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="panel" data-part="settings" transition:panelMotion role="dialog" tabindex="-1" aria-label={info.name} onclick={(e) => e.stopPropagation()}>
    <Nav />
    <div class="content">
      <div class="scroll page" bind:this={scroller} use:fadeEdges={{ top: 22, bottom: 22 }}>
        <div class="inner">
          <div class="title">
            {#each crumbs as c, i (i)}
              {#if i > 0}<Icon name="chevron-right" size={16} color="rgba(255,255,255,.4)" />{/if}
              {#if i < crumbs.length - 1 && c.onclick}
                <button class="crumb dim" onclick={c.onclick}>{c.label}</button>
              {:else if i < crumbs.length - 1}
                <span class="crumb dim">{c.label}</span>
              {:else}
                <span class="crumb cur">{c.label}</span>
              {/if}
            {/each}
          </div>
          <div class="sub">{pageSub}</div>
        </div>
        <div class="bstack">
          {#key `${page}:${sui.sub}:${tab ?? ""}`}
            <div class="body" in:fade={{ duration: 160, delay: 30 }} out:fade={{ duration: 120 }}><C sub={sui.sub} /></div>
          {/key}
        </div>
      </div>
      {#if !app.compact}<OverlayScrollbar el={scroller} top={14} bottom={14} />{/if}
      <button class="x" onclick={close} use:tip={t("tip.x")} aria-label={t("tip.x")}><Icon name="x" size={16} color="rgba(255,255,255,.7)" /></button>
    </div>
  </div>
</div>

<style>
  .back { position: fixed; inset: 0; z-index: 30; display: flex; align-items: center; justify-content: center; background: rgba(0, 0, 0, .45); }
  .back.closing { pointer-events: none; }
  .panel { position: relative; display: flex; gap: 4px; padding: 8px; width: 920px; max-width: 94%; height: 660px; max-height: 90%; border-radius: 16px; overflow: hidden; background: #141414; }
  .content { position: relative; flex: 1; min-width: 0; height: 100%; border-radius: 12px; overflow: hidden; background: #0c0c0c; }
  .page { height: 100%; padding: 22px 28px; scrollbar-width: none; -ms-overflow-style: none; }
  .page::-webkit-scrollbar { display: none; width: 0; height: 0; }
  .inner { display: flex; flex-direction: column; gap: 4px; padding-right: 36px; }
  .title { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; font-size: 17px; font-weight: 600; color: #fff; }
  .crumb { font: inherit; color: #fff; border-radius: 5px; transition: color .12s; }
  .crumb.dim { color: rgba(255, 255, 255, .5); }
  button.crumb.dim { cursor: pointer; }
  button.crumb.dim:hover { color: #fff; }
  .crumb.cur { animation: fade-in .18s ease-out; }
  .sub { font-size: 13px; color: rgba(255, 255, 255, .55); }
  .body { grid-area: 1 / 1; min-width: 0; padding-top: 10px; }
  .bstack { display: grid; }
  .x { position: absolute; top: 14px; right: 14px; width: 30px; height: 30px; display: grid; place-items: center; border-radius: 8px; transition: background-color .12s; }
  .x:hover { background: rgba(255, 255, 255, .08); }
  :global(html.compact) .panel { width: 100%; max-width: 100%; height: 100%; max-height: 100%; border-radius: 0; }
</style>
