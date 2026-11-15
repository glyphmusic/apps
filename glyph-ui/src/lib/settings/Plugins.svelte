<script lang="ts">
  // settings/plugins.rs plugins_page + browse + plugin_ui.rs plugin_page:
  // Settings > Plugins. Sub-tab 0 installed (auto update, update all, rows
  // with preview 28, name, status and controls; empty state), 1 browse the
  // catalog from GitHub (checked + community, search, sort), 2+ a plugin's
  // own settings page. A plugin's detail view replaces either. Plugins are
  // other people's work: the own-risk note shows on the catalog and detail.
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import Button from "../components/Button.svelte";
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import Chip from "../components/Chip.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import { compact } from "../fmt";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import SectionLabel from "./SectionLabel.svelte";
  import Segmented from "./Segmented.svelte";
  import Tog from "./Tog.svelte";
  import Spin from "./Spin.svelte";
  import PluginHost from "./PluginHost.svelte";
  import PluginSection from "./PluginSection.svelte";
  import PluginPreview from "./PluginPreview.svelte";
  import PluginDetail from "./PluginDetail.svelte";
  import RiskNote from "./RiskNote.svelte";
  import { tip } from "../ui.svelte";
  import { sui } from "./settings.svelte";
  import { pl, loadPlugins, refreshCatalog, pluginUpdates, installPlugin, uninstallPlugin, setEnabled, reloadPlugins, showPlugin, themeId } from "./plugins-state.svelte";
  import { setTab } from "./settings.svelte";
  import { isMobile, openUrl } from "./util";
  import type { PluginListing, PluginInstalled } from "./types";
  let { sub = 0 }: { sub?: number } = $props();

  const WEB_URL = "https://glyph.my/plugins";
  const GUIDE_URL = "https://github.com/glyphmusic/plugins#readme";
  const ICON = "rgba(255,255,255,.7)";
  let query = $state("");

  onMount(() => { loadPlugins(); refreshCatalog(false); });

  const updates = $derived(pluginUpdates());
  const sort = $derived((app.config.plugins.sort as string) || "stars");
  const curTheme = $derived(app.config.visuals.theme);
  const ownIndex = $derived(sub - 2);

  /** Fuzzy match: every query character in order, an exact substring first. */
  function score(text: string, q: string): number | null {
    text = text.toLowerCase();
    const i = text.indexOf(q);
    if (i >= 0) return 1000 - i;
    let at = 0, s = 0;
    for (const ch of q) { const j = text.indexOf(ch, at); if (j < 0) return null; s -= j - at; at = j + 1; }
    return s;
  }
  function filterSort(list: PluginListing[]): PluginListing[] {
    const q = query.trim().toLowerCase();
    let out = list.map((l) => ({ l, s: q ? score(`${l.name} ${l.id} ${l.author} ${l.description}`, q) : 0 })).filter((x) => x.s != null);
    const key = (a: PluginListing, b: PluginListing) => sort === "name" ? a.name.localeCompare(b.name) : sort === "updated" ? (b.updated ?? "").localeCompare(a.updated ?? "") : (b.stars ?? 0) - (a.stars ?? 0);
    out = out.sort((a, b) => (q ? (b.s as number) - (a.s as number) : 0) || key(a.l, b.l));
    return out.map((x) => x.l);
  }
  const checkedHits = $derived(filterSort(pl.catalog?.checked ?? []));
  const communityHits = $derived(filterSort(pl.catalog?.community ?? []));

  const newer = (a: string, b: string) => {
    const pa = a.split(".").map((x) => parseInt(x, 10) || 0), pb = b.split(".").map((x) => parseInt(x, 10) || 0);
    for (let i = 0; i < Math.max(pa.length, pb.length); i++) if ((pa[i] ?? 0) !== (pb[i] ?? 0)) return (pa[i] ?? 0) > (pb[i] ?? 0);
    return false;
  };
  function metaLine(m: PluginListing, showUnchecked: boolean, stars?: number | null) {
    const p = [`v${m.version}`];
    if (m.author.trim()) p.push(m.author);
    if (stars != null) p.push(`★ ${compact(stars)}`);
    if (showUnchecked) p.push(t("plugins.unchecked"));
    if (m.description.trim()) p.push(m.description);
    return p.join(" · ");
  }
  function installedStatus(p: PluginInstalled) {
    let s = metaLine(p, !p.checked && !p.local);
    if (p.local) s += ` · ${t("plugins.local")}`;
    if (p.missing_credentials) s = `${t("plugins.setup.needed")} · ${s}`;
    if (p.stopped && p.enabled) s = `${t("plugins.stopped_short")} · ${s}`;
    else if (!p.enabled && p.kind !== "theme") s = `${t("plugins.off")} · ${s}`;
    return s;
  }
  const catalogStatus = $derived(pl.loading ? t("plugins.loading") : pl.error ? tf("plugins.error", { e: pl.error }) : pl.catalog?.partial ? t("plugins.partial") : t("plugins.source"));
  const ownPlugin = $derived(ownIndex >= 0 ? pl.ui.pages[ownIndex] : undefined);
  const ownInstalled = $derived(ownPlugin ? pl.installed.find((p) => p.id === ownPlugin.id) : undefined);
  const ownSections = $derived(ownPlugin ? pl.ui.sections.filter((s) => s.plugin === ownPlugin.id && s.page === "own") : []);
  const by = $derived(ownInstalled?.author.trim() ? ownInstalled.author : t("plugins.warn.someone"));
