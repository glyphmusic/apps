<script lang="ts">
  // Quick feedback overlay: mobile bottom sheet (opened by shaking the phone)
  // with a "Don't show again" switch, and centered desktop modal (opened via
  // keyboard shortcut, default Ctrl+Shift+F / ⌘⇧F). Submitting opens a
  // pre-filled GitHub issue with the user's message.
  import { fade, scale } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import Button from "../components/Button.svelte";
  import Tog from "../settings/Tog.svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { buildFeedbackIssueUrl } from "../feedback";
  import { isMobile, openUrl } from "../settings/util";
  import { drag, sheetIn, sheetOut, shouldClose } from "../compact/cstate.svelte";

  let message = $state("");
  let pull = $state(0);
  let inputEl: HTMLTextAreaElement | undefined = $state();

  const compact = $derived(app.compact);
  const mobile = $derived(isMobile());
  const neverShowAgain = $derived(app.config?.ui.shake_feedback === false);
  const canSend = $derived(message.trim().length > 0);

  $effect(() => {
    if (!compact) inputEl?.focus();
  });

  function close() {
    app.feedbackOpen = false;
  }

  function setNeverShowAgain(off: boolean) {
    app.setConfig((c) => {
      c.ui.shake_feedback = !off;
    });
  }

  function send() {
    if (!canSend) return;
    const url = buildFeedbackIssueUrl(message, {
      version: app.version,
      platform: app.platform,
    });
    openUrl(url);
    message = "";
    close();
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      e.stopPropagation();
      send();
    }
  }
</script>

