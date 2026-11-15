// Plugin state of Settings > Plugins (app/plugins.rs PluginsUi + the
// actions on Glyph) and of the rows plugins add to other pages
// (settings/plugin_ui.rs). Plugins load locally from GitHub through the
// backend, never through glyph-server.
import { backend } from "../api/backend";
import { app } from "../state.svelte";
import { setOverrides, t, tf } from "../i18n.svelte";
import { setPluginThemes, type Theme } from "../theme";
import { errText } from "./util";
import type { PluginCatalog, PluginInstalled, PluginListing, PluginUiState } from "./types";

export const THEME_PREFIX = "plugin:";
export const themeId = (id: string) => `${THEME_PREFIX}${id}`;

const emptyUi = (): PluginUiState => ({ hidden: [], sections: [], pages: [], strings: {} });

export const pl = $state({
  loaded: false,
  installed: [] as PluginInstalled[],
  catalog: null as PluginCatalog | null,
  loading: false,
  error: null as string | null,
  /** Plugin ids being installed or removed. */
  busy: {} as Record<string, boolean>,
  /** Last failed install per plugin id. */
  failed: {} as Record<string, string>,
  /** Plugin shown in the detail view. */
  detail: null as string | null,
  /** First install waiting for the user to accept the warning. */
  confirm: null as PluginListing | null,
  confirmSkip: false,
  ui: emptyUi(),
});

/** Reads the installed plugins and what they add; publishes themes and texts. */
export async function loadPlugins() {
  const st = await backend().call<{ installed: PluginInstalled[]; ui: PluginUiState } | undefined>("plugins_state");
  if (!st) return;
  pl.installed = st.installed;
  pl.ui = st.ui;
  pl.loaded = true;
  const themes: Theme[] = st.installed.filter((p) => p.theme).map((p) => ({ id: themeId(p.id), name: p.name, ...p.theme! }));
  setPluginThemes(themes);
  const texts: Record<string, Record<string, string>> = JSON.parse(JSON.stringify(st.ui.strings));
  for (const p of st.ui.pages) (texts.en ??= {})[`plugin-page.${p.id}`] = p.name;
  setOverrides(texts);
}

/** Loads the catalog (the backend caches it for an hour unless `force`). */
export async function refreshCatalog(force = false) {
  if (pl.loading) return;
  pl.loading = true;
  pl.error = null;
  try {
    const c = await backend().call<PluginCatalog | undefined>("plugins_catalog", { force });
    if (c) {
      pl.catalog = c;
      const missing = pl.detail && !findListing(pl.detail);
      if (app.config.plugins.auto_update) for (const l of silentUpdates()) installPlugin(l);
      if (missing && !force) { pl.loading = false; return refreshCatalog(true); }
    }
  } catch (e) { pl.error = errText(e); }
  pl.loading = false;
}

export function findListing(id: string): PluginListing | undefined {
  return pl.catalog?.checked.find((l) => l.id === id) ?? pl.catalog?.community.find((l) => l.id === id);
}

/** Installed plugins with a newer version in the catalog. */
export function pluginUpdates(): PluginListing[] {
  return pl.installed.flatMap((p) => (p.update ? [p.update] : []));
}

/** Permissions as the warning lists them: `code`, then each permission. */
export function allPermissions(m: PluginListing): string[] {
  return [...(m.code ? ["code"] : []), ...m.permissions];
}

function newPermissions(l: PluginListing, old: PluginListing): string[] {
  const had = new Set(allPermissions(old));
  return allPermissions(l).filter((p) => !had.has(p));
}

function silentUpdates(): PluginListing[] {
  return pluginUpdates().filter((l) => { const o = pl.installed.find((p) => p.id === l.id); return o && newPermissions(l, o).length === 0; });
}

/** Installs or updates `l`. A first install shows the warning first (plugins
 *  are made by other people, installed at your own risk), unless turned off;
 *  plugins with code always show it. Updates go straight through unless they
 *  ask for more permissions. */
export function installPlugin(l: PluginListing) {
  const old = pl.installed.find((p) => p.id === l.id);
  const fresh = !old;
  const asksMore = old ? newPermissions(l, old).length > 0 : l.code;
  if (asksMore || (fresh && !app.config.plugins.skip_install_warning)) {
    pl.detail = l.id;
    pl.confirm = l;
    pl.confirmSkip = false;
    return;
  }
  return installNow(l);
}

