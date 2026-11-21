<script lang="ts">
  // settings/nav.rs nav: w 232, search on top (h 34, mx 6, px 10, radius 8,
  // white 6 %, 13 px, icon 14), then the list that scrolls: group labels
  // (12 px, white 45 %), pages (h 36, px 10, radius 8, icon 16, 14 px; the
  // open one white 8 % fill; the others white 75 % -> white on hover with a
  // 4 % fill), a chevron when the page has sub-tabs. Sub-tabs hang off a thin
  // line (ml 18, pl 8, border white 10 %), h 32, 13.5 px, and fold open /
  // shut over 180 ms. The search finds pages, sub-tabs and single options;
  // an option jumps to its page and its row lights up.
  import { slide } from "svelte/transition";
  import { cubicInOut } from "svelte/easing";
  import { app, type SettingsPage } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import Icon from "../components/Icon.svelte";
  import { GROUPS, OPTIONS, PAGE_WORDS, pageInfo, pageShown, subtabs } from "./pages";
  import { onWeb } from "../web";
  import { fadeEdges } from "../ui.svelte";
  import { clickPage, flashOption, setTab, sui } from "./settings.svelte";
  import { isMobile } from "./util";
  import Tog from "./Tog.svelte";
  import { adv, ADVANCED_ROWS, ADVANCED_SUBTABS, pageAdvanced, setAdvanced, subHidden } from "./advanced.svelte";
  import { migrateRunning } from "./migrate-state.svelte";

  const q = $derived(sui.query.trim().toLowerCase());
  const transferActive = $derived(migrateRunning() !== null);
  const hit = (text: string) => !q || text.toLowerCase().includes(q);
  const signedIn = $derived(app.account != null);
  const mobile = $derived(isMobile());

  const WEB_HIDDEN_KEYS = new Set([
    "sp.patient_search",
    "sp.search.mode",
    "sp.extractor",
    "ytdlp.title",
    "sp.player.np_download",
    "sp.appearance.mini",
    "sp.discord",
    "sp.discord.id",
    "sp.mcp.mode",
    "sp.support.data",
    "sp.keys.download",
  ]);
  const MOBILE_HIDDEN_KEYS = new Set([
    "sp.extractor",
    "ytdlp.title",
    "sp.player.lyrics",
    "sp.player.np_download",
    "sp.player.keys",
    "sp.appearance.mini",
    "sp.appearance.throttle",
    "sp.storage.dl_dir",
    "sp.storage.reveal",
    "sp.storage.local",
    "sp.support.data",
    "sp.keys.play",
    "sp.keys.zen",
    "sp.keys.download",
    "sp.player.wheel_seek",
    "sp.player.wheel_volume",
  ]);

  /** Single options matching the search (with their page / sub-tab). */
  /** An option that only shows with Advanced on (its page, sub-tab or row). */
  // OPTIONS count Playback sub-tabs the desktop way (the web player drops General).
  const PLAYBACK_TABS = ["sp.sub.general", "sp.sub.content", "settings.sources", "sp.sub.player"];
  const advOpt = (p: SettingsPage, s: number, key: string) =>
    pageAdvanced(p) || ADVANCED_ROWS.has(key) || ADVANCED_SUBTABS.has((p === "playback" ? PLAYBACK_TABS[s] : subtabs(p)[s]) ?? "");
  /** Options the search would find with Advanced on but hides now. */
  const hiddenHits = $derived(!q || adv.on ? 0 : OPTIONS.filter(([p, s, key, words]) => advOpt(p, s, key) && (hit(t(key)) || words.includes(q))).length);
  const found = $derived.by(() => {
    if (!q) return [];
    return OPTIONS
      .filter(([p]) => p !== "account" || signedIn)
      .filter(([p, s, key]) => adv.on || !advOpt(p, s, key))
      .filter(([p]) => pageShown(p))
      // Web player: only its own pages, supported Playback sub-tabs, and browser-supported options.
      .filter(([p, s, key]) => !onWeb() || (!WEB_HIDDEN_KEYS.has(key) && (p !== "playback" || s === 1 || s === 2 || (!mobile && s === 3))))
      .filter(([, , key]) => !onWeb() || signedIn || (key !== "sp.stats" && key !== "sp.mcp"))
      .filter(([, , key]) => !mobile || !MOBILE_HIDDEN_KEYS.has(key))
      .filter(([, , key, words]) => hit(t(key)) || words.includes(q))
      .slice(0, 14)
      .map(([p, s, key]) => ({ page: p, sub: onWeb() ? (p === "playback" ? s - 1 : p === "integrations" ? 0 : s) : s, label: t(key) }));
  });

  const groups = $derived(GROUPS.map((g) => ({
    label: t(g.label),
    items: g.pages.filter(pageShown).map((p) => {
      const subs = subtabs(p);
      const subHit = subs.map((k) => !!q && !subHidden(k) && hit(t(k)));
      const info = pageInfo(p);
      const page = hit(info.name) || PAGE_WORDS[p].includes(q) || found.some((f) => f.page === p);
      return { p, info, subs, subHit, show: page || subHit.includes(true) };
    }).filter((x) => x.show),
  })).filter((g) => g.items.length));

  function crumb(page: SettingsPage, sub: number): string {
    const subs = subtabs(page), k = subs[sub];
    const name = pageInfo(page).name;
    return k ? `${name} › ${t(k)}` : name;
  }
</script>

