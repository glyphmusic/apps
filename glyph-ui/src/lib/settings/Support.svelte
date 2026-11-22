<script lang="ts">
  // settings/about.rs support: Settings > About > Support. Report a bug
  // (opens the issue form), copy the diagnostics (version, OS, setup; no
  // keys, no personal data), the docs and the data folder.
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import { keyLabel, keyOf } from "../keys";
  import Icon from "../components/Icon.svelte";
  import Rows from "./Rows.svelte";
  import Tog from "./Tog.svelte";
  import { sui } from "./settings.svelte";
  import { onWeb } from "../web";
  import { copyText, isMobile, openUrl } from "./util";

  const ISSUES_URL = "https://github.com/glyphmusic/apps/issues/new";
  const DOCS_URL = "https://docs.glyph.my";
  const REPO_URL = "https://github.com/glyphmusic/apps";

  let copied = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const mobile = $derived(isMobile());
  const shakeOn = $derived(app.config?.ui.shake_feedback !== false);
  const feedbackKey = $derived(keyLabel(keyOf("feedback")));
  const feedbackDesc = $derived(
    mobile
      ? t("sp.support.feedback.desc_mobile")
      : feedbackKey
        ? tf("sp.support.feedback.desc_key", { key: feedbackKey })
        : t("sp.support.feedback.desc"),
  );

  function openFeedback() {
    app.feedbackOpen = true;
  }

  function setShake(on: boolean) {
    app.setConfig((c) => {
      c.ui.shake_feedback = on;
    });
  }

  async function diag() {
    const text = await backend().call<string>("diagnostics");
    if (text && (await copyText(text))) {
      // The button turns into "Copied": no toast on top.
      copied = true;
      clearTimeout(timer);
      timer = setTimeout(() => { copied = false; }, 1800);
    }
  }
</script>

{#snippet actionRow(icon: string, label: string, desc: string, trail: string, run: () => void, accentTrail = false)}
  {@const lit = sui.flash?.label === label}
  <button class="srow item" class:lit onclick={run}>
    <span class="ic"><Icon name={icon} size={16} color="rgba(255,255,255,.78)" /></span>
    <span class="tx">
      <span class="lb">{label}</span>
      <span class="ds">{desc}</span>
    </span>
    <span class="tr">
      <Icon name={trail} size={14} color={accentTrail ? "var(--ok)" : "rgba(255,255,255,.42)"} />
    </span>
  </button>
{/snippet}

<Rows>
  {@render actionRow("message-square-more", t("feedback.title"), feedbackDesc, "chevron-right", openFeedback)}
  {#if mobile}
    <div class="srow item static">
      <span class="ic"><Icon name="smartphone" size={16} color="rgba(255,255,255,.78)" /></span>
      <span class="tx">
        <span class="lb">{t("sp.support.shake")}</span>
        <span class="ds">{t("sp.support.shake.desc")}</span>
      </span>
      <Tog on={shakeOn} label={t("sp.support.shake")} onchange={setShake} />
    </div>
  {/if}
  {@render actionRow("life-buoy", t("sp.support.issue"), t("sp.support.issue.desc"), "external-link", () => openUrl(ISSUES_URL))}
  {@render actionRow(copied ? "check" : "copy", t("sp.support.diag"), t("sp.support.diag.desc"), copied ? "check" : "copy", diag, copied)}
  {@render actionRow("book-open", t("sp.docs"), t("sp.support.docs.desc"), "external-link", () => openUrl(DOCS_URL))}
  {@render actionRow("github", t("sp.support.repo"), t("sp.support.repo.desc"), "external-link", () => openUrl(REPO_URL))}
  {#if !onWeb() && !isMobile()}
    {@render actionRow("folder-open", t("sp.support.data"), t("sp.support.data.desc"), "chevron-right", () => backend().call("reveal_data_dir"))}
  {/if}
</Rows>

<style>
  .item { position: relative; width: 100%; display: flex; align-items: center; gap: 14px; padding: 14px 0; text-align: left; }
  .item::before { content: ""; position: absolute; inset: 4px -10px; border-radius: 10px; pointer-events: none; transition: background-color .12s; }
  .item:hover::before { background: rgba(255, 255, 255, .045); }
  .item.static:hover::before { background: transparent; }
  .item.lit::before { animation: sflash 1.8s linear forwards; }
  @keyframes sflash { 0%, 39% { background: rgba(var(--accent-rgb, 255, 95, 210), .14); } 100% { background: transparent; } }
  .ic, .tx, .tr { position: relative; }
  .ic { flex: none; width: 32px; height: 32px; border-radius: 8px; background: rgba(255, 255, 255, .06); display: grid; place-items: center; }
  .tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .lb { font-size: 14px; font-weight: 500; color: #fff; }
  .ds { font-size: 12.5px; color: rgba(255, 255, 255, .52); }
  .tr { flex: none; display: grid; place-items: center; padding-right: 4px; }
</style>