/** "Install anyway" on the warning. */
export function acceptWarning() {
  const l = pl.confirm;
  if (!l) return;
  pl.confirm = null;
  if (pl.confirmSkip && !l.code) app.setConfig((c) => { c.plugins.skip_install_warning = true; });
  return installNow(l);
}
export function cancelWarning() { pl.confirm = null; }

async function installNow(l: PluginListing) {
  if (pl.busy[l.id]) return;
  pl.busy[l.id] = true;
  delete pl.failed[l.id];
  const fresh = !pl.installed.some((p) => p.id === l.id);
  try {
    await backend().call("plugin_install", { id: l.id, repo: l.repo, path: l.path, rev: l.rev });
    await loadPlugins();
    const id = themeId(l.id);
    if (l.kind === "theme" && (fresh || app.config.visuals.theme === id)) app.setConfig((c) => { c.visuals.theme = id; });
    app.toast(tf(fresh ? "plugins.installed_toast" : "plugins.updated_toast", { name: l.name }));
    if (pl.installed.find((p) => p.id === l.id)?.missing_credentials) openOwnPage(l.id);
  } catch (e) { pl.failed[l.id] = errText(e); }
  delete pl.busy[l.id];
}

/** Asks for confirmation, then removes an installed plugin; its theme goes back to the default. */
export function uninstallPlugin(id: string) {
  if (pl.busy[id]) return;
  const name = pl.installed.find((p) => p.id === id)?.name ?? id;
  app.ask({
    title: tf("plugins.remove.confirm.title", { name }),
    body: t("plugins.remove.confirm.body"),
    ok: t("plugins.remove"),
    danger: true,
    run: () => void removePlugin(id, name),
  });
}

async function removePlugin(id: string, name: string) {
  if (pl.busy[id]) return;
  pl.busy[id] = true;
  if (app.config.visuals.theme === themeId(id)) app.setConfig((c) => { c.visuals.theme = "glyph"; });
  try {
    await backend().call("plugin_uninstall", { id });
    await loadPlugins();
    app.toast(tf("plugins.removed_toast", { name }));
  } catch (e) { app.toastInfo(errText(e)); }
  delete pl.busy[id];
}

export async function setEnabled(id: string, on: boolean) {
  const p = pl.installed.find((x) => x.id === id);
  if (p) p.enabled = on;
  await backend().call("plugin_enable", { id, on });
  await loadPlugins();
}

/** "Reload" for plugin makers: reads the plugins folder again. */
export async function reloadPlugins() {
  await backend().call("plugins_reload");
  await loadPlugins();
  app.toast(tf("plugins.reloaded", { n: pl.installed.length }));
}

export function showPlugin(id: string) {
  if (pl.confirm && pl.confirm.id !== id) pl.confirm = null;
  pl.detail = id;
}
export function closeDetail() { pl.detail = null; pl.confirm = null; }

/** Opens the own settings page of a plugin (set by settings.svelte.ts). */
let ownPageHook: ((id: string) => void) | null = null;
export function onOpenOwnPage(f: (id: string) => void) { ownPageHook = f; }
export function openOwnPage(id: string) { ownPageHook?.(id); }

// ---- rows plugins add to Glyph's pages ------------------------------------

/** Labels (translated) of the Glyph rows plugins hide. */
export function hiddenLabels(): Set<string> { return new Set(pl.ui.hidden.map((k) => t(k))); }

export async function setValue(plugin: string, item: string, value: unknown) {
  const it = pl.ui.sections.find((x) => x.plugin === plugin && x.items.some((i) => i.id === item))?.items.find((i) => i.id === item);
  if (it) it.value = value;
  await backend().call("plugin_set_value", { plugin, item, value });
  const st = await backend().call<{ installed: PluginInstalled[]; ui: PluginUiState } | undefined>("plugins_state");
  if (st) { pl.ui = st.ui; pl.installed = st.installed; }
}

export function pluginAction(plugin: string, item: string) {
  return backend().call("plugin_action", { plugin, item });
}
