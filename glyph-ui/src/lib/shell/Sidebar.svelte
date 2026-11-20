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

  let { width = app.leftSidebarW }: { width?: number } = $props();
  const expanded = $derived(width >= 112);

  const nav: { view: View | "settings"; icon: string; label: string; motion: string }[] = [
    { view: "home", icon: "house", label: "nav.home", motion: "hop" },
    { view: "search", icon: "search", label: "nav.search", motion: "wiggle" },
    { view: "library", icon: "library", label: "nav.library", motion: "grow" },
    { view: "now_playing", icon: "disc-3", label: "nav.now_playing", motion: "spin" },
  ];
  const items = $derived(app.sidebarItems(5));
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
  {#each nav as n}
    <button class="it {n.motion}" class:on={active(n.view)} onclick={() => app.navigate(n.view)} use:tip={{ label: expanded ? "" : t(n.label), side: "right" }} aria-label={t(n.label)}>
      <span class="ico"><Icon name={n.icon} size={20} color={active(n.view) ? "var(--text)" : "var(--dim)"} /></span>
      {#if expanded}<span class="lab trunc">{t(n.label)}</span>{/if}
    </button>
  {/each}
  {#if items.length}
    <span class="rule"></span>
    {#each items as it}
      {@const title = it.kind === "list" ? app.listTitle(it.list) : it.name}
      <button class="it" class:on={activeItem(it)} onclick={() => openItem(it)} oncontextmenu={(e) => menu(e, it)}
        use:tip={{ label: expanded ? "" : title, side: "right" }}>
        {#if it.kind === "list"}<ListCover list={it.list} size={expanded ? 32 : 36} />
        {:else if app.artistPhoto(it.name) || !it.track}<ArtistAvatar name={it.name} size={expanded ? 32 : 36} iconSize={16} />
        {:else}<Cover track={it.track} size={expanded ? 32 : 36} round />{/if}
        {#if expanded}<span class="lab trunc">{title}</span>{/if}
      </button>
    {/each}
  {/if}
  <span class="flex1"></span>
  <button class="it turn" class:on={app.settingsOpen} onclick={() => app.navigate("settings")} use:tip={{ label: expanded ? "" : t("nav.settings"), side: "right" }} aria-label={t("nav.settings")}>
    <span class="ico"><Icon name="settings" size={20} color={app.settingsOpen ? "var(--text)" : "var(--dim)"} /></span>
    {#if expanded}<span class="lab trunc">{t("nav.settings")}</span>{/if}
  </button>
</nav>

<style>
  .sb { flex: none; width: var(--sidebar-w); height: 100%; display: flex; flex-direction: column; align-items: center; padding: 12px 0; gap: 6px; overflow: hidden; }
  .sb.wide { align-items: stretch; padding: 12px 8px; }
  .it { flex: none; width: 44px; height: 44px; display: grid; place-items: center; border-radius: 10px; transition: background-color .12s ease-out; }
  .sb.wide .it { width: 100%; height: 42px; padding: 0 10px; display: flex; align-items: center; gap: 11px; justify-content: flex-start; }
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
  @keyframes hop { 50% { transform: translateY(-3px); } }
  @keyframes wiggle { 15% { transform: rotate(14deg); } 30% { transform: rotate(-10deg); } 45% { transform: rotate(6deg); } 60% { transform: rotate(-3deg); } 100% { transform: none; } }
  @keyframes grow { 50% { transform: scale(1.16); } }
  .rule { width: 24px; height: 1px; margin: 4px 0; background: rgba(255, 255, 255, .08); }
  .sb.wide .rule { width: calc(100% - 12px); align-self: center; }
  .art { flex: none; width: 36px; height: 36px; border-radius: 50%; background: var(--card); display: grid; place-items: center; }
  .art.sm { width: 32px; height: 32px; }
</style>
