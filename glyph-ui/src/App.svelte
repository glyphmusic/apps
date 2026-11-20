<script lang="ts">
  // ui/mod.rs: the window root. Glow behind everything; titlebar; sidebar,
  // page and the right panel (queue / Connect / add songs) in a row with
  // 10 px gaps; the player bar slides in at the bottom. Settings is a modal
  // over the page (the UI behind fades out). Menus, dialogs, toasts and
  // tooltips float on top.
  import { onMount, tick } from "svelte";
  import { fly, slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { app } from "./lib/state.svelte";
  import { applyTheme } from "./lib/theme";
  import { activeLooks, applyLooks } from "./lib/skin";
  import { fx, setUiSuspended, startRevealClock, suspendBackground } from "./lib/ui.svelte";
  import { installKeys } from "./lib/keys";
  import Background from "./lib/shell/Background.svelte";
  import Titlebar from "./lib/shell/Titlebar.svelte";
  import Sidebar from "./lib/shell/Sidebar.svelte";
  import PlayerBar from "./lib/shell/PlayerBar.svelte";
  import Toasts from "./lib/shell/Toasts.svelte";
  import TooltipLayer from "./lib/shell/TooltipLayer.svelte";
  import RightPanel from "./lib/shell/RightPanel.svelte";
  import Page from "./lib/shell/Page.svelte";
  import Overlays from "./lib/shell/Overlays.svelte";
  import SettingsModal from "./lib/settings/SettingsModal.svelte";
  import Compact from "./lib/compact/Compact.svelte";
  import { loadPlugins, pl } from "./lib/settings/plugins-state.svelte";
  import { installSettingsHash } from "./lib/settings/hash.svelte";
  import { installRoutes } from "./lib/routes.svelte";
  import { installAccountEvents } from "./lib/account-events.svelte";
  import { backend } from "./lib/api/backend";
  import ErrorCard from "./lib/components/ErrorCard.svelte";
  import Icon from "./lib/components/Icon.svelte";
  import { t } from "./lib/i18n.svelte";
  import { isMac, minimize, close as closeWindow, titlebarDown, inMiniWindow, inShareSheet } from "./lib/window";
  import AddShared from "./lib/overlays/AddShared.svelte";
  import NameDialog from "./lib/overlays/NameDialog.svelte";
  import { closeShareSheet, startShareSheet } from "./lib/share-sheet.svelte";
  import { isTauri, isWeb } from "./lib/api/backend";
  import { mcpq, type McpApproval } from "./lib/settings/plugin-events.svelte";
  import {
    setupWebMiniMain,
    setupWebMiniPopup,
    syncWebMiniShown,
    notifyWebMiniClosed,
    requestWebMiniSync,
  } from "./lib/web-mini.svelte";
  import MiniPlayer from "./lib/overlays/MiniPlayer.svelte";
  import type SearchField from "./lib/shell/SearchField.svelte";

  let search: SearchField | undefined = $state();
  let error = $state<string | null>(null);
  let narrow = $state(false);
  let winW = $state(typeof window !== "undefined" ? window.innerWidth : 1280);
  let resizing = $state<"left" | "right" | null>(null);
  let pendingMcpConnectId = $state<string | null>(null);

  function checkMcpConnectHash() {
    if (typeof window === "undefined" || !isWeb()) return;
    const hash = window.location.hash || "";
    const m = hash.match(/^#mcp-connect=([^&]+)/);
    if (!m) return;
    const id = decodeURIComponent(m[1]);
    const rest = hash.slice(m[0].length);
    const newHash = rest ? (rest.startsWith("&") ? "#" + rest.slice(1) : rest) : "";
    const newUrl = window.location.pathname + window.location.search + newHash;
    window.history.replaceState(null, "", newUrl);
    pendingMcpConnectId = id;
    if (!app.account) {
      app.toast(t("sp.mcp.web_connect.signin"), "info");
      app.openSettings("account");
    }
  }

  $effect(() => {
    if (isWeb() && app.account && pendingMcpConnectId) {
      const id = pendingMcpConnectId;
      pendingMcpConnectId = null;
      backend()
        .call<McpApproval>("mcp_claim_oauth", { id })
        .then((approval) => {
          if (approval && !mcpq.approvals.some((a) => a.id === approval.id)) {
            mcpq.approvals = [approval, ...mcpq.approvals];
          }
        })
        .catch(() => {
          app.toast(t("sp.mcp.web_connect.failed"), "info");
        });
    }
  });

  // Tauri opens the mini player as a window of its own (`#mini`).
  const mini = inMiniWindow();
  // Android: the share sheet over another app (`#share`) shows only the
  // "Add to Glyph" sheet on a see-through page (share-sheet.svelte.ts).
  const share = inShareSheet();
  let shareStarted = $state(false);
  if (share) document.documentElement.classList.add("share-sheet");
  // Nothing left on the sheet page (closed, saved, Back): the window goes.
  $effect(() => {
    if (share && shareStarted && !app.pickSongs && !app.nameDialog) closeShareSheet();
  });
  if (mini) {
    document.documentElement.classList.add("mini-window");
    if (isWeb()) {
      document.documentElement.style.background = "#0c0c0c";
      document.body.style.background = "#0c0c0c";
      document.documentElement.style.colorScheme = "dark";
    }
  }

  // Desktop windows never switch to the mobile layout (`compact`), even when
  // narrow or on coarse-pointer touch laptops; web and phones still do.
  $effect(() => {
    const desktop = isTauri() && app.platform !== "android" && app.platform !== "ios";
    app.compact = desktop ? false : narrow;
    document.documentElement.classList.toggle("compact", app.compact);
  });

  onMount(() => {
    if (share) {
      // Only phones have it: always the phone layout.
      narrow = true;
      fx.reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
      app.init().then(() => {
        startShareSheet();
        shareStarted = true;
        void backend().call("ui_ready");
      }).catch((e) => closeShareSheet(String(e)));
      return;
    }
    if (mini) {
      let offWebMini: (() => void) | undefined;
      if (isWeb()) offWebMini = setupWebMiniPopup(app);
      app.init().then(() => {
        if (isWeb()) requestWebMiniSync();
      }).catch((e) => (error = String(e)));
      startRevealClock();
      return () => {
        offWebMini?.();
        if (isWeb()) notifyWebMiniClosed();
      };
    }
    const fontReady = document.fonts ? document.fonts.load('700 10px "JetBrains Mono"').catch(() => {}) : Promise.resolve();
    const reveal = async () => {
      await Promise.race([fontReady, new Promise((r) => setTimeout(r, 150))]);
      await tick();
      if (isTauri()) void backend().call("ui_ready");
    };
    let dead = false;
    let offRoutes: (() => void) | undefined;
    let offSettingsHash: (() => void) | undefined;
    let cleanupWebMini: (() => void) | undefined;
    const clearStartupListeners = () => {
      window.removeEventListener("error", onStartupError);
      window.removeEventListener("unhandledrejection", onStartupRejection);
    };
    const onStartupError = (ev: ErrorEvent) => {
      if (dead || app.ready || error) return;
      error = ev.error ? String(ev.error) : (ev.message || "Unexpected startup error");
      clearStartupListeners();
      void reveal();
    };
    const onStartupRejection = (ev: PromiseRejectionEvent) => {
      if (dead || app.ready || error) return;
      error = String(ev.reason ?? "Unhandled startup rejection");
      clearStartupListeners();
      void reveal();
    };
    window.addEventListener("error", onStartupError);
    window.addEventListener("unhandledrejection", onStartupRejection);
    if (isWeb()) cleanupWebMini = setupWebMiniMain(app);
    // Page addresses (/search?q=, /artist/..., ...) only where there is an address bar;
    // the settings URL fragment (`#settings/...`) stays synced everywhere so reloads keep it open.
    app.init().then(async () => {
      if (dead) return;
      clearStartupListeners();
      if (backend().kind !== "tauri") offRoutes = installRoutes();
      offSettingsHash = installSettingsHash();
      if (backend().kind !== "web") loadPlugins();
      if (backend().kind === "tauri" || backend().kind === "web") installAccountEvents();
      await reveal();
    }).catch(async (e) => {
      if (dead) return;
      clearStartupListeners();
      error = String(e);
      await reveal();
    });
    startRevealClock();
    const mq = matchMedia("(max-width: 760px), (pointer: coarse) and (max-width: 1024px)");
    const sync = () => (narrow = mq.matches);
    sync();
    mq.addEventListener("change", sync);
    const rm = matchMedia("(prefers-reduced-motion: reduce)");
    fx.reduce = rm.matches;
    const offKeys = installKeys(() => search?.focus());
    checkMcpConnectHash();
    const onHashChange = () => checkMcpConnectHash();
    window.addEventListener("hashchange", onHashChange);
    return () => {
      dead = true;
      clearStartupListeners();
      offRoutes?.();
      offSettingsHash?.();
      cleanupWebMini?.();
      mq.removeEventListener("change", sync);
      offKeys();
      window.removeEventListener("hashchange", onHashChange);
    };
  });

  // The mini player button opens / closes that window (and the window's own
  // close button tells the main window).
  $effect(() => {
    const shown = app.miniShown;
    if (isTauri() && app.ready) {
      void backend().call("mini_show", { shown });
    } else if (isWeb() && app.ready) {
      if (mini) {
        if (!shown) {
          notifyWebMiniClosed();
          window.close();
        }
      } else {
        syncWebMiniShown(app, shown);
      }
    }
  });

  // Plugins' looks: font, corners, icons, style sheet (skin.ts). Paused on
  // Settings > Plugins and the install warning: there a look can always be
  // turned off, and the plugin warnings always look like Glyph.
  $effect(() => {
    const paused = (app.settingsOpen && app.settingsPage === "plugins") || !!pl.confirm;
    applyLooks(app.config && !paused ? activeLooks(pl.ui.looks ?? [], app.config.visuals.theme) : []);
  });

  $effect(() => {
    if (!app.config) return;
    applyTheme(app.config.visuals.theme, app.config.ui.text_scale);
    // Performance mode: no decode effects, placeholder characters stand
    // still, no blur behind panels.
    const perf = !!app.config.visuals.performance_mode;
    fx.ascii = app.config.visuals.ascii_effects;
    fx.still = perf;
    document.documentElement.classList.toggle("perf", perf);
  });

  // Song + lyrics in Now Playing carries its own transport: no bar there.
  const fullNp = $derived(app.view === "now_playing" && !!app.player.current && !(app.showLyrics && app.lyricsOnly) && !app.zen);
  const showBar = $derived(!fullNp && !!app.player.current);
  const rightOpen = $derived(!app.zen && (app.showQueue || app.showDevices || app.addSongsOpen));
  const zen = $derived(app.zen && app.view === "now_playing" && !!app.player.current && !app.settingsOpen);
  const leftW = $derived(Math.min(app.leftSidebarW, Math.max(64, winW - (rightOpen ? 260 : 0) - 340)));
  const rightW = $derived(Math.min(app.rightSidebarW, Math.max(260, winW - leftW - 340)));

  $effect(() => {
    setUiSuspended("sash", resizing !== null);
    return () => setUiSuspended("sash", false);
  });

  let layoutInit = false;
  $effect(() => {
    void app.view;
    void rightOpen;
    void showBar;
    void app.compact;
    if (!layoutInit) { layoutInit = true; return; }
    suspendBackground(260);
  });

  function startResize(e: PointerEvent, side: "left" | "right") {
    if (e.button !== 0) return;
    e.preventDefault();
    const startX = e.clientX;
    const startW = side === "left" ? leftW : rightW;
    let latestX = startX;
    let rafId = 0;
    resizing = side;
    document.documentElement.style.cursor = "col-resize";
    const applyWidth = () => {
      rafId = 0;
      if (side === "left") {
        const max = Math.max(64, Math.min(280, winW - (rightOpen ? rightW : 0) - 340));
        app.setLeftSidebarW(Math.min(max, startW + (latestX - startX)));
      } else {
        const max = Math.max(260, Math.min(580, winW - leftW - 340));
        app.setRightSidebarW(Math.min(max, startW + (startX - latestX)));
      }
    };
    const move = (ev: PointerEvent) => {
      latestX = ev.clientX;
      if (!rafId) rafId = requestAnimationFrame(applyWidth);
    };
    const end = () => {
      if (rafId) {
        cancelAnimationFrame(rafId);
        applyWidth();
      }
      removeEventListener("pointermove", move);
      removeEventListener("pointerup", end);
      removeEventListener("pointercancel", end);
      document.documentElement.style.cursor = "";
      resizing = null;
      if (side === "left") app.setLeftSidebarW(app.leftSidebarW, true);
      else app.setRightSidebarW(app.rightSidebarW, true);
    };
    addEventListener("pointermove", move);
    addEventListener("pointerup", end);
    addEventListener("pointercancel", end);
  }
</script>

<svelte:window bind:innerWidth={winW} />

{#if !mini && !share && (app.ready || error)}<Background blurred={app.settingsOpen || app.onboarding} />{/if}

{#if share}
  {#if app.pickSongs}<AddShared />{/if}
  {#if app.nameDialog}<NameDialog />{/if}
{:else if mini}
  {#if app.ready}<MiniPlayer />{:else if error}<p class="mini-err">{error}</p>{/if}
{:else if error}
  <div class="fatal">
    <div class="ftb" class:mac={isMac()} onmousedown={titlebarDown} role="presentation" data-tauri-drag-region>
      <span class="logo mono">glyph</span>
      <span class="sp"></span>
      {#if isTauri() && !isMac() && app.platform !== "android" && app.platform !== "ios"}
        <button class="wb" aria-label="Minimize" onclick={minimize}><Icon name="minus" size={13} /></button>
        <button class="wb close" aria-label="Close" onclick={closeWindow}><Icon name="x" size={13} /></button>
      {/if}
    </div>
    <div class="fbody">
      <ErrorCard title={t("error.start.title")} body={t("error.start.body")} detail={error} onretry={() => location.reload()} />
    </div>
  </div>
{:else if app.ready}
  {#if app.compact}
    <Compact />
  {:else}
    <div class="root" class:dim={app.settingsOpen || app.onboarding} class:resizing={resizing !== null} data-skin>
      {#if zen}<div class="tb-space"></div>{:else}<Titlebar bind:search />{/if}
      <div class="body" class:with-bar={showBar}>
        {#if !zen}
          <Sidebar width={leftW} />
          <div class="sash" class:on={resizing === "left"} role="separator" aria-orientation="vertical" aria-label="Resize left sidebar"
            onpointerdown={(e) => startResize(e, "left")} ondblclick={() => app.setLeftSidebarW(app.leftSidebarW > 64 ? 64 : 210, true)}></div>
        {/if}
        <main class="main" class:narrow={rightOpen} data-part="page">
          <Page />
        </main>
        {#if rightOpen}
          <div class="sash" class:on={resizing === "right"} role="separator" aria-orientation="vertical" aria-label="Resize right sidebar"
            onpointerdown={(e) => startResize(e, "right")} ondblclick={() => app.setRightSidebarW(320, true)}></div>
          <aside class="right" style:width="{rightW}px" data-part="side-panel" in:fly={{ x: 24, duration: 160, opacity: 0, easing: cubicOut }}><RightPanel /></aside>
        {/if}
      </div>
      {#if showBar}
        <div class="barslot" transition:slide={{ duration: 250 }}><PlayerBar /></div>
      {/if}
    </div>
    {#if app.settingsOpen}<SettingsModal />{/if}
  {/if}
  <Overlays />
  <Toasts />
  <TooltipLayer />
{/if}

<style>
  .root { position: relative; z-index: 1; height: 100%; display: flex; flex-direction: column; transition: opacity .2s ease-out, filter .18s ease-out; }
  .root.resizing .main, .root.resizing .right, .root.resizing :global(nav.sb) { pointer-events: none; }
  /* Blur the background UI directly so WebView2 never drops backdrop-filter after transitions settle. */
  .root.dim { pointer-events: none; filter: blur(14px) saturate(1.1); }
  .mini-err { margin: 0; padding: 10px 12px; border-radius: 12px; background: #141414; color: var(--text); font-size: 11px; }
  .tb-space { height: var(--titlebar-h); flex: none; }
  .body { flex: 1; min-height: 0; display: flex; gap: var(--gap); padding: 0 var(--gap); }
  .body.with-bar { margin-bottom: calc(-1 * var(--gap)); }
  .body > :global(nav) { padding-bottom: calc(12px + var(--gap)); }
  /* The page reaches into the window margin so its scrollbar sits there. */
  .main { position: relative; flex: 1; min-width: 0; height: 100%; margin-right: calc(-1 * var(--gap)); padding-right: var(--gap); }
  .main.narrow { padding-right: 0; margin-right: 0; }
  .right { flex: none; width: 320px; height: 100%; padding: var(--gap) 0 var(--gap); overflow: hidden; }
  .sash { position: relative; flex: none; width: 10px; margin: 12px -10px; z-index: 20; cursor: col-resize; touch-action: none; }
  .sash::after { content: ""; position: absolute; top: 50%; left: 50%; width: 2px; height: 36px; border-radius: 2px; background: rgba(255, 255, 255, .24); opacity: 0; transform: translate(-50%, -50%) scaleY(.75); transition: opacity .15s ease-out, transform .15s ease-out, background-color .15s ease-out, height .15s ease-out; }
  .sash:hover::after, .sash.on::after { opacity: 1; transform: translate(-50%, -50%) scaleY(1); }
  .sash.on::after { height: 56px; background: var(--accent); }
  .barslot { flex: none; padding: var(--gap); }
  .fatal { position: relative; z-index: 1; height: 100%; display: flex; flex-direction: column; }
  .ftb { flex: none; height: var(--titlebar-h); padding: 0 8px 0 18px; display: flex; align-items: center; gap: 4px; }
  .ftb.mac { padding-left: 84px; }
  .ftb .logo { font-size: 15px; font-weight: 700; color: var(--accent); }
  .ftb .sp { flex: 1; }
  .ftb .wb { width: 34px; height: 28px; border-radius: 6px; display: grid; place-items: center; color: var(--dim); transition: background-color .12s, color .12s; }
  .ftb .wb:hover { background: rgba(255, 255, 255, .08); color: var(--text); }
  .ftb .wb.close:hover { background: #c42b1c; color: #fff; }
  .fbody { flex: 1; min-height: 0; display: grid; place-items: center; padding: 16px; }
</style>
