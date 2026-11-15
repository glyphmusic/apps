<script lang="ts">
  // compact.rs settings: a full screen list (quick tiles for data saver,
  // equalizer, hide explicit; then one row per page with its icon and a
  // sub line). A page opens full screen with its sub-tab chips on top.
  // The page bodies come from the desktop settings (lib/settings/pages.ts).
  import type { Component } from "svelte";
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Icon from "../components/Icon.svelte";
  import Chip from "../components/Chip.svelte";
  import { app, type SettingsPage } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { phone } from "./cstate.svelte";
  import { fadeEdges } from "../ui.svelte";
  import { onWeb } from "../web";
  import { closeDetail, pl } from "../settings/plugins-state.svelte";

  interface PagesModule {
    PAGES: SettingsPage[];
    pageInfo(p: SettingsPage): { name: string; sub: string; icon: string };
    pageShown(p: SettingsPage): boolean;
    subtabs(p: SettingsPage): string[];
    pageComponent(p: SettingsPage): Component<{ sub?: number }>;
  }
  const mods = import.meta.glob<PagesModule>("../settings/pages.ts");
  let pages = $state<PagesModule | null>(null);
  $effect(() => { const load = mods["../settings/pages.ts"]; load?.().then((m) => (pages = m)); });

  const order = $derived<SettingsPage[]>((onWeb() ? [
    "overview", "account",
    "playback", "appearance", "integrations", "about",
  ] : [
    "overview", "account",
    "playback", "audio", "appearance", "storage", "migrate", "transfer", "server", "plugins", "about",
  ]).filter((p) => pages?.pageShown(p as SettingsPage) ?? true) as SettingsPage[]);
  const page = $derived(app.settingsPage);
  const Body = $derived(pages?.pageComponent(page));
  const tabs = $derived(pages?.subtabs(page) ?? []);
  const sub = $derived(Math.min(phone.settingsSub, Math.max(0, tabs.length - 1)));
  function back() {
    if (phone.settingsInPage && page === "plugins" && pl.detail && phone.settingsSub < 2) { closeDetail(); return; }
    if (phone.settingsInPage) phone.settingsInPage = false; else app.settingsOpen = false;
  }
  function open(p: SettingsPage) { if (p !== app.settingsPage) phone.settingsSub = 0; app.settingsPage = p; phone.settingsInPage = true; }
  const quick = $derived([
    ...(onWeb() ? [] : [{ icon: "chart-column", label: t("sp.storage.saver"), on: app.config.storage.data_saver, run: () => app.setConfig((c) => (c.storage.data_saver = !c.storage.data_saver)) },
    { icon: "sliders-horizontal", label: t("sp.eq"), on: app.config.audio.eq_enabled, run: () => app.setConfig((c) => (c.audio.eq_enabled = !c.audio.eq_enabled)) }]),
    { icon: "eye-off", label: t("sp.explicit"), on: app.config.audio.hide_explicit, run: () => app.setConfig((c) => (c.audio.hide_explicit = !c.audio.hide_explicit)) },
  ]);
</script>

<div class="screen" transition:fly|global={{ x: 24, duration: 200, easing: cubicOut }} onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")}>
  <div class="head">
    <button class="bk" onclick={back} aria-label={t("tip.chevron-left")}><Icon name="arrow-left" size={22} color="#fff" /></button>
    <h1>{phone.settingsInPage && pages ? pages.pageInfo(page).name : t("nav.settings")}</h1>
    <span class="sp"></span>
  </div>
  {#if phone.settingsInPage}
    <div class="scroll body page" use:fadeEdges={{ top: 18, bottom: 18 }}>
      {#if tabs.length}
        <div class="chips">{#each tabs as k, i}<Chip on={i === sub} onclick={() => (phone.settingsSub = i)}>{t(k)}</Chip>{/each}</div>
      {/if}
      {#if Body}<Body {sub} />{/if}
    </div>
  {:else}
    <div class="scroll body" use:fadeEdges={{ top: 18, bottom: 18 }}>
      <div class="quick">
        {#each quick as q}
          <button class="qt" class:on={q.on} onclick={q.run}>
            <Icon name={q.icon} size={22} color={q.on ? "var(--accent)" : "rgba(255,255,255,.8)"} />
            <span class="ql trunc">{q.label}</span>
            <span class="qs">{q.on ? t("common.on") : t("common.off")}</span>
          </button>
        {/each}
      </div>
      {#each order as p}
        {@const info = pages?.pageInfo(p)}
        <button class="mr" onclick={() => open(p)}>
          <Icon name={info?.icon ?? "settings"} size={22} color="rgba(255,255,255,.85)" />
          <span class="mt"><span class="trunc">{info?.name ?? p}</span>{#if info?.sub}<span class="ms">{info.sub}</span>{/if}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .screen { position: fixed; inset: 0; z-index: 50; background: hsl(0 0% 7%); padding-top: var(--safe-top, env(safe-area-inset-top)); display: flex; flex-direction: column; --ink: 242, 242, 242; }
  .head { flex: none; height: 60px; padding: 0 6px; display: flex; align-items: center; }
  .bk, .sp { flex: none; width: 44px; height: 44px; border-radius: 50%; display: grid; place-items: center; }
  .bk:active { background: rgba(255, 255, 255, .08); }
  h1 { flex: 1; margin: 0; text-align: center; font-size: 17px; font-weight: 700; color: #fff; }
  .body { flex: 1; min-height: 0; padding-bottom: calc(max(var(--safe-bottom, env(safe-area-inset-bottom)), var(--ime-bottom, 0px)) + 24px); }
  .page { padding-left: 16px; padding-right: 16px; }
  .chips { display: flex; flex-wrap: wrap; gap: 8px; padding-bottom: 12px; }
  .quick { display: flex; gap: 8px; padding: 0 16px 10px; }
  .qt { flex: 1; min-width: 0; height: 84px; padding: 0 6px; border-radius: 10px; background: rgba(255, 255, 255, .07); display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 4px; }
  .qt.on { background: rgba(255, 255, 255, .14); }
  .qt:active { background: rgba(255, 255, 255, .18); }
  .ql { max-width: 100%; font-size: 12px; color: #fff; }
  .qs { font-size: 11px; color: rgba(255, 255, 255, .55); }
  .mr { width: 100%; display: flex; align-items: center; gap: 16px; min-height: 52px; padding: 8px 16px; text-align: left; }
  .mr:active { background: rgba(255, 255, 255, .06); }
  .mt { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; font-size: 15px; color: #fff; }
  .ms { font-size: 12.5px; color: rgba(255, 255, 255, .55); }
</style>
