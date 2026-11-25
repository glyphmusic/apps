<script lang="ts">
  // plugins.rs detail + warning: one plugin with preview, description,
  // actions and every manifest field. Opened by a click on a name (or
  // glyph://plugin/<id>). Before a first install (and an update that asks
  // for more) the warning replaces the actions: who made the plugin, that
  // Glyph cannot vouch for it, what its code may do, and that installing is
  // at the user's own risk (the plugin own-risk disclaimer always shows).
  import { app } from "../state.svelte";
  import { t, tf } from "../i18n.svelte";
  import { keyLabel, keyOf } from "../keys";
  import { css } from "../theme";
  import Button from "../components/Button.svelte";
  import Icon from "../components/Icon.svelte";
  import IconButton from "../components/IconButton.svelte";
  import Chip from "../components/Chip.svelte";
  import Skeleton from "../components/Skeleton.svelte";
  import { compact } from "../fmt";
  import Rows from "./Rows.svelte";
  import RowBase from "./RowBase.svelte";
  import SectionLabel from "./SectionLabel.svelte";
  import Surface from "./Surface.svelte";
  import Spin from "./Spin.svelte";
  import Tog from "./Tog.svelte";
  import PluginPreview from "./PluginPreview.svelte";
  import RiskNote from "./RiskNote.svelte";
  import { acceptWarning, allPermissions, cancelWarning, closeDetail, findListing, installPlugin, pl, refreshCatalog, setEnabled, uninstallPlugin, themeId } from "./plugins-state.svelte";
  import { openUrl } from "./util";
  import { setTab } from "./settings.svelte";
  let { id }: { id: string } = $props();

  const listing = $derived(findListing(id));
  const installed = $derived(pl.installed.find((p) => p.id === id));
  const m = $derived(listing ?? installed);
  const confirming = $derived(pl.confirm?.id === id);
  const busy = $derived(!!pl.busy[id]);
  const stars = $derived(listing?.stars ?? null);
  const checked = $derived(m?.checked ?? false);
  const curTheme = $derived(app.config.visuals.theme);
  const asks = $derived.by(() => {
    if (!m) return [] as string[];
    if (!installed) return allPermissions(m);
    const had = new Set(allPermissions(installed));
    return allPermissions(m).filter((p) => !had.has(p));
  });
  const permLabel = (p: string) => (p.startsWith("network:") ? tf("plugins.perm.network", { host: p.slice(8) }) : t(`plugins.perm.${p}`));
  const dash = (s: string | null | undefined) => (s && s.trim() ? s : "–");
  const source = $derived(m ? (m.path ? `${m.repo}/${m.path}` : m.repo) : "");
  const pinned = $derived(m && m.rev.length === 40 ? `${source} @ ${m.rev.slice(0, 7)}` : source);
  const canSkip = $derived(!!m && !m.code && !installed);
  const newer = $derived(!!listing && !!installed && installed.update?.id === id);
  const ownPage = $derived(pl.ui.pages.findIndex((p) => p.id === id));
  const ICON = "rgba(255,255,255,.7)";
  const sub = $derived(m ? [`v${m.version}`, ...(m.author.trim() ? [m.author] : []), ...(stars != null ? [`★ ${compact(stars)}`] : [])].join(" · ") : "");
</script>

