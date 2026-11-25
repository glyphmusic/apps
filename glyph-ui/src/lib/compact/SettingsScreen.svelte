<script lang="ts">
  // compact.rs settings: a full screen list (quick tiles for data saver,
  // equalizer, hide explicit; then one row per page with its icon and a
  // sub line). A page opens full screen with its sub-tab chips on top.
  // The page bodies come from the desktop settings (lib/settings/pages.ts).
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Icon from "../components/Icon.svelte";
  import { app, type SettingsPage } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { phone, settingsBack } from "./cstate.svelte";
  import { fadeEdges } from "../ui.svelte";
  import { onWeb } from "../web";
  import Tog from "../settings/Tog.svelte";
  import { adv, setAdvanced, subHidden } from "../settings/advanced.svelte";
  import { GROUPS, pageComponent, pageInfo, pageShown, subtabs } from "../settings/pages";

  const order = $derived<SettingsPage[]>((onWeb() ? [
    "overview", "account",
    "playback", "appearance", "transfer", "integrations", "about",
  ] : [
    "overview", "account",
    "playback", "audio", "appearance", "storage", "migrate", "transfer", "server", "plugins", "notifications", "about",
  ]).filter((p) => pageShown(p as SettingsPage)) as SettingsPage[]);
  // The rows in cards by group (like the desktop nav: overview, then
  // preferences, then the app), so the long list reads in three parts.
  const grouped = $derived(
    GROUPS.map((g) => ({ label: g.label, pages: order.filter((p) => g.pages.includes(p)) }))
      .concat([{ label: "", pages: order.filter((p) => !GROUPS.some((g) => g.pages.includes(p))) }])
      .filter((g) => g.pages.length),
  );
  const page = $derived(app.settingsPage);
  const Body = $derived(pageComponent(page));
  let pageEl: HTMLElement | undefined = $state();
  const tabs = $derived(subtabs(page));
  // Sub-tabs keep their index (the page body maps it), Advanced ones just hide.
  const shownTabs = $derived(tabs.map((k, i) => ({ k, i })).filter((x) => !subHidden(x.k)));
  const sub = $derived.by(() => {
    const s = Math.min(phone.settingsSub, Math.max(0, tabs.length - 1));
    return tabs[s] && subHidden(tabs[s]) ? 0 : s;
  });
  function open(p: SettingsPage) { if (p !== app.settingsPage) phone.settingsSub = 0; app.settingsPage = p; phone.settingsInPage = true; }
  // Opened for one page (a tapped notification): straight into it.
  $effect(() => { const d = app.settingsDirect; if (d) { app.settingsDirect = null; open(d); } });
  const quick = $derived([
    ...(onWeb() ? [
      { icon: "chart-column", label: t("sp.stats"), on: app.config.stats.enabled, run: () => app.setConfig((c) => (c.stats.enabled = !c.stats.enabled)) },
      { icon: "sparkles", label: t("sp.appearance.ascii_fx"), on: app.config.visuals.ascii_effects, run: () => app.setConfig((c) => (c.visuals.ascii_effects = !c.visuals.ascii_effects)) },
    ] : [
      { icon: "chart-column", label: t("sp.storage.saver"), on: app.config.storage.data_saver, run: () => app.setConfig((c) => (c.storage.data_saver = !c.storage.data_saver)) },
      { icon: "sliders-horizontal", label: t("sp.eq"), on: app.config.audio.eq_enabled, run: () => app.setConfig((c) => (c.audio.eq_enabled = !c.audio.eq_enabled)) },
    ]),
    { icon: "eye-off", label: t("sp.explicit"), on: app.config.audio.hide_explicit, run: () => app.setConfig((c) => (c.audio.hide_explicit = !c.audio.hide_explicit)) },
  ]);
</script>

