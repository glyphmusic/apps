<script lang="ts">
  // An AI assistant asks for access (OAuth sign-in through the server): the
  // user compares the code with the browser page and allows or declines
  // (controls.rs mcp_approval). Clicking outside does not decline; the
  // request stays until answered. Same build as ConfirmDialog.
  import { fade, scale } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import Button from "../components/Button.svelte";
  import { answerMcp, mcpq } from "../settings/plugin-events.svelte";
  import { t, tf } from "../i18n.svelte";

  const a = $derived(mcpq.approvals[0]);
</script>

{#if a}
  <div class="layer" transition:fade|global={{ duration: 160 }}>
    <div class="tdim"></div>
    <div class="bd">
      <div class="panel" transition:scale|global={{ start: 0.97, duration: 180, easing: cubicOut }} role="alertdialog" tabindex="-1">
        <div class="head">
          <Icon name="bot" size={20} color="var(--accent)" />
          <div class="tt">{tf("mcp.approve.title", { client: a.client })}</div>
          <IconButton name="x" size={18} onclick={() => answerMcp(a.id, false)} />
        </div>
        <div class="body">{t("mcp.approve.body")}</div>
        <div class="col">
          <div class="code mono">{a.code}</div>
          <div class="hint">{t("mcp.approve.hint")}</div>
        </div>
        <div class="foot">
          <span class="flex1"></span>
          <Button onclick={() => answerMcp(a.id, false)}>{t("mcp.approve.deny")}</Button>
          <Button kind="primary" onclick={() => answerMcp(a.id, true)}>{t("mcp.approve.allow")}</Button>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .layer { position: fixed; inset: 0; z-index: 92; }
  .tdim { position: absolute; left: 0; right: 0; top: 0; height: var(--titlebar-h); background: rgba(0, 0, 0, .55); pointer-events: none; }
  .bd { position: absolute; top: var(--titlebar-h); left: 0; right: 0; bottom: 0; padding-bottom: var(--titlebar-h); background: rgba(0, 0, 0, .55); display: flex; align-items: center; justify-content: center; }
  .panel { width: 460px; max-width: calc(100vw - 32px); padding: 24px; border-radius: 12px; background: #171717; display: flex; flex-direction: column; gap: 18px; }
  .head { display: flex; align-items: center; gap: 10px; }
  .tt { flex: 1; font-size: 20px; font-weight: 700; color: rgba(255, 255, 255, .95); }
  .body { font-size: 14px; line-height: 1.45; color: rgba(255, 255, 255, .7); }
  .col { display: flex; flex-direction: column; gap: 8px; }
  .code { height: 64px; display: flex; align-items: center; justify-content: center; border-radius: 8px; background: rgba(255, 255, 255, .07); font-size: 26px; font-weight: 700; color: var(--accent); }
  .hint { font-size: var(--sm); color: rgba(255, 255, 255, .45); }
  .foot { display: flex; align-items: center; gap: 8px; }
  .flex1 { flex: 1; }
  .mono { font-family: var(--mono, ui-monospace, monospace); }
</style>
