<script lang="ts">
  // Flat install sheet (bottom sheet on compact/mobile, modal on desktop) for
  // adding Glyph Web as a standalone PWA / iOS Home Screen pseudo-app.
  import { fade, scale } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Icon from "../components/Icon.svelte";
  import Button from "../components/Button.svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { GET_APP_URL } from "../web";
  import { closeInstallSheet, pwa, triggerNativeInstall } from "../pwa.svelte";
  import { drag, sheetIn, sheetOut, shouldClose } from "../compact/cstate.svelte";

  let pull = $state(0);
  let installing = $state(false);
  let copiedUrl = $state(false);

  const iconUrl = `${import.meta.env.BASE_URL}apple-touch-icon.png`;
  const close = () => closeInstallSheet();

  async function installNow() {
    installing = true;
    const outcome = await triggerNativeInstall();
    installing = false;
    if (outcome === "accepted") {
      app.toast(t("web.pwa.installed_toast"));
    }
  }

  async function copyPageUrl() {
    try {
      await navigator.clipboard.writeText(`${location.origin}${import.meta.env.BASE_URL}`);
      copiedUrl = true;
      setTimeout(() => (copiedUrl = false), 2000);
    } catch { /* clipboard blocked */ }
  }
</script>

{#snippet body()}
  <div class="hero">
    <img class="appicon" src={iconUrl} alt="Glyph" width="56" height="56" />
    <div class="ht">
      <h2>{t(pwa.ios ? "web.pwa.title_ios" : "web.pwa.title")}</h2>
      <p>{t(pwa.ios ? "web.pwa.sub_ios" : "web.pwa.sub")}</p>
    </div>
    {#if !app.compact}
      <button class="xbtn" onclick={close} aria-label={t("tip.x")}><Icon name="x" size={16} /></button>
    {/if}
  </div>

  <div class="perks">
    <span class="perk"><Icon name="smartphone" size={13} color="var(--accent)" />{t("web.pwa.perk.fullscreen")}</span>
    <span class="perk"><Icon name="sparkles" size={13} color="var(--accent)" />{t("web.pwa.perk.fast")}</span>
    <span class="perk"><Icon name="refresh-cw" size={13} color="var(--accent)" />{t("web.pwa.perk.sync")}</span>
  </div>

  {#if pwa.canPrompt}
    <div class="cta">
      <Button kind="primary" icon="download" class="full" disabled={installing} onclick={installNow}>
        {installing ? t("web.pwa.installing") : t("web.pwa.install_now")}
      </Button>
      <p class="hint">{t("web.pwa.prompt_hint")}</p>
    </div>
  {:else if pwa.ios}
    {#if !pwa.iosSafari}
      <div class="safari-note">
        <span class="sn-tx">{t("web.pwa.ios_use_safari")}</span>
        <button class="sn-copy" onclick={copyPageUrl}>
          <Icon name={copiedUrl ? "check" : "copy"} size={13} />
          <span>{t(copiedUrl ? "share.copied" : "ctx.copy_link")}</span>
        </button>
      </div>
    {/if}
    <ol class="steps">
      <li>
        <span class="step-ic" aria-hidden="true">
          <!-- Apple Share icon (square with upward arrow) -->
          <svg width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 3v12" />
            <path d="M8 7l4-4 4 4" />
            <path d="M5 12v7a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-7" />
          </svg>
        </span>
        <span class="step-tx">
          <b>{t("web.pwa.ios.step1.title")}</b>
          <span>{t("web.pwa.ios.step1.desc")}</span>
        </span>
      </li>
      <li>
        <span class="step-ic" aria-hidden="true">
          <svg width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="3" width="18" height="18" rx="4" />
            <path d="M12 8v8" />
            <path d="M8 12h8" />
          </svg>
        </span>
        <span class="step-tx">
          <b>{t("web.pwa.ios.step2.title")}</b>
          <span>{t("web.pwa.ios.step2.desc")}</span>
        </span>
      </li>
      <li>
        <span class="step-ic accent-ic" aria-hidden="true">
          <Icon name="check" size={18} />
        </span>
        <span class="step-tx">
          <b>{t("web.pwa.ios.step3.title")}</b>
          <span>{t("web.pwa.ios.step3.desc")}</span>
        </span>
      </li>
    </ol>
  {:else if pwa.macSafari}
    <ol class="steps">
      <li>
        <span class="step-ic" aria-hidden="true"><Icon name="download" size={18} /></span>
        <span class="step-tx">
          <b>{t("web.pwa.mac.step1.title")}</b>
          <span>{t("web.pwa.mac.step1.desc")}</span>
        </span>
      </li>
      <li>
        <span class="step-ic accent-ic" aria-hidden="true"><Icon name="check" size={18} /></span>
        <span class="step-tx">
          <b>{t("web.pwa.mac.step2.title")}</b>
          <span>{t("web.pwa.mac.step2.desc")}</span>
        </span>
      </li>
    </ol>
  {:else}
    <ol class="steps">
      <li>
        <span class="step-ic" aria-hidden="true"><Icon name="ellipsis-vertical" size={18} /></span>
        <span class="step-tx">
          <b>{t("web.pwa.other.step1.title")}</b>
          <span>{t("web.pwa.other.step1.desc")}</span>
        </span>
      </li>
      <li>
        <span class="step-ic accent-ic" aria-hidden="true"><Icon name="smartphone" size={18} /></span>
        <span class="step-tx">
          <b>{t("web.pwa.other.step2.title")}</b>
          <span>{t("web.pwa.other.step2.desc")}</span>
        </span>
      </li>
    </ol>
  {/if}

  {#if !pwa.ios}
    <div class="foot">
      <a class="native" href={GET_APP_URL} target="_blank" rel="noopener noreferrer" onclick={close}>
        <span class="nt">
          <b>{t("web.pwa.native.title")}</b>
          <span>{t("web.pwa.native.sub")}</span>
        </span>
        <Icon name="external-link" size={15} color="var(--dim)" />
      </a>
    </div>
  {/if}
{/snippet}

<div
  class="dim"
  class:modal={!app.compact}
  transition:fade|global={{ duration: 180 }}
  onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")}
  onpointerdown={close}
  role="presentation"
>
  {#if app.compact}
    <div
      class="sheet"
      style:transform="translateY({pull}px)"
      class:settle={pull === 0}
      in:sheetIn|global={{ duration: 260 }}
      out:sheetOut|global={{ duration: 200 }}
      onpointerdown={(e) => e.stopPropagation()}
      role="dialog"
      aria-label={t("web.pwa.title")}
      tabindex="-1"
      use:drag={{
        axis: "y",
        move: (_, dy) => (pull = Math.max(0, dy)),
        end: (_, dy, _vx, vy) => {
          if (shouldClose(dy, vy)) { close(); return; }
          pull = 0;
        },
      }}
    >
      <span class="hdl"></span>
      <div class="inner">{@render body()}</div>
    </div>
  {:else}
    <div
      class="box"
      in:scale|global={{ start: 0.96, duration: 180, easing: cubicOut }}
      out:fade|global={{ duration: 130 }}
      onpointerdown={(e) => e.stopPropagation()}
      role="dialog"
      aria-label={t("web.pwa.title")}
      tabindex="-1"
    >
      {@render body()}
    </div>
  {/if}
</div>

<style>
  .dim { position: fixed; inset: 0; z-index: 70; background: rgba(0, 0, 0, .62); }
  .dim.modal { display: grid; place-items: center; padding: 20px; }

  .sheet {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    max-height: calc(100vh - var(--safe-top, env(safe-area-inset-top)) - 16px);
    max-height: calc(100dvh - var(--safe-top, env(safe-area-inset-top)) - 16px);
    border-radius: 18px 18px 0 0;
    background: hsl(0 0% 8%);
    padding-bottom: calc(18px + var(--safe-bottom, env(safe-area-inset-bottom)));
    display: flex;
    flex-direction: column;
    overflow-y: auto;
  }
  .sheet.settle { transition: transform .24s var(--ease); }
  .hdl { flex: none; display: block; width: 36px; height: 4px; margin: 10px auto 6px; border-radius: 2px; background: rgba(255, 255, 255, .25); }
  .inner { padding: 8px 20px 6px; display: flex; flex-direction: column; gap: 16px; }

  .box {
    width: min(440px, 100%);
    max-height: calc(100dvh - 40px);
    overflow-y: auto;
    padding: 20px;
    border-radius: 14px;
    background: #141414;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .hero { display: flex; align-items: center; gap: 14px; }
  .appicon { flex: none; width: 56px; height: 56px; border-radius: 14px; background: #050505; }
  .ht { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .ht h2 { margin: 0; font-size: 18px; font-weight: 700; color: #fff; line-height: 1.2; }
  .ht p { margin: 0; font-size: 13px; color: var(--dim); line-height: 1.35; }
  .xbtn { flex: none; align-self: flex-start; width: 32px; height: 32px; border-radius: 8px; display: grid; place-items: center; color: var(--dim); transition: background-color .12s, color .12s; }
  .xbtn:hover { background: rgba(255, 255, 255, .08); color: #fff; }

  .perks { display: flex; flex-wrap: wrap; gap: 6px; }
  .perk {
    height: 28px;
    padding: 0 10px;
    border-radius: 999px;
    background: rgba(255, 255, 255, .06);
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    font-weight: 500;
    color: rgba(255, 255, 255, .86);
  }

  .cta { display: flex; flex-direction: column; gap: 8px; }
  .cta :global(.full) { width: 100%; justify-content: center; height: 42px; font-size: 14px; }
  .hint { margin: 0; font-size: 12px; color: var(--faint); text-align: center; }

  .safari-note {
    padding: 10px 12px;
    border-radius: 10px;
    background: rgba(var(--accent-rgb), .12);
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .sn-tx { flex: 1; font-size: 12.5px; color: rgba(255, 255, 255, .9); line-height: 1.35; }
  .sn-copy {
    flex: none;
    height: 30px;
    padding: 0 10px;
    border-radius: 8px;
    background: rgba(255, 255, 255, .12);
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    font-weight: 600;
    color: #fff;
  }
  .sn-copy:active { background: rgba(255, 255, 255, .2); }

  .steps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .steps li {
    padding: 11px 12px;
    border-radius: 12px;
    background: rgba(255, 255, 255, .05);
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .step-ic {
    flex: none;
    width: 38px;
    height: 38px;
    border-radius: 10px;
    background: rgba(255, 255, 255, .08);
    color: #fff;
    display: grid;
    place-items: center;
  }
  .step-ic.accent-ic {
    background: rgba(var(--accent-rgb), .16);
    color: var(--accent);
  }
  .step-tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .step-tx b { font-size: 13.5px; font-weight: 600; color: #fff; }
  .step-tx span { font-size: 12.5px; color: var(--dim); line-height: 1.35; }

  .foot { padding-top: 2px; }
  .native {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 11px 12px;
    border-radius: 10px;
    background: rgba(255, 255, 255, .04);
    transition: background-color .12s;
  }
  .native:hover { background: rgba(255, 255, 255, .08); }
  .native:active { background: rgba(255, 255, 255, .11); }
  .nt { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .nt b { font-size: 13px; font-weight: 600; color: rgba(255, 255, 255, .92); }
  .nt span { font-size: 12px; color: var(--faint); }
</style>