<div class="screen" transition:fly|global={{ x: 24, duration: 200, easing: cubicOut }} onoutrostart={(e) => ((e.currentTarget as HTMLElement).style.pointerEvents = "none")}>
  <div class="head">
    <button class="bk" onclick={settingsBack} aria-label={t("tip.chevron-left")}><Icon name="arrow-left" size={22} color="#fff" /></button>
    <h1>{phone.settingsInPage ? pageInfo(page).name : t("nav.settings")}</h1>
    <span class="sp"></span>
  </div>
  {#if phone.settingsInPage}
    {#if shownTabs.length}
      <!-- Sub-pages as tabs under the title (they stay put while the page scrolls):
           equal widths when they fit, else a row that scrolls sideways. -->
      <div class="tabs" class:fit={shownTabs.length <= 3} role="tablist">
        {#each shownTabs as { k, i }}
          <button class="tb" class:on={i === sub} role="tab" aria-selected={i === sub} onclick={() => { phone.settingsSub = i; pageEl?.scrollTo(0, 0); }}>{t(k)}</button>
        {/each}
      </div>
    {/if}
    <div class="scroll body page" class:tabbed={shownTabs.length > 0} bind:this={pageEl} use:fadeEdges={{ top: 18, bottom: 18 }}>
      {#if Body}<Body {sub} />{/if}
    </div>
  {:else}
    <div class="scroll body" use:fadeEdges={{ top: 18, bottom: 18 }}>
      <div class="quick">
        {#each quick as q}
          <button class="qt" class:on={q.on} onclick={q.run}>
            <Icon name={q.icon} size={22} color={q.on ? "var(--accent)" : "rgba(255,255,255,.8)"} />
            <span class="ql">{q.label}</span>
            <span class="qs">{q.on ? t("common.on") : t("common.off")}</span>
          </button>
        {/each}
      </div>
      {#each grouped as g}
        {#if g.label}<div class="gl">{t(g.label)}</div>{/if}
        <div class="grp">
          {#each g.pages as p}
            {@const info = pageInfo(p)}
            <button class="mr" onclick={() => open(p)}>
              <span class="mi"><Icon name={info.icon} size={20} color="rgba(255,255,255,.9)" /></span>
              <span class="mt"><span class="trunc">{info.name}</span>{#if info.sub}<span class="ms">{info.sub}</span>{/if}</span>
              <Icon name="chevron-right" size={18} color="rgba(255,255,255,.35)" />
            </button>
          {/each}
        </div>
      {/each}
      <div class="adv">
        <span class="mt"><span class="trunc">{t("settings.advanced")}</span><span class="ms">{t("settings.advanced.sub")}</span></span>
        <Tog on={adv.on} label={t("settings.advanced")} onchange={setAdvanced} />
      </div>
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
  .tabs { flex: none; display: flex; gap: 2px; padding: 0 10px; overflow-x: auto; scrollbar-width: none; }
  .tabs::-webkit-scrollbar { display: none; }
  .tb { position: relative; flex: none; height: 46px; padding: 0 14px; border-radius: 10px 10px 0 0; font-size: 14.5px; font-weight: 600; color: rgba(255, 255, 255, .5); white-space: nowrap; text-align: center; transition: color .15s ease-out; }
  .tabs.fit .tb { flex: 1 1 0; min-width: 0; padding: 0 6px; overflow: hidden; text-overflow: ellipsis; }
  .tb.on { color: #fff; }
  .tb.on::after { content: ""; position: absolute; left: 22%; right: 22%; bottom: 0; height: 3px; border-radius: 3px 3px 0 0; background: var(--accent); }
  .tb:active { background: rgba(255, 255, 255, .05); }
  .page.tabbed { padding-top: 14px; }
  .quick { display: flex; gap: 8px; padding: 0 16px 10px; }
  .qt { flex: 1; min-width: 0; height: 84px; padding: 0 6px; border-radius: 10px; background: rgba(255, 255, 255, .07); display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 4px; }
  .qt.on { background: rgba(255, 255, 255, .14); }
  .qt:active { background: rgba(255, 255, 255, .18); }
  .ql { max-width: 100%; font-size: 12px; line-height: 14px; color: #fff; text-align: center; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; overflow: hidden; }
  .qs { font-size: 11px; color: rgba(255, 255, 255, .55); }
  .gl { padding: 14px 28px 6px; font-size: 12px; font-weight: 600; letter-spacing: .04em; text-transform: uppercase; color: rgba(255, 255, 255, .45); }
  .grp { margin: 0 12px; border-radius: 14px; background: rgba(255, 255, 255, .04); overflow: hidden; }
  .grp + .grp { margin-top: 10px; }
  .mi { flex: none; width: 34px; height: 34px; border-radius: 9px; display: grid; place-items: center; background: rgba(255, 255, 255, .07); }
  .mr { width: 100%; display: flex; align-items: center; gap: 14px; min-height: 56px; padding: 10px 12px 10px 12px; text-align: left; }
  .mr + .mr { border-top: 1px solid rgba(255, 255, 255, .05); }
  .mr:active { background: rgba(255, 255, 255, .06); }
  .mt { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; font-size: 15px; color: #fff; }
  .adv { display: flex; align-items: center; gap: 16px; min-height: 52px; margin: 12px 12px 0; padding: 8px 12px; border-radius: 14px; background: rgba(255, 255, 255, .04); }
  .ms { font-size: 12.5px; color: rgba(255, 255, 255, .55); }
</style>
