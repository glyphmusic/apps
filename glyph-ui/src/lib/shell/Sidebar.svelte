<script lang="ts">
  // chrome.rs sidebar: 64 px column, 44 px icon tiles (radius 10, active =
  // white 10 %), each icon with its own hover motion (house hops, magnifier
  // wiggles, library pops, disc spins, gear turns); under a short rule the
  // recently used lists / artists as 36 px covers; settings at the bottom.
  import Icon from "../components/Icon.svelte";
  import Cover from "../components/Cover.svelte";
  import ListCover from "../components/ListCover.svelte";
  import ArtistAvatar from "../components/ArtistAvatar.svelte";
  import { app, type SideItem, type View } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { tip } from "../ui.svelte";
  import { sameList } from "../api/types";
  import { admin } from "../admin/admin.svelte";
  import { mg, migrateRunning, serviceOf } from "../settings/migrate-state.svelte";
  import { arrange, layout } from "../layout.svelte";

  const trRun = $derived(migrateRunning());
  const trTip = $derived(
    trRun && mg.service
      ? `${serviceOf(mg.service).name} · ${trRun.index + 1}/${Math.max(1, trRun.lists)}${trRun.total > 0 ? ` · ${Math.min(100, Math.round((trRun.done * 100) / trRun.total))}%` : ""}`
      : "",
  );

  let { width = app.leftSidebarW }: { width?: number } = $props();
  const expanded = $derived(width >= 112);

  type NavPart = "home" | "search" | "library" | "now_playing";
  const nav: Record<NavPart, { view: View; icon: string; label: string; motion: string }> = {
    home: { view: "home", icon: "house", label: "nav.home", motion: "hop" },
    search: { view: "search", icon: "search", label: "nav.search", motion: "wiggle" },
    library: { view: "library", icon: "library", label: "nav.library", motion: "grow" },
    now_playing: { view: "now_playing", icon: "disc-3", label: "nav.now_playing", motion: "spin" },
  };
  // Plugin looks can put these in order or hide them (layout.svelte.ts);
  // "lists" is the block of recent lists and artists. Settings always stays.
  const PARTS = ["home", "search", "library", "now_playing", "lists"] as const;
  const parts = $derived(arrange(PARTS, layout.areas.sidebar));
  // Tooltips point away from the window edge the sidebar sits at.
  const tipSide = $derived(layout.sidebarRight ? "left" : "right");
  const items = $derived(app.sidebarItems(5));
  // The server dashboard shows up only for accounts the server made admins.
  $effect(() => admin.check(app.account?.handle, app.backendState.kind === "ready"));
  const active = (v: View | "settings") => (v === "settings" ? app.settingsOpen : app.view === v && !app.settingsOpen);
  function activeItem(it: SideItem) {
    if (it.kind === "list") return app.view === "list" && sameList(app.openList, it.list);
    return app.view === "artist" && app.openArtist?.toLowerCase() === it.name.toLowerCase();
  }
  function openItem(it: SideItem) { if (it.kind === "list") app.openListView(it.list); else app.openArtistPage(it.name); }
  function menu(e: MouseEvent, it: SideItem) {
    e.preventDefault();
    const id = it.kind === "list" ? (it.list.kind === "playlist" ? `playlist:${it.list.id}` : it.list.kind) : `artist:${it.name.toLowerCase()}`;
    app.openCtx({ x: e.clientX, y: e.clientY, track: it.kind === "artist" ? it.track : app.listTracks(it.list)[0] ?? null, list: it.kind === "list" ? it.list : null, artist: it.kind === "artist" ? it.name : null, sidebar: id });
  }
</script>