{#snippet body()}
  <div class="head">
    <span class="ic" aria-hidden="true">
      <Icon name="message-square-more" size={18} color="var(--accent)" />
    </span>
    <div class="ht">
      <h2>{t("feedback.title")}</h2>
      <p>{t("feedback.sub")}</p>
    </div>
    {#if !compact}
      <IconButton name="x" size={18} color="rgba(255,255,255,.6)" onclick={close} />
    {/if}
  </div>

  <div class="field-wrap" data-drag-own>
    <textarea
      class="input"
      bind:this={inputEl}
      bind:value={message}
      rows={compact ? 4 : 5}
      placeholder={t("feedback.placeholder")}
      aria-label={t("feedback.title")}
      onkeydown={onKeyDown}
    ></textarea>
  </div>

  {#if mobile}
    <div class="never-row">
      <div class="ntx">
        <span class="nlb">{t("feedback.never_show")}</span>
        <span class="nds">{t("feedback.never_show.sub")}</span>
      </div>
      <Tog on={neverShowAgain} label={t("feedback.never_show")} onchange={setNeverShowAgain} />
    </div>
  {/if}

  <div class="foot">
    <span class="gh-hint">
      <Icon name="github" size={14} color="var(--dim)" />
      <span>{t("feedback.github_note")}</span>
    </span>
    <span class="flex1"></span>
    <Button onclick={close}>{t("dialog.cancel")}</Button>
    <Button kind="primary" disabled={!canSend} onclick={send}>
      <span>{t("feedback.send")}</span>
      <Icon name="external-link" size={14} />
    </Button>
  </div>
{/snippet}

{#if compact}
  <div
    class="bs-dim"
    transition:fade|global={{ duration: 180 }}
    onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")}
    onpointerdown={close}
    role="presentation"
  >
    <div
      class="bs-panel"
      data-part="dialog"
      style:transform="translateY({pull}px)"
      class:settle={pull === 0}
      in:sheetIn|global={{ duration: 260 }}
      out:sheetOut|global={{ duration: 210 }}
      onpointerdown={(e) => e.stopPropagation()}
      role="dialog"
      aria-modal="true"
      aria-label={t("feedback.title")}
      tabindex="-1"
    >
      <div
        class="bs-grab"
        use:drag={{
          axis: "y",
          move: (_, dy) => (pull = Math.max(0, dy)),
          end: (_, dy, _vx, vy) => {
            if (shouldClose(dy, vy)) {
              close();
              return;
            }
            pull = 0;
          },
        }}
      >
        <span class="hdl"></span>
      </div>
      <div class="bs-inner">{@render body()}</div>
    </div>
  </div>
{:else}
  <div class="layer" transition:fade|global={{ duration: 160 }}>
    <div class="tdim"></div>
    <div
      class="bd"
      onpointerdown={(e) => e.target === e.currentTarget && close()}
      role="presentation"
    >
      <div
        class="panel"
        data-part="dialog"
        transition:scale|global={{ start: 0.97, duration: 180, easing: cubicOut }}
        role="dialog"
        aria-modal="true"
        aria-label={t("feedback.title")}
        tabindex="-1"
      >
        {@render body()}
      </div>
    </div>
  </div>
{/if}

<style>
  .layer { position: fixed; inset: 0; z-index: 92; }
  .tdim { position: absolute; left: 0; right: 0; top: 0; height: var(--titlebar-h); background: rgba(0, 0, 0, .52); pointer-events: none; }
  .bd { position: absolute; top: var(--titlebar-h); left: 0; right: 0; bottom: 0; padding: 16px; padding-bottom: calc(16px + var(--titlebar-h)); background: rgba(0, 0, 0, .52); display: flex; align-items: center; justify-content: center; }
  .panel { width: 460px; max-width: calc(100vw - 32px); padding: 20px; border-radius: 14px; background: #161616; display: flex; flex-direction: column; gap: 14px; font-family: var(--font); }

  .bs-dim { position: fixed; inset: 0; z-index: 92; background: rgba(0, 0, 0, .58); }
  .bs-panel { position: absolute; left: 0; right: 0; bottom: 0; max-height: min(86vh, calc(100dvh - var(--safe-top, env(safe-area-inset-top)) - 16px)); border-radius: 16px 16px 0 0; background: hsl(0 0% 8%); padding-bottom: max(14px, var(--safe-bottom, env(safe-area-inset-bottom)), var(--ime-bottom, 0px)); display: flex; flex-direction: column; overflow: hidden; font-family: var(--font); }
  .bs-panel.settle { transition: transform .24s var(--ease); }
  .bs-grab { flex: none; touch-action: none; padding: 10px 0 4px; }
  .hdl { display: block; width: 36px; height: 4px; margin: 0 auto; border-radius: 2px; background: rgba(255, 255, 255, .25); }
  .bs-inner { flex: 1; min-height: 0; padding: 6px 16px 8px; display: flex; flex-direction: column; gap: 14px; overflow-y: auto; }

  .head { display: flex; align-items: flex-start; gap: 12px; }
  .ic { flex: none; width: 36px; height: 36px; border-radius: 10px; background: rgba(var(--accent-rgb, 255, 95, 210), .14); display: grid; place-items: center; }
  .ht { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .ht h2 { margin: 0; font-size: 16.5px; font-weight: 700; color: rgba(255, 255, 255, .96); line-height: 1.25; }
  .ht p { margin: 0; font-size: 12.5px; color: var(--dim); line-height: 1.35; }

  .field-wrap { display: flex; }
  .input {
    width: 100%;
    min-height: 108px;
    padding: 12px;
    border: 0;
    border-radius: 10px;
    background: rgba(255, 255, 255, .07);
    color: rgba(255, 255, 255, .95);
    font-family: inherit;
    font-size: 13.5px;
    line-height: 1.45;
    resize: none;
    outline: none;
    transition: background-color .14s ease-out;
  }
  .input:focus { background: rgba(255, 255, 255, .1); }
  .input::placeholder { color: rgba(255, 255, 255, .36); }

  .never-row {
    padding: 10px 12px;
    border-radius: 10px;
    background: rgba(255, 255, 255, .045);
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .ntx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .nlb { font-size: 13.5px; font-weight: 500; color: rgba(255, 255, 255, .92); }
  .nds { font-size: 12px; color: var(--dim); line-height: 1.3; }

  .foot { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .gh-hint { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; color: var(--dim); }
  .flex1 { flex: 1; min-width: 8px; }

  @media (max-width: 480px) {
    .gh-hint { display: none; }
    .foot { justify-content: flex-end; }
  }
</style>