</script>

{#snippet prow(m: PluginListing | PluginInstalled, status: string, installed: boolean)}
  <div class="srow pr">
    <button class="open" onclick={() => showPlugin(m.id)}>
      <PluginPreview theme={m.theme} size={28} />
      <span class="px"><span class="pn">{m.name}</span><span class="ps">{status}</span></span>
    </button>
    <div class="pc">{#if installed}{@render ictl(m as PluginInstalled)}{:else}{@render listingControls(m)}{/if}</div>
  </div>
{/snippet}

{#snippet guide(installed: boolean)}
  {@const lit = sui.flash?.label === t("plugins.publish")}
  <div class="foot" class:lit>
    {#if installed}
      <div class="fgrp">
        {#if !isMobile()}
          <button class="fbtn" onclick={() => backend().call("plugins_folder")} use:tip={{ label: t("plugins.maker.desc"), wrap: true }}>
            <Icon name="folder-open" size={13} color="currentColor" />
            <span>{t("sp.storage.open")}</span>
          </button>
          <span class="fsep" aria-hidden="true">·</span>
        {/if}
        <button class="fbtn" onclick={reloadPlugins}>
          <Icon name="refresh-cw" size={12} color="currentColor" />
          <span>{t("plugins.reload")}</span>
        </button>
      </div>
    {/if}
    <div class="fgrp" class:right={installed}>
      <button class="fbtn" onclick={() => openUrl(WEB_URL)} use:tip={{ label: t("plugins.publish.desc"), wrap: true }}>
        <Icon name="globe" size={12} color="currentColor" />
        <span>{t("plugins.web")}</span>
      </button>
      <span class="fsep" aria-hidden="true">·</span>
      <button class="fbtn" onclick={() => openUrl(GUIDE_URL)} use:tip={{ label: t("plugins.publish.desc"), wrap: true }}>
        <Icon name="book-open" size={12} color="currentColor" />
        <span>{t("plugins.publish.open")}</span>
      </button>
    </div>
  </div>
{/snippet}

{#snippet listingControls(l: PluginListing)}
  {@const inst = pl.installed.find((p) => p.id === l.id)}
  {@const foreign = !!inst && inst.repo.toLowerCase() !== l.repo.toLowerCase()}
  {#if pl.busy[l.id]}<Spin />
  {:else if l.installable === "needs_newer_app"}<Chip kind="kind">{t("plugins.needs_update")}</Chip>
  {:else if l.installable === "broken"}<Chip kind="kind">{t("plugins.broken")}</Chip>
  {:else if foreign}<Chip kind="kind">{t("plugins.id_taken")}</Chip>
  {:else if inst && newer(l.version, inst.version)}<Button kind="primary" onclick={() => installPlugin(l)}>{tf("plugins.update_to", { v: l.version })}</Button>
  {:else if inst}<span class="ins"><Icon name="check" size={14} color="rgba(255,255,255,.55)" />{t("plugins.installed")}</span>
  {:else}<IconButton name="download" size={15} color={ICON} label={t("plugins.install")} onclick={() => installPlugin(l)} />{/if}
{/snippet}

{#if pl.detail && !(sub >= 2)}
  <PluginDetail id={pl.detail} />
{:else if sub >= 2}
  <!-- A plugin's own page (plugin_ui.rs plugin_page). -->
  {#if ownPlugin && ownInstalled}
    <div class="col">
      <div class="by">
        <span class="bi"><Icon name="puzzle" size={14} color="rgba(255,255,255,.45)" /></span>
        <span class="bt">{tf("plugins.page.by", { author: by })}</span>
        <button class="about" onclick={() => { setTab("plugins", 0); showPlugin(ownPlugin!.id); }}>{t("plugins.page.about")}</button>
      </div>
      {#if ownInstalled.missing_credentials}<div class="wn">{t("plugins.setup.missing")}</div>{/if}
      {#if ownInstalled.stopped}<div class="wn">{tf("plugins.stopped", { e: ownInstalled.stopped })}</div>{/if}
      {#each ownSections as s (s.title + s.after)}<PluginSection section={s} named={false} />{/each}
    </div>
  {/if}
{:else if sub === 1}
  <!-- ------------------------------------------------------------- browse -->
  <div class="col">
    <div class="bar" class:lit={sui.flash?.label === t("plugins.catalog")}>
      <label class="srch"><Icon name="search" size={14} color="rgba(255,255,255,.5)" /><input bind:value={query} placeholder={t("plugins.search")} spellcheck="false" autocomplete="off" /></label>
      <Segmented value={sort} options={[{ value: "stars", label: t("plugins.sort.stars") }, { value: "updated", label: t("plugins.sort.updated") }, { value: "name", label: t("plugins.sort.name") }]}
        onpick={(v) => app.setConfig((c) => { c.plugins.sort = v; })} />
      <IconButton name="refresh-cw" size={14} color="rgba(255,255,255,.55)" label={`${t("plugins.refresh")} · ${catalogStatus}`} onclick={() => refreshCatalog(true)} />
    </div>
    {#if pl.error || pl.catalog?.partial}
      <div class="cat-msg">{catalogStatus}</div>
    {/if}
    {#if !pl.catalog}
      <div class="sk">{#each [0, 1, 2, 3] as i}<div class="skr"><Skeleton w={44} h={44} radius={6} id={`pl-c${i}`} /><div class="skt"><Skeleton w="45%" h={12} radius={4} id={`pl-t${i}`} /><Skeleton w="25%" h={10} radius={4} id={`pl-a${i}`} /></div></div>{/each}</div>
    {:else}
      {#if query.trim() && !checkedHits.length && !communityHits.length}
        <div class="empty">{tf("plugins.no_match", { q: query.trim() })}</div>
      {:else}
        <SectionLabel text={t("plugins.checked")} />
        {#if !checkedHits.length}<div class="empty">{query.trim() ? t("plugins.none_match") : t("plugins.none")}</div>
        {:else}<Rows>{#each checkedHits as l (l.id)}{@render prow(l, metaLine(l, false, l.stars) + (pl.failed[l.id] ? ` · ${pl.failed[l.id]}` : ""), false)}{/each}</Rows>{/if}
        <SectionLabel text={t("plugins.community")} />
        {#if !communityHits.length}<div class="empty">{query.trim() ? t("plugins.none_match") : t("plugins.none_community")}</div>
        {:else}<Rows>{#each communityHits as l (l.id)}{@render prow(l, metaLine(l, false, l.stars) + (pl.failed[l.id] ? ` · ${pl.failed[l.id]}` : ""), false)}{/each}</Rows>{/if}
      {/if}
    {/if}
    <RiskNote />
    {@render guide(false)}
  </div>
{:else}
  <!-- ---------------------------------------------------------- installed -->
  <PluginHost page="plugins">
    <div class="col">
      {#if !pl.installed.length}
        <div class="none">
          <Icon name="puzzle" size={28} color="rgba(255,255,255,.35)" />
          <div class="nt">{t("plugins.empty")}</div>
          <div class="nd">{t("plugins.empty.desc")}</div>
          <Button kind="primary" onclick={() => setTab("plugins", 1)}>{t("plugins.browse")}</Button>
        </div>
      {:else}
        <Rows>
          <Row label={t("plugins.auto_update")} info={t("plugins.auto_update.desc")}>
            <Tog on={app.config.plugins.auto_update} onchange={(v) => app.setConfig((c) => { c.plugins.auto_update = v; })} label={t("plugins.auto_update")} />
          </Row>
          {#if updates.length > 1}
            <Row label={tf("plugins.updates", { n: updates.length })}>
              <Button onclick={() => updates.forEach((l) => installPlugin(l))}>{t("plugins.update_all")}</Button>
            </Row>
          {/if}
          {#each pl.installed as p (p.id)}
            {@render prow(p, installedStatus(p), true)}
          {/each}
        </Rows>
      {/if}
      {@render guide(true)}
    </div>
  </PluginHost>
{/if}

{#snippet ictl(p: PluginInstalled)}
  {@const own = pl.ui.pages.findIndex((x) => x.id === p.id)}
  {#if pl.busy[p.id]}<Spin />
  {:else}
    {#if p.update}<Button kind="primary" onclick={() => installPlugin(p.update!)}>{tf("plugins.update_to", { v: p.update.version })}</Button>{/if}
    {#if p.theme}
      {#if curTheme === themeId(p.id)}<Chip kind="kind">{t("plugins.in_use")}</Chip>{:else}<Button onclick={() => app.setConfig((c) => { c.visuals.theme = themeId(p.id); })}>{t("plugins.use")}</Button>{/if}
    {/if}
    {#if own >= 0}<IconButton name="sliders-horizontal" size={15} color={ICON} label={t("plugins.settings")} onclick={() => setTab("plugins", 2 + own)} />{/if}
    {#if p.kind !== "theme"}<Tog on={p.enabled} onchange={(v) => setEnabled(p.id, v)} label={p.enabled ? t("plugins.turn_off") : t("plugins.turn_on")} />{/if}
    <IconButton name="trash-2" size={15} color={ICON} label={t("plugins.remove")} onclick={() => uninstallPlugin(p.id)} />
  {/if}
{/snippet}

<style>
  .col { display: flex; flex-direction: column; }
  .pr { position: relative; display: flex; align-items: center; gap: 12px; padding: 14px 0; }
  .pr::before { content: ""; position: absolute; inset: 4px -10px; border-radius: 10px; pointer-events: none; transition: background-color .12s; }
  .pr:hover::before { background: rgba(255, 255, 255, .045); }
  .open, .pc { position: relative; }
  .open { flex: 1; min-width: 0; display: flex; align-items: center; gap: 14px; border-radius: 8px; }
  .open:focus-visible { outline: 1px solid rgba(255, 255, 255, .35); outline-offset: 4px; }
  .px { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .pn, .ps { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pn { font-size: 14px; font-weight: 500; color: #fff; }
  .ps { font-size: 12.5px; color: rgba(255, 255, 255, .55); }
  .pc { flex: none; display: flex; align-items: center; gap: 8px; }
  .ins { display: flex; align-items: center; gap: 6px; font-size: 12.5px; color: rgba(255, 255, 255, .6); }
  .none { padding: 36px 0; display: flex; flex-direction: column; align-items: center; gap: 10px; }
  .nt { font-size: 14px; font-weight: 500; color: #fff; }
  .nd { font-size: 12.5px; color: rgba(255, 255, 255, .55); }
  .foot { position: relative; margin-top: 0; padding: 10px 0 0; display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px 16px; border-radius: 6px; }
  .foot::before { content: ""; position: absolute; left: 0; right: 0; top: 0; height: 1px; background: rgba(255, 255, 255, .06); pointer-events: none; }
  .foot.lit { animation: fflash 1.8s linear forwards; }
  @keyframes fflash { 0%, 39% { background: rgba(var(--accent-rgb, 255, 95, 210), .12); } 100% { background: transparent; } }
  .fgrp { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .fgrp.right { margin-left: auto; }
  .fbtn { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; color: rgba(255, 255, 255, .48); transition: color .12s; padding: 2px 0; }
  .fbtn:hover { color: rgba(255, 255, 255, .92); }
  .fsep { font-size: 12px; color: rgba(255, 255, 255, .2); user-select: none; }
  .bar { padding-top: 6px; display: flex; flex-wrap: wrap; align-items: center; gap: 10px; border-radius: 8px; }
  .bar.lit { animation: fflash 1.8s linear forwards; }
  .cat-msg { padding-top: 8px; font-size: 12px; color: rgba(255, 255, 255, .48); }
  .srch { flex: 1 1 200px; min-width: 200px; height: 34px; padding: 0 10px; display: flex; align-items: center; gap: 8px; border-radius: 8px; background: rgba(255, 255, 255, .06); }
  .srch input { flex: 1; min-width: 0; border: 0; outline: 0; background: none; font-size: 13px; color: #fff; }
  .empty { padding: 14px 0; font-size: 12.5px; color: rgba(255, 255, 255, .5); }
  .sk { padding-top: 14px; display: flex; flex-direction: column; gap: 6px; }
  .skr { display: flex; align-items: center; gap: 12px; padding: 6px 8px; }
  .skt { flex: 1; display: flex; flex-direction: column; gap: 8px; }
  .by { padding-top: 4px; display: flex; align-items: flex-start; gap: 8px; }
  .bi { padding-top: 1px; display: flex; }
  .bt { flex: 1; min-width: 0; font-size: 12.5px; line-height: 18px; color: rgba(255, 255, 255, .5); }
  .about { flex: none; font-size: 12.5px; color: rgba(255, 255, 255, .6); transition: color .12s; }
  .about:hover { color: #fff; }
  .wn { padding-top: 10px; font-size: 12.5px; color: var(--warn); }
</style>
