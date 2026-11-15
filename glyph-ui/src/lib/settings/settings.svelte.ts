// UI state of the settings that lives only on the Svelte side (the Rust
// `Glyph` fields settings_sub, settings_collapsed, settings_search and the
// FLASH of settings.rs). The page itself is `app.settingsPage`.
import { app, type SettingsPage } from "../state.svelte";
import { closeDetail, onOpenOwnPage, pl, refreshCatalog, showPlugin } from "./plugins-state.svelte";

export const sui = $state({
  /** Sub-tab of the open page (`app.settings_sub`). */
  sub: 0,
  /** Nav search text (`app.settings_search`). */
  query: "",
  /** Pages whose sub-tabs the user folded away (`settings_collapsed`). */
  collapsed: {} as Record<string, boolean>,
  /** Page last shown (the modal resets the sub-tab when the page changes from elsewhere). */
  lastPage: "overview" as SettingsPage,
  /** Option picked in the search: its row lights up and scrolls into view. */
  flash: null as { label: string; at: number } | null,
});

export const FLASH_S = 1.8;

/** Highlights the row labelled `label` on the next page shown (settings.rs flash_option). */
export function flashOption(label: string) {
  sui.flash = { label, at: performance.now() };
  setTimeout(() => { if (sui.flash?.label === label) sui.flash = null; }, FLASH_S * 1000 + 100);
}

/** Opens a page at one of its sub-tabs (app/settings.rs set_settings_tab). */
export function setTab(page: SettingsPage, sub = 0) {
  // Any tab click leaves a plugin's detail view (and its warning).
  closeDetail();
  app.settingsPage = page;
  sui.lastPage = page;
  sui.sub = sub;
  sui.collapsed[page] = false;
}

/** Nav click on a page: open it, or fold / unfold its sub-tabs when it is open. */
export function clickPage(page: SettingsPage, hasSubs: boolean) {
  if (app.settingsPage !== page) setTab(page, 0);
  else if (hasSubs) sui.collapsed[page] = !sui.collapsed[page];
}

/** Opens Settings > Plugins on one plugin, ready to install (app/plugins.rs open_plugin). */
export function openPlugin(id: string) {
  app.settingsOpen = true;
  setTab("plugins", 1);
  showPlugin(id);
  refreshCatalog(false);
}

// A plugin without its credentials opens its own page right after install.
onOpenOwnPage((id) => {
  const i = pl.ui.pages.findIndex((p) => p.id === id);
  if (i >= 0) { app.settingsOpen = true; setTab("plugins", 2 + i); }
});