<nav class="sb" class:wide={expanded} style:width="{width}px" data-part="sidebar">
  {#each parts as part, pi (part)}
    {#if part !== "lists"}
      {@const n = nav[part]}
      <button class="it {n.motion}" class:on={active(n.view)} onclick={() => app.navigate(n.view)} use:tip={{ label: expanded ? "" : t(n.label), side: tipSide }} aria-label={t(n.label)} data-item={part}>
        <span class="ico"><Icon name={n.icon} size={20} color={active(n.view) ? "var(--text)" : "var(--dim)"} /></span>
        {#if expanded}<span class="lab trunc">{t(n.label)}</span>{/if}
      </button>
    {:else if items.length}
    {#if pi > 0}<span class="rule"></span>{/if}
    {#each items as it}
      {@const title = it.kind === "list" ? app.listTitle(it.list) : it.name}
      <button class="it" class:on={activeItem(it)} onclick={() => openItem(it)} oncontextmenu={(e) => menu(e, it)}
        use:tip={{ label: expanded ? "" : title, side: tipSide }}>
        {#if it.kind === "list"}<ListCover list={it.list} size={expanded ? 32 : 36} />
        {:else if app.artistPhoto(it.name) || !it.track}<ArtistAvatar name={it.name} size={expanded ? 32 : 36} iconSize={16} />
        {:else}<Cover track={it.track} size={expanded ? 32 : 36} round />{/if}
        {#if expanded}<span class="lab trunc">{title}</span>{/if}
      </button>
    {/each}
    {#if pi < parts.length - 1}<span class="rule"></span>{/if}
    {/if}
  {/each}
  <span class="flex1"></span>
  {#if admin.on}
    <button class="it pulse" class:on={active("server")} onclick={() => { admin.sub = ""; app.navigate("server"); }} use:tip={{ label: expanded ? "" : t("admin.title"), side: tipSide }} aria-label={t("admin.title")}>
      <span class="ico"><Icon name="server" size={20} color={active("server") ? "var(--text)" : "var(--dim)"} /></span>
      {#if expanded}<span class="lab trunc">{t("admin.title")}</span>{/if}
    </button>
  {/if}
  {#if trRun && !app.settingsOpen}
    <button class="it trbtn" onclick={() => app.openSettings("migrate")} use:tip={{ label: expanded ? "" : trTip, side: tipSide }} aria-label={t("sp.migrate")}>
      <span class="ico trico"><Icon name="arrow-right-left" size={18} color="var(--accent)" /><span class="trbadge"></span></span>
      {#if expanded}<span class="lab trunc trlab">{trTip}</span>{/if}
    </button>
  {/if}
  <button class="it turn" class:on={app.settingsOpen} onclick={() => app.navigate("settings")} use:tip={{ label: expanded ? "" : t("nav.settings"), side: tipSide }} aria-label={t("nav.settings")}>
    <span class="ico"><Icon name="settings" size={20} color={app.settingsOpen ? "var(--text)" : "var(--dim)"} /></span>
    {#if expanded}<span class="lab trunc">{t("nav.settings")}</span>{/if}
  </button>
</nav>

<style>
  .sb { flex: none; width: var(--sidebar-w); height: 100%; display: flex; flex-direction: column; align-items: center; padding: 12px 0; gap: 6px; overflow: hidden; }
  .sb.wide { align-items: stretch; padding: 12px 8px; }
  .it { flex: none; width: 44px; height: 44px; display: grid; place-items: center; border-radius: 10px; transition: background-color .12s ease-out; }
  .sb.wide .it { width: 100%; height: 42px; padding: 0 10px 0 5px; display: flex; align-items: center; gap: 10px; justify-content: flex-start; }
  .sb.wide .ico { width: 32px; height: 32px; }
  .it:hover { background: var(--hover); }
  .it.on { background: var(--active); }
  .ico { flex: none; display: grid; place-items: center; transition: transform .3s cubic-bezier(.33, 1, .68, 1); }
  .lab { flex: 1; min-width: 0; font-size: var(--sm); font-weight: 500; color: var(--dim); text-align: left; transition: color .12s ease-out; }
  .it:hover .lab, .it.on .lab { color: var(--text); }
  .hop:hover .ico { animation: hop .35s ease-out; }
  .wiggle:hover .ico { animation: wiggle .6s ease-out; }
  .grow:hover .ico { animation: grow .3s ease-out; }
  .spin:hover .ico { animation: spin 1.4s linear infinite; }
  .turn:hover .ico { transform: rotate(90deg); transition-duration: .5s; }
  .pulse:hover .ico { animation: grow .3s ease-out; }
  @keyframes hop { 50% { transform: translateY(-3px); } }
  @keyframes wiggle { 15% { transform: rotate(14deg); } 30% { transform: rotate(-10deg); } 45% { transform: rotate(6deg); } 60% { transform: rotate(-3deg); } 100% { transform: none; } }
  @keyframes grow { 50% { transform: scale(1.16); } }
  .rule { width: 24px; height: 1px; margin: 4px 0; background: rgba(255, 255, 255, .08); }
  .sb.wide .rule { width: calc(100% - 12px); align-self: center; }
  .art { flex: none; width: 36px; height: 36px; border-radius: 50%; background: var(--card); display: grid; place-items: center; }
  .art.sm { width: 32px; height: 32px; }
  .trbtn { background: rgba(var(--accent-rgb, 255, 95, 210), .09); }
  .trbtn:hover { background: rgba(var(--accent-rgb, 255, 95, 210), .16); }
  .trico { position: relative; }
  .trbadge { position: absolute; top: 0; right: 0; width: 7px; height: 7px; border-radius: 50%; background: var(--accent); animation: sbtrpulse 1.5s ease-in-out infinite; }
  .trlab { color: var(--text); }
  @keyframes sbtrpulse { 0%, 100% { opacity: 1; } 50% { opacity: .4; } }
</style>