<div class="nav">
  <label class="search">
    <Icon name="search" size={14} color="rgba(255,255,255,.5)" />
    <input bind:value={sui.query} placeholder={t("settings.search")} spellcheck="false" autocomplete="off" aria-label={t("settings.search")} />
  </label>
  <div class="list scroll" use:fadeEdges={{ top: 18, bottom: 18, minOverflow: 12 }}>
    {#each groups as g (g.label)}
      {#if g.label}<div class="gl">{g.label}</div>{/if}
      {#each g.items as { p, info, subs, subHit } (p)}
        {@const on = p === app.settingsPage}
        {@const hasSubs = subs.length > 0}
        {@const open = hasSubs && !sui.collapsed[p] && (on || subHit.includes(true))}
        <button class="pg" class:on onclick={() => clickPage(p, hasSubs)}>
          <Icon name={info.icon} size={16} color={on ? "#fff" : "rgba(255,255,255,.7)"} />
          <span class="nm">{info.name}</span>
          {#if p === "migrate" && transferActive}<span class="trdot" aria-hidden="true"></span>{/if}
          {#if hasSubs}<span class="chev" class:open><Icon name="chevron-right" size={14} color="rgba(255,255,255,.45)" /></span>{/if}
        </button>
        {#if open}
          <div class="subs" transition:slide={{ duration: 180, easing: cubicInOut }}>
            {#each subs as key, i (key)}
              {#if !subHidden(key)}<button class="sub" class:sel={on && sui.sub === i} onclick={() => setTab(p, i)}>{t(key)}</button>{/if}
            {/each}
          </div>
        {/if}
      {/each}
    {/each}
    {#if found.length}
      <div class="gl found">{t("sp.group.options")}</div>
      {#each found as f (f.page + f.sub + f.label)}
        <button class="opt" onclick={() => { flashOption(f.label); setTab(f.page, f.sub); }}>
          <span class="ol">{f.label}</span>
          <span class="oc">{crumb(f.page, f.sub)}</span>
        </button>
      {/each}
    {/if}
    {#if hiddenHits}
      <button class="more" onclick={() => setAdvanced(true)}>{t("settings.advanced.more")}</button>
    {/if}
  </div>
  <div class="advrow">
    <span class="al">{t("settings.advanced")}</span>
    <Tog on={adv.on} label={t("settings.advanced")} onchange={setAdvanced} />
  </div>
</div>

<style>
  .nav { flex: none; width: 232px; height: 100%; display: flex; flex-direction: column; }
  .search { flex: none; height: 34px; margin: 6px; padding: 0 10px; display: flex; align-items: center; gap: 8px; border-radius: 8px; background: rgba(255, 255, 255, .06); font-size: 13px; cursor: text; }
  .search input { flex: 1; min-width: 0; border: 0; outline: 0; background: none; font-size: 13px; color: #fff; }
  .search input::placeholder { color: rgba(255, 255, 255, .35); }
  .list { flex: 1; min-height: 0; display: flex; flex-direction: column; gap: 2px; padding: 0 6px 6px; }
  .gl { flex: none; padding: 10px 10px 4px; font-size: 12px; color: rgba(255, 255, 255, .45); }
  .gl.found { padding-top: 14px; }
  .pg { flex: none; height: 36px; padding: 0 10px; display: flex; align-items: center; gap: 10px; border-radius: 8px; font-size: 14px; color: rgba(255, 255, 255, .75); transition: background-color .15s ease-out, color .15s ease-out; }
  .pg:hover { background: rgba(255, 255, 255, .04); color: #fff; }
  .pg.on { background: rgba(255, 255, 255, .08); color: #fff; }
  .nm { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: left; }
  .trdot { flex: none; width: 7px; height: 7px; border-radius: 50%; background: var(--accent); animation: trpulse 1.6s ease-in-out infinite; }
  @keyframes trpulse { 0%, 100% { opacity: 1; transform: scale(1); } 50% { opacity: .45; transform: scale(.82); } }
  .chev { display: flex; transition: transform .18s var(--ease); }
  .chev.open { transform: rotate(90deg); }
  .subs { flex: none; margin: 2px 0 2px 18px; padding-left: 8px; border-left: 1px solid rgba(255, 255, 255, .10); display: flex; flex-direction: column; gap: 2px; overflow: hidden; }
  .sub { flex: none; height: 32px; padding: 0 10px; display: flex; align-items: center; border-radius: 7px; font-size: 13.5px; color: rgba(255, 255, 255, .6); transition: background-color .15s ease-out, color .15s ease-out; }
  .sub:hover { background: rgba(255, 255, 255, .04); color: #fff; }
  .sub.sel { background: rgba(255, 255, 255, .08); color: #fff; font-weight: 500; }
  .more { flex: none; margin-top: 6px; padding: 8px 10px; border-radius: 8px; text-align: left; font-size: 12.5px; color: rgba(255, 255, 255, .55); transition: background-color .12s, color .12s; }
  .more:hover { background: rgba(255, 255, 255, .05); color: #fff; }
  .advrow { flex: none; height: 44px; margin: 0 6px 6px; padding: 0 10px; display: flex; align-items: center; gap: 10px; border-radius: 8px; background: rgba(255, 255, 255, .04); }
  .al { flex: 1; min-width: 0; font-size: 13.5px; color: rgba(255, 255, 255, .75); }
  .opt { flex: none; padding: 6px 10px; display: flex; flex-direction: column; justify-content: center; gap: 1px; border-radius: 8px; text-align: left; transition: background-color .12s; }
  .opt:hover { background: rgba(255, 255, 255, .05); }
  .ol { font-size: 13.5px; color: rgba(255, 255, 255, .9); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .oc { font-size: 11.5px; color: rgba(255, 255, 255, .45); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
