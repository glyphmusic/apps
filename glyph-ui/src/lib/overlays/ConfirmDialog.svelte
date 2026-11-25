<script lang="ts">
  import { eatTap } from "../ui.svelte";
  // Confirm dialog (app.confirm). GPUI has no shared confirm dialog; this
  // one is built from lists/dialogs.rs name_dialog's frame: modal_layer
  // with a black 45 % backdrop (titlebar dimmed, still draggable), panel
  // w 380, p 20, radius 12, hsla(0,0,.07,.99), 1 px white 8 %, gap 14; title
  // 16 semibold white 95 %, body sm dim, Cancel + the action (primary, or
  // the danger button for destructive ones). Escape cancels, Enter confirms.
  import { fade, scale } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Button from "../components/Button.svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";

  const c = $derived(app.confirm!);
  let ok: HTMLDivElement | undefined = $state();
  $effect(() => { ok?.querySelector("button")?.focus(); });
  function close() { app.confirm = null; }
  function confirm() { const run = c.run; close(); run(); }
  function key(e: KeyboardEvent) {
    if (e.key === "Escape") { e.preventDefault(); close(); }
    else if (e.key === "Enter") { e.preventDefault(); confirm(); }
  }
</script>

<svelte:window onkeydown={key} />

<div class="layer" transition:fade|global={{ duration: 160 }}>
  <div class="tdim"></div>
  <div class="bd" onpointerdown={(e) => { if (e.target === e.currentTarget) { eatTap(); close(); } }} role="presentation">
    <div class="panel" data-part="dialog" transition:scale|global={{ start: 0.97, duration: 180, easing: cubicOut }} role="alertdialog" tabindex="-1">
      <div class="tt">{c.title}</div>
      {#if c.body}<div class="body">{c.body}</div>{/if}
      <div class="foot">
        <span class="flex1"></span>
        <Button onclick={close}>{t("dialog.cancel")}</Button>
        <div class="ok" bind:this={ok}><Button kind={c.danger ? "danger" : "primary"} onclick={confirm}>{c.ok}</Button></div>
      </div>
    </div>
  </div>
</div>

<style>
  .layer { position: fixed; inset: 0; z-index: 91; }
  .tdim { position: absolute; left: 0; right: 0; top: 0; height: var(--titlebar-h); background: rgba(0, 0, 0, .45); pointer-events: none; }
  .bd { position: absolute; top: var(--titlebar-h); left: 0; right: 0; bottom: 0; padding-bottom: var(--titlebar-h); background: rgba(0, 0, 0, .45); display: flex; align-items: center; justify-content: center; }
  .panel { width: 380px; max-width: calc(100vw - 32px); padding: 20px; border-radius: 12px; background: #161616; display: flex; flex-direction: column; gap: 14px; }
  .tt { font-size: 16px; font-weight: 600; color: rgba(255, 255, 255, .95); }
  .body { font-size: var(--sm); line-height: 1.45; color: var(--dim); white-space: pre-line; }
  .foot { display: flex; align-items: center; gap: 8px; }
  .ok { display: flex; }
</style>
