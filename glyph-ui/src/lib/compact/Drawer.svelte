<script lang="ts">
  // compact.rs drawer: the profile menu from the left, 86 % of the width (max
  // 360), surface hsla(0 0 7%); profile row (52 px picture, @handle 19 bold),
  // a rule, then Stats, Recently played, Downloads, Transfer music, Settings.
  // Tap outside or swipe left closes it.
  import { fade } from "svelte/transition";
  import Icon from "../components/Icon.svelte";
  import Avatar from "./Avatar.svelte";
  import { app } from "../state.svelte";
  import { onWeb } from "../web";
  import { openInstallSheet, pwa } from "../pwa.svelte";
  import { t } from "../i18n.svelte";
  import { admin } from "../admin/admin.svelte";
  import { phone, drag, sheetIn, sheetOut, shouldClose } from "./cstate.svelte";
  let sx = $state(0);
  $effect(() => admin.onResume(app.account?.handle, app.backendState.kind === "ready"));
  const close = () => (phone.drawerOpen = false);
  function go(f: () => void) { close(); f(); }
  const rows = $derived([
    { icon: "sparkles", label: t("taste.title"), run: () => app.navigate("taste") },
    { icon: "chart-column", label: t("stats.title"), run: () => app.navigate("stats") },
    { icon: "history", label: t("home.recent"), run: () => app.openListView({ kind: "recent" }) },
    ...(app.library.downloads.length ? [{ icon: "arrow-down-to-line", label: t("library.downloads"), run: () => app.openListView({ kind: "downloads" }) }] : []),
    ...(onWeb() ? [] : [
      { icon: "users", label: t("jam.title"), run: () => { app.showQueue = false; app.showDevices = true; app.jamOpen = true; } },
      { icon: "arrow-right-left", label: t("library.migrate"), run: () => app.openSettings("migrate") },
    ]),
    ...(onWeb() && !pwa.installed ? [{ icon: "smartphone", label: t(pwa.ios ? "web.pwa.add_home" : "web.install_app"), run: openInstallSheet }] : []),
    ...(admin.on ? [{ icon: "server", label: t("admin.title"), run: () => { admin.sub = ""; app.navigate("server"); } }] : []),
    { icon: "settings", label: t("nav.settings"), run: () => app.openSettings() },
  ]);
</script>

<div class="back" transition:fade|global={{ duration: 200 }} onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")} onpointerdown={close} role="presentation"
  use:drag={{ axis: "x", move: (dx) => (sx = Math.min(0, dx)), end: (dx, _dy, vx) => { if (shouldClose(-dx, -vx)) { close(); return; } sx = 0; } }}>
  <div class="panel" style:transform="translateX({sx}px)" in:sheetIn|global={{ axis: "x", dir: -1, duration: 240 }} out:sheetOut|global={{ axis: "x", dir: -1, duration: 200 }} onpointerdown={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
    <button class="prof" onclick={() => go(() => (app.account ? app.openProfilePage(app.account.handle) : app.openSettings("account")))}>
      <Avatar size={52} />
      <span class="pt">
        <span class="n trunc">{app.account ? `@${app.account.handle}` : t("mobile.signed_out")}</span>
        <span class="s">{app.account ? t("profile.view") : t("account.signin.button")}</span>
      </span>
    </button>
    <span class="rule"></span>
    {#each rows as r}
      <button class="mr" onclick={() => go(r.run)}>
        <Icon name={r.icon} size={22} color="rgba(255,255,255,.85)" />
        <span class="trunc">{r.label}</span>
      </button>
    {/each}
  </div>
</div>

<style>
  .back { position: fixed; inset: 0; z-index: 60; background: rgba(0, 0, 0, .55); touch-action: pan-y; }
  .panel { position: absolute; top: 0; bottom: 0; left: 0; width: min(86vw, 360px); background: hsl(0 0% 7%); padding: calc(var(--safe-top, env(safe-area-inset-top)) + 8px) 0 var(--safe-bottom, env(safe-area-inset-bottom)); display: flex; flex-direction: column; transition: transform .2s var(--ease); }
  .prof { display: flex; align-items: center; gap: 14px; padding: 14px 16px; text-align: left; }
  .prof:active, .mr:active { background: rgba(255, 255, 255, .06); }
  .pt { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .n { font-size: 19px; font-weight: 700; color: #fff; }
  .s { font-size: 13px; color: rgba(255, 255, 255, .6); }
  .rule { height: 1px; margin: 0 16px 6px; background: rgba(255, 255, 255, .1); }
  .mr { display: flex; align-items: center; gap: 16px; min-height: 52px; padding: 8px 16px; font-size: 15px; color: #fff; text-align: left; }
</style>
