<script lang="ts">
  // compact.rs tab_bar: Home / Search / Library, h 50, 22 px icons, 10 px
  // labels; active in text colour (medium), the rest faint.
  import Icon from "../components/Icon.svelte";
  import { app, type View } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { press } from "../ui.svelte";
  const tabs: { view: View; icon: string; label: string }[] = [
    { view: "home", icon: "house", label: "nav.home" },
    { view: "search", icon: "search", label: "nav.search" },
    { view: "library", icon: "library", label: "nav.library" },
  ];
  function go(v: View) { app.settingsOpen = false; if (app.view === v) { document.getElementById("main-scroll")?.scrollTo({ top: 0, behavior: "smooth" }); return; } app.navigate(v); }
</script>

<nav class="tabs" data-no-longpress>
  {#each tabs as tb}
    {@const on = app.view === tb.view && !app.settingsOpen}
    <button class="tab" class:on use:press onclick={() => go(tb.view)}>
      <span class="ic"><Icon name={tb.icon} size={22} /></span>
      <span>{t(tb.label)}</span>
    </button>
  {/each}
</nav>

<style>
  .tabs { display: flex; align-items: center; height: 46px; }
  .tab { flex: 1; height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 2px; color: rgba(242, 242, 242, .56); font-size: 10px; line-height: 1.1; transition: color .14s var(--ease), transform .16s var(--ease); }
  .tab.on { color: rgba(242, 242, 242, 1); font-weight: 500; }
  .tab:active { transform: scale(.92); }
  .tab .ic { display: grid; place-items: center; }
  .tab:global(.pressing) .ic { animation: tab-bounce .3s cubic-bezier(.34, 1.56, .64, 1); }
  @keyframes tab-bounce { 0% { transform: scale(.82); } 50% { transform: scale(1.16); } 100% { transform: scale(1); } }
</style>
