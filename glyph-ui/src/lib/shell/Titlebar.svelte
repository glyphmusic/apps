<script lang="ts">
  // chrome.rs titlebar: 44 px, "glyph" in accent mono 15 bold, the search
  // field centred on the window, backend status / update notice, and the
  // window pill (min / max / close) on Windows and Linux.
  import { onMount } from "svelte";
  import SearchField from "./SearchField.svelte";
  import Icon from "../components/Icon.svelte";
  import Avatar from "../pages/profile/Avatar.svelte";
  import { app } from "../state.svelte";
  import { tip } from "../ui.svelte";
  import { GET_APP_URL, onWeb } from "../web";
  import { isTauri } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import { close, isMac, isMaximized, minimize, titlebarDown, toggleMaximize, watchMaximized } from "../window";
  let { search = $bindable() }: { search?: SearchField } = $props();
  const mac = isMac();
  let maximized = $state(false);
  const desktop = $derived(
    isTauri() && app.platform !== "android" && app.platform !== "ios" && app.platform !== "web" && !onWeb(),
  );
  const windowControls = $derived(desktop && !mac);
  const webInstall = $derived(!isTauri());
  const canBack = $derived(app.history.length > 0);
  const canForward = $derived(app.forwardHistory.length > 0);
  onMount(() => {
    if (mac || !isTauri() || onWeb()) return;
    return watchMaximized((v) => (maximized = v));
  });
  async function onMaximize() {
    await toggleMaximize();
    maximized = await isMaximized();
  }
  const status = $derived.by(() => {
    const b = app.backendState;
    if (b.kind === "connecting") return { text: t("status.connecting"), warn: false };
    if (b.kind === "offline") return { text: t("status.backend_offline"), warn: true };
    if (b.kind === "incompatible") return { text: tf("status.incompatible", { v: b.version }), warn: true };
    return null;
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<header class="tb" class:mac onmousedown={titlebarDown}>
  <div class="left">
    <span class="logo mono">glyph</span>
    {#if desktop}
      <div class="nav" data-no-drag>
        <button
          class="nb"
          type="button"
          disabled={!canBack}
          aria-label={t("nav.back")}
          use:tip={canBack ? t("nav.back") : undefined}
          onclick={() => app.back()}
        >
          <Icon name="chevron-left" size={18} />
        </button>
        <button
          class="nb"
          type="button"
          disabled={!canForward}
          aria-label={t("nav.forward")}
          use:tip={canForward ? t("nav.forward") : undefined}
          onclick={() => app.forward()}
        >
          <Icon name="chevron-right" size={18} />
        </button>
      </div>
    {/if}
  </div>
  <div class="center"><SearchField bind:this={search} /></div>
  <div class="right">
    {#if status}
      <button class="st mono" class:warn={status.warn} onclick={() => !onWeb() && app.openSettings("server")}>{status.text}</button>
    {/if}
    {#if app.update}
      <button class="up mono" onclick={() => app.openSettings("about")}>{tf("update.badge", { v: app.update.version.replace(/^v/, "") })}</button>
    {/if}
    {#if webInstall}
      <a class="install" href={GET_APP_URL} target="_blank" rel="noopener noreferrer" data-no-drag>
        <Icon name="download" size={14} />
        <span>{t("web.install_app")}</span>
      </a>
    {/if}
    <button
      class="pfp"
      class:open={app.controlMenu?.menu === "account"}
      data-no-drag
      aria-label={app.account ? `@${app.account.handle}` : t("menu.signin")}
      use:tip={app.account ? `@${app.account.handle}` : t("menu.signin")}
      onmousedown={(e) => e.stopPropagation()}
      onclick={(e) => {
        const r = e.currentTarget.getBoundingClientRect();
        app.ctx = null;
        app.controlMenu = app.controlMenu?.menu === "account" ? null : { menu: "account", x: r.right, y: r.bottom + 6 };
      }}
    >
      <Avatar src={app.ownAvatarUrl} size={28} flat />
    </button>
    {#if windowControls}
      <div class="pill" data-no-drag>
        <button class="wb" aria-label="Minimize" onclick={minimize}><Icon name="minus" size={13} /></button>
        <button class="wb" aria-label={maximized ? "Restore" : "Maximize"} onclick={onMaximize}>
          <span class="ico" class:restore={maximized}><Icon name={maximized ? "copy" : "square"} size={maximized ? 12 : 11} /></span>
        </button>
        <button class="wb close" aria-label="Close" onclick={close}><Icon name="x" size={13} /></button>
      </div>
    {/if}
  </div>
</header>

<style>
  .tb { position: relative; flex: none; height: var(--titlebar-h); width: 100%; display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 420px) minmax(0, 1fr); align-items: center; gap: 12px; padding: 0 8px 0 18px; z-index: 20; }
  .tb.mac { padding-left: 84px; }
  .left { justify-self: start; display: flex; align-items: center; gap: 14px; min-width: 0; }
  .logo { font-size: 15px; font-weight: 700; color: var(--accent); }
  .nav { display: flex; align-items: center; gap: 2px; }
  .nb { width: 28px; height: 28px; display: grid; place-items: center; border-radius: 7px; color: var(--text); opacity: .82; transition: background-color .12s ease-out, opacity .12s ease-out, transform .12s ease-out; }
  .nb:hover:not(:disabled) { background: var(--hover); opacity: 1; }
  .nb:active:not(:disabled) { background: var(--active); transform: scale(.94); }
  .nb:disabled { opacity: .28; pointer-events: none; }
  .center { justify-self: center; width: 100%; max-width: 420px; min-width: 0; display: flex; align-items: center; justify-content: center; }
  .right { justify-self: end; display: flex; align-items: center; gap: 12px; min-width: 0; }
  .st, .up { flex: none; white-space: nowrap; font-size: var(--xs); color: var(--faint); position: relative; }
  .st.warn { color: var(--warn); }
  .up { color: var(--ok); }
  .up:hover { color: var(--text); }
  .install { flex: none; height: 32px; padding: 0 11px; display: inline-flex; align-items: center; gap: 6px; border-radius: 8px; background: #141414; font-size: var(--sm); font-weight: 500; color: var(--dim); white-space: nowrap; transition: background-color .12s ease-out, color .12s ease-out; }
  .install:hover { background: #222224; color: var(--text); }
  .install:active { background: rgba(255, 255, 255, .1); }
  .pfp { position: relative; flex: none; width: 32px; height: 32px; display: grid; place-items: center; border-radius: 50%; background: #141414; transition: background-color .12s, transform .12s; }
  .pfp :global(.av) { background: #1c1c1e; }
  .pfp:hover, .pfp.open { background: #222224; transform: scale(1.04); }
  /* Window pill: 3 x 26 px buttons, 2 px gaps, 3 px padding, solid dark surface */
  .pill { position: relative; flex: none; display: flex; align-items: center; height: 32px; padding: 0 3px; gap: 2px; border-radius: 8px; background: #141414; }
  .wb { width: 26px; height: 26px; display: grid; place-items: center; border-radius: 6px; color: rgba(255, 255, 255, .7); }
  .wb:hover { background: rgba(255, 255, 255, .1); color: #fff; }
  .wb.close:hover { background: #e81123; }
  .ico { display: inline-flex; }
  .ico.restore { transform: scaleX(-1); }
</style>
