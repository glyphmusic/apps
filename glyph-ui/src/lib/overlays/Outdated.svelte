<script lang="ts">
  // "Update Glyph" (app.outdated): the server takes this version no more
  // (its min_app_version, or a 426 answer). Covers the whole app, so nothing
  // half works, and cannot be closed. Built like ConfirmDialog's panel
  // (w 380, p 20, radius 12, #161616, gap 14, title 16 semibold, body sm
  // dim) with the update flow of Settings > About: "Update now" checks,
  // downloads and installs; then "Restart" (Android asks itself). Where
  // Glyph cannot update itself, "Download" opens the release page.
  import { fade, scale } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Button from "../components/Button.svelte";
  import Icon from "../components/Icon.svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import { errText } from "../settings/util";
  import type { UpdateState } from "../settings/types";

  const DOWNLOAD_URL = "https://glyph.my/download";
  const o = $derived(app.outdated!);
  const android = $derived(app.platform === "android");

  let busy = $state<"checking" | "installing" | null>(null);
  let progress = $state<[number, number] | null>(null);
  let installed = $state(false);
  let error = $state<string | null>(null);
  let page = $state<string | null>(null);

  async function update() {
    if (busy) return;
    error = null;
    busy = "checking";
    try {
      const s = await backend().call<UpdateState>("update_check", { manual: true });
      if (s?.kind !== "available" || s.downgrade) {
        // No newer version on this update channel: the download page has one.
        busy = null;
        page = DOWNLOAD_URL;
        void backend().call("open_url", { url: DOWNLOAD_URL });
        return;
      }
      if (!s.can_self_install) {
        busy = null;
        page = s.html_url || DOWNLOAD_URL;
        void backend().call("open_url", { url: page });
        return;
      }
      busy = "installing";
      progress = [0, 0];
      const poll = setInterval(async () => {
        const u = await backend().call<{ installing: [number, number] | null } | undefined>("update_state").catch(() => undefined);
        if (u?.installing) progress = u.installing;
      }, 700);
      try { await backend().call("update_install"); installed = true; }
      finally { clearInterval(poll); }
    } catch (e) {
      error = errText(e);
    }
    busy = null;
    progress = null;
  }
  const restart = () => backend().call("update_restart");
  const openPage = () => backend().call("open_url", { url: page ?? DOWNLOAD_URL });

  const line = $derived.by(() => {
    if (busy === "checking") return t("update.checking");
    if (busy === "installing" && progress) {
      const [d, n] = progress;
      return tf("update.downloading", { p: n > 0 ? Math.floor((d * 100) / n) : 0 });
    }
    if (installed) return t(android ? "update.android_confirm" : "update.installed");
    if (error) return tf("update.error", { e: error });
    return null;
  });
</script>

<div class="layer" transition:fade|global={{ duration: 160 }} data-part="dialog">
  <div class="panel" transition:scale|global={{ start: 0.97, duration: 180, easing: cubicOut }} role="alertdialog" aria-modal="true" aria-labelledby="outdated-title" tabindex="-1">
    <span class="ic"><Icon name="arrow-down-to-line" size={22} /></span>
    <div class="tt" id="outdated-title">{t("outdated.title")}</div>
    <div class="body">{o.min ? tf("outdated.body", { min: o.min, current: o.current }) : tf("outdated.body_any", { current: o.current })}</div>
    {#if line}<div class="line" class:err={!!error} class:ok={installed}>{line}</div>{/if}
    <div class="foot">
      {#if page}<Button onclick={openPage}>{t("outdated.download")}</Button>{/if}
      <span class="flex1"></span>
      {#if installed && !android}
        <Button kind="primary" onclick={restart}>{t("update.restart")}</Button>
      {:else}
        <Button kind="primary" onclick={update} disabled={!!busy}>{t("outdated.update")}</Button>
      {/if}
    </div>
  </div>
</div>

<style>
  .layer { position: fixed; inset: 0; z-index: 200; display: flex; align-items: center; justify-content: center; padding: 16px; background: rgba(0, 0, 0, .82); backdrop-filter: blur(14px); -webkit-backdrop-filter: blur(14px); }
  .panel { width: 380px; max-width: 100%; padding: 20px; border-radius: 12px; background: #161616; display: flex; flex-direction: column; gap: 14px; }
  .ic { width: 40px; height: 40px; border-radius: 50%; display: grid; place-items: center; background: rgba(255, 255, 255, .08); color: var(--accent); }
  .tt { font-size: 16px; font-weight: 600; color: rgba(255, 255, 255, .95); }
  .body { font-size: var(--sm); line-height: 1.45; color: var(--dim); }
  .line { font-size: var(--sm); color: rgba(255, 255, 255, .7); }
  .line.err { color: var(--warn); }
  .line.ok { color: var(--ok); }
  .foot { display: flex; align-items: center; gap: 8px; }
  .flex1 { flex: 1; }
</style>