{#snippet sw(hex: number)}<span class="sws"><span class="dot" style:background={css(hex)}></span><span class="mono v2">{css(hex)}</span></span>{/snippet}

<div class="col">
  {#if app.compact}
    <div class="top">
      <button class="back" onclick={closeDetail}><Icon name="chevron-left" size={14} color="rgba(255,255,255,.7)" /><span>{t("plugins.back")}</span></button>
    </div>
  {/if}

  {#if !m}
    {#if pl.loading || !pl.catalog}
      <div class="sk"><Skeleton w="100%" h={44} radius={6} id="pd1" /><Skeleton w="70%" h={44} radius={6} id="pd2" /><Skeleton w="85%" h={44} radius={6} id="pd3" /></div>
    {:else}
      <div class="nf">
        <Icon name="puzzle" size={28} color="rgba(255,255,255,.35)" />
        <div class="nft">{t("plugins.not_found")}</div>
        <div class="nfd">{tf("plugins.not_found.desc", { id })}</div>
        <Button onclick={() => refreshCatalog(true)}>{t("plugins.refresh")}</Button>
      </div>
    {/if}
  {:else}
    <div class="head">
      <PluginPreview theme={m.theme} size={64} />
      <div class="hx">
        <div class="hn">{m.name}</div>
        <div class="hs">{sub}</div>
        <div class="chips">
          <Chip kind="kind">{checked ? t("plugins.checked") : t("plugins.community")}</Chip>
          <Chip kind="kind">{m.kind === "theme" ? t("plugins.kind.theme") : t("plugins.kind.extension")}</Chip>
          {#if m.code}<Chip kind="kind">{t("plugins.kind.code")}</Chip>{/if}
        </div>
      </div>
      {#if !confirming}
        <div class="hact">
          {#if busy}
            <Spin />
          {:else}
            {#if listing}
              {#if listing.installable === "needs_newer_app"}<Chip kind="kind">{t("plugins.needs_update")}</Chip>
              {:else if listing.installable === "broken"}<Chip kind="kind">{t("plugins.broken")}</Chip>
              {:else if installed && installed.repo.toLowerCase() !== listing.repo.toLowerCase()}<Chip kind="kind">{t("plugins.id_taken")}</Chip>
              {:else if newer}<Button kind="primary" onclick={() => installPlugin(listing!)}>{tf("plugins.update_to", { v: listing!.version })}</Button>
              {:else if !installed}<Button kind="primary" onclick={() => installPlugin(listing!)}>{t("plugins.install")}</Button>{/if}
            {/if}
            {#if installed}
              {#if installed.theme}
                {#if curTheme === themeId(id)}<Chip kind="kind">{t("plugins.in_use")}</Chip>{:else}<Button kind="primary" onclick={() => app.setConfig((c) => { c.visuals.theme = themeId(id); })}>{t("plugins.use")}</Button>{/if}
              {/if}
              {#if ownPage >= 0}<IconButton name="sliders-horizontal" size={15} color={ICON} label={t("plugins.settings")} onclick={() => setTab("plugins", 2 + ownPage)} />{/if}
              {#if installed.kind !== "theme"}<Tog on={installed.enabled} onchange={(v) => setEnabled(id, v)} label={installed.enabled ? t("plugins.turn_off") : t("plugins.turn_on")} />{/if}
              <IconButton name="trash-2" size={15} color={ICON} label={t("plugins.remove")} onclick={() => uninstallPlugin(id)} />
            {/if}
            <IconButton name="github" size={15} color={ICON} label={t("plugins.repo")} onclick={() => openUrl(`https://github.com/${m.repo}`)} />
          {/if}
        </div>
      {/if}
    </div>
    {#if m.description.trim()}<div class="desc">{m.description}</div>{/if}
    {#if m.setup.length}<div class="req">{tf("plugins.setup.requires", { items: m.setup.join(", ") })}</div>{/if}
    {#if installed?.missing_credentials}<div class="wn">{t("plugins.setup.missing")}</div>{/if}

    {#if confirming && pl.confirm}
      <!-- Shown before a first install (and an update that asks for more). -->
      <div class="warn">
        <div class="wh"><Icon name="info" size={16} color="var(--warn)" /><span>{t("plugins.warn.title")}</span></div>
        <div class="wb">{tf("plugins.warn.body", { author: m.author.trim() ? m.author : t("plugins.warn.someone") })}</div>
        <div class="wb">{checked ? t("plugins.warn.listed") : t("plugins.warn.community")}</div>
        {#if asks.length}
          <div class="pm">
            <div class="pt">{installed ? t("plugins.warn.more") : t("plugins.warn.may")}</div>
            {#each asks as p (p)}<div class="pi"><Icon name="check" size={13} color="rgba(255,255,255,.6)" /><span>{permLabel(p)}</span></div>{/each}
          </div>
        {/if}
        {#if canSkip}
          <button class="skip" onclick={() => (pl.confirmSkip = !pl.confirmSkip)}><Tog on={pl.confirmSkip} label={t("plugins.warn.skip")} /><span>{t("plugins.warn.skip")}</span></button>
        {/if}
        <div class="wa"><Button kind="primary" onclick={acceptWarning}>{t("plugins.warn.accept")}</Button><Button onclick={cancelWarning}>{t("plugins.warn.cancel")}</Button></div>
      </div>
    {:else}
      <div class="rn-wrap"><RiskNote /></div>
    {/if}
    {#if pl.failed[id]}<div class="wn">{pl.failed[id]}</div>{/if}
    {#if installed?.stopped && installed.enabled}<div class="wn">{tf("plugins.stopped", { e: installed.stopped })}</div>{/if}

    <SectionLabel text={t("plugins.details")} />
    <Rows>
      <div class="srow"><RowBase label={t("plugins.version")}><span class="v">{m.version}</span></RowBase></div>
      {#if installed && installed.version !== m.version}<div class="srow"><RowBase label={t("plugins.installed_version")}><span class="v">{installed.version}</span></RowBase></div>{/if}
      <div class="srow"><RowBase label={t("plugins.author")}><span class="v">{dash(m.author)}</span></RowBase></div>
      <div class="srow"><RowBase label={t("plugins.license")}><span class="v">{dash(m.license)}</span></RowBase></div>
      <div class="srow"><RowBase label={t("plugins.min_app")}><span class="v">{dash(m.min_app)}</span></RowBase></div>
      <div class="srow"><RowBase label={t("plugins.repo_source")}><button class="repo-link mono v2" onclick={() => openUrl(`https://github.com/${m.repo}`)}><span>{pinned}</span><Icon name="external-link" size={12} color="rgba(255,255,255,.45)" /></button></RowBase></div>
      {#if stars != null}<div class="srow"><RowBase label={t("plugins.stars")} info={t("plugins.stars.desc")}><span class="v">{stars}</span></RowBase></div>{/if}
      {#if listing?.updated}<div class="srow"><RowBase label={t("plugins.updated")}><span class="v">{listing.updated.slice(0, 10)}</span></RowBase></div>{/if}
    </Rows>

    {#if allPermissions(m).length || m.sections.length || m.menus.length || m.strings || m.hide.length || m.look}
      <SectionLabel text={t("plugins.does")} />
      <Rows>
        {#each allPermissions(m) as p (p)}<div class="srow"><RowBase label={permLabel(p)} /></div>{/each}
        {#if m.sections.length}<div class="srow"><RowBase label={t("plugins.does.settings")}><span class="v">{m.sections.join(", ")}</span></RowBase></div>{/if}
        {#if m.menus.length}<div class="srow"><RowBase label={t("plugins.does.menus")}><span class="v">{m.menus.join(", ")}</span></RowBase></div>{/if}
        {#if m.strings}<div class="srow"><RowBase label={t("plugins.does.strings")}><span class="v">{m.strings}</span></RowBase></div>{/if}
        {#if m.hide.length}<div class="srow"><RowBase label={t("plugins.does.hide")}><span class="v">{m.hide.map((k) => t(k)).join(", ")}</span></RowBase></div>{/if}
        {#if m.look?.font}<div class="srow"><RowBase label={t("plugins.does.font")}><span class="v">{t(`plugins.font.${m.look.font}`)}</span></RowBase></div>{/if}
        {#if m.look?.mono}<div class="srow"><RowBase label={t("plugins.does.mono")}><span class="v">{t(`plugins.font.${m.look.mono}`)}</span></RowBase></div>{/if}
        {#if m.look?.app_icon}<div class="srow"><RowBase label={t("plugins.does.app_icon")} info={t("plugins.does.app_icon.desc")} /></div>{/if}
        {#if m.look?.radius != null}<div class="srow"><RowBase label={t("plugins.does.corners")}><span class="v">{Math.round(m.look.radius * 100)}%</span></RowBase></div>{/if}
        {#if m.look?.backgrounds}<div class="srow"><RowBase label={t("plugins.does.backgrounds")}><span class="v">{m.look.backgrounds}</span></RowBase></div>{/if}
        {#if m.look?.icons}<div class="srow"><RowBase label={t("plugins.does.icons")}><span class="v">{m.look.icons}</span></RowBase></div>{/if}
        {#if m.look?.css}<div class="srow"><RowBase label={t("plugins.does.css")} info={tf("plugins.does.css.desc", { key: keyLabel(keyOf("plugins")) })} /></div>{/if}
      </Rows>
    {/if}

    {#if installed?.local && installed.log.length}
      <SectionLabel text={t("plugins.log")} />
      <Surface><div class="mono log">{#each installed.log.slice(-14) as l}<div>{l}</div>{/each}</div></Surface>
    {/if}

    {#if m.theme}
      <SectionLabel text={t("plugins.colours")} />
      <Rows>
        <div class="srow"><RowBase label={t("plugins.accent")}>{@render sw(m.theme.accent)}</RowBase></div>
        {#if m.theme.glow}<div class="srow"><RowBase label={t("plugins.glow")} info={t("plugins.glow.desc")}><span class="gl">{#each m.theme.glow as g}{@render sw(g)}{/each}</span></RowBase></div>{/if}
        <div class="srow"><RowBase label={t("plugins.base")}>{@render sw(m.theme.base)}</RowBase></div>
      </Rows>
    {/if}
  {/if}
</div>

<style>
  .col { display: flex; flex-direction: column; }
  .top { display: flex; }
  .back { height: 28px; padding: 0 8px; margin-left: -8px; display: flex; align-items: center; gap: 6px; border-radius: 7px; font-size: 13px; color: rgba(255, 255, 255, .7); transition: background-color .12s; }
  .back:hover { background: rgba(255, 255, 255, .05); }
  .sk { padding-top: 14px; display: flex; flex-direction: column; gap: 10px; }
  .nf { padding: 36px 0; display: flex; flex-direction: column; align-items: center; gap: 10px; }
  .nft { font-size: 14px; font-weight: 500; color: #fff; }
  .nfd { font-size: 12.5px; color: rgba(255, 255, 255, .55); }
  .head { padding-top: 6px; display: flex; flex-wrap: wrap; align-items: center; gap: 14px 16px; }
  .hx { flex: 1 1 200px; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .hact { flex: none; display: flex; align-items: center; gap: 6px; margin-left: auto; }
  .hn { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 20px; font-weight: 600; color: #fff; }
  .hs { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12.5px; color: rgba(255, 255, 255, .55); }
  .chips { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .desc { padding-top: 14px; font-size: 13.5px; line-height: 20px; color: rgba(255, 255, 255, .72); }
  .req { padding-top: 10px; font-size: 12.5px; line-height: 18px; color: rgba(255, 255, 255, .62); }
  .wn { padding-top: 10px; font-size: 12.5px; color: var(--warn); }
  .rn-wrap { padding-top: 10px; }
  .repo-link { display: inline-flex; align-items: center; gap: 6px; text-align: right; transition: color .12s; }
  .repo-link:hover { color: #fff; }
  .warn { margin-top: 16px; padding: 18px; border-radius: 10px; background: rgba(255, 255, 255, .035); display: flex; flex-direction: column; gap: 10px; animation: rise-in .2s var(--ease); }
  .wh { display: flex; align-items: center; gap: 8px; font-size: 14px; font-weight: 600; color: #fff; }
  .wb { font-size: 13px; line-height: 19px; color: rgba(255, 255, 255, .72); }
  .pm { display: flex; flex-direction: column; gap: 6px; }
  .pt { font-size: 13px; font-weight: 500; color: #fff; }
  .pi { display: flex; align-items: center; gap: 8px; font-size: 13px; color: rgba(255, 255, 255, .75); }
  .skip { padding-top: 4px; display: flex; align-items: center; gap: 10px; font-size: 13px; color: rgba(255, 255, 255, .75); }
  .wa { padding-top: 6px; display: flex; flex-wrap: wrap; gap: 8px; }
  .v { font-size: 13px; color: rgba(255, 255, 255, .75); }
  .v2 { font-size: 12px; color: rgba(255, 255, 255, .75); overflow-wrap: anywhere; }
  .log { font-size: 11.5px; line-height: 17px; color: rgba(255, 255, 255, .7); }
  .sws { display: flex; align-items: center; gap: 6px; }
  .dot { width: 14px; height: 14px; border-radius: 50%; border: 1px solid rgba(255, 255, 255, .12); }
  .gl { display: flex; gap: 12px; }
</style>
