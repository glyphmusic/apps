<script lang="ts">
  // An error shown as a calm card instead of a raw line of text: round
  // alert icon, a plain title and sentence, the technical details folded
  // away (copyable), and buttons to try again or go back.
  import Icon from "./Icon.svelte";
  import Button from "./Button.svelte";
  import { t } from "../i18n.svelte";

  let { title, body, detail = "", onretry, onback, compact = false }: {
    title: string; body: string; detail?: string; onretry?: () => void; onback?: () => void; compact?: boolean;
  } = $props();

  let open = $state(false);
  let copied = $state(false);
  async function copy() {
    try { await navigator.clipboard.writeText(detail); copied = true; setTimeout(() => (copied = false), 1600); } catch { /* no clipboard */ }
  }
</script>

<div class="card" class:compact role="alert">
  <span class="ic"><Icon name="circle-alert" size={compact ? 22 : 26} color="var(--warn)" /></span>
  <h2>{title}</h2>
  <p>{body}</p>
  <div class="btns">
    {#if onretry}<Button kind="primary" icon="refresh-cw" onclick={onretry}>{t("common.retry")}</Button>{/if}
    {#if onback}<Button onclick={onback} icon="arrow-left">{t("common.back")}</Button>{/if}
  </div>
  {#if detail}
    <button class="tog" onclick={() => (open = !open)} aria-expanded={open}>
      {t("error.details")}<span class="chev" class:open><Icon name="chevron-down" size={14} /></span>
    </button>
    {#if open}
      <div class="det">
        <pre class="mono">{detail}</pre>
        <button class="cp" onclick={copy}><Icon name={copied ? "check" : "copy"} size={14} />{copied ? t("error.copied") : t("error.copy")}</button>
      </div>
    {/if}
  {/if}
</div>

<style>
  .card { width: 440px; max-width: calc(100vw - 32px); padding: 28px; border-radius: 16px; background: #141414; display: flex; flex-direction: column; align-items: center; gap: 10px; text-align: center; animation: rise-in .3s var(--ease) both; }
  .card.compact { background: #141414; }
  .ic { width: 52px; height: 52px; margin-bottom: 4px; border-radius: 50%; background: #262016; display: grid; place-items: center; }
  .compact .ic { width: 44px; height: 44px; }
  h2 { margin: 0; font-size: 17px; font-weight: 600; color: var(--text); }
  p { margin: 0; max-width: 360px; font-size: var(--md); line-height: 1.45; color: var(--dim); }
  .btns { display: flex; gap: 8px; margin-top: 8px; }
  .btns:empty { display: none; }
  .tog { margin-top: 6px; display: inline-flex; align-items: center; gap: 4px; font-size: var(--sm); color: var(--faint); transition: color .12s; }
  .tog:hover { color: var(--dim); }
  .chev { display: inline-flex; transition: transform .2s var(--ease); }
  .chev.open { transform: rotate(180deg); }
  .det { width: 100%; display: flex; flex-direction: column; gap: 8px; align-items: flex-end; animation: fade-in .18s ease-out; }
  pre { width: 100%; max-height: 160px; margin: 0; padding: 10px 12px; overflow: auto; border-radius: 8px; background: rgba(255, 255, 255, .05); font-size: 11.5px; line-height: 1.5; color: var(--dim); text-align: left; white-space: pre-wrap; word-break: break-word; user-select: text; }
  .cp { display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 10px; border-radius: 8px; font-size: var(--sm); color: var(--dim); transition: background-color .12s, color .12s; }
  .cp:hover { background: rgba(255, 255, 255, .06); color: var(--text); }
  /* Phones: no box, just a calm screen, with big buttons a thumb can hit and
     the details tucked away below them. */
  :global(html.compact) .card { width: 100%; max-width: 420px; padding: 24px 8px; gap: 12px; background: none; border-radius: 0; }
  :global(html.compact) .ic { width: 64px; height: 64px; margin-bottom: 8px; }
  :global(html.compact) h2 { font-size: 20px; font-weight: 700; }
  :global(html.compact) p { font-size: 15px; }
  :global(html.compact) .btns { align-self: stretch; flex-direction: column; gap: 10px; margin-top: 18px; }
  :global(html.compact) .btns :global(.btn) { width: 100%; height: 50px; justify-content: center; border-radius: 25px; font-size: 15px; }
  :global(html.compact) .tog { margin-top: 10px; min-height: 40px; }
  :global(html.compact) pre { max-height: 40vh; }
</style>
