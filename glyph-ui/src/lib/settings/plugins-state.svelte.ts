// Plugin state of Settings > Plugins (app/plugins.rs PluginsUi + the
// actions on Glyph) and of the rows plugins add to other pages
// (settings/plugin_ui.rs). Plugins load locally from GitHub through the
// backend, never through glyph-server.
import { backend } from "../api/backend";
import { app } from "../state.svelte";
import { setOverrides, t, tf } from "../i18n.svelte";
import { setPluginThemes, type Theme } from "../theme";
import { loadBackgrounds } from "../backgrounds.svelte";
import { errText } from "./util";
import { preferOrder, sortLikeGlyph, type Named } from "./plugin-order";
import type { PluginCatalog, PluginInstalled, PluginListing, PluginUiState, PluginVersion } from "./types";

export const THEME_PREFIX = "plugin:";
export const themeId = (id: string) => `${THEME_PREFIX}${id}`;

const emptyUi = (): PluginUiState => ({ hidden: [], sections: [], pages: [], strings: {}, looks: [] });

export const pl = $state({
  loaded: false,
  installed: [] as PluginInstalled[],
  /** Installed but paused: this Glyph's version does not fit them. */
  unfit: [] as PluginInstalled[],
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
  /** The waiting install is an earlier version picked by hand (kept afterwards). */
  confirmHold: false,
  ui: emptyUi(),
  /** The home screen can get a shortcut with a plugin's app icon (Android). */
  canPinIcon: false,
});

/** Reads the installed plugins and what they add; publishes themes and texts. */
export async function loadPlugins() {
  const st = await backend().call<{ installed: PluginInstalled[]; unfit?: PluginInstalled[]; ui: PluginUiState; can_pin_icon?: boolean } | undefined>("plugins_state");
  if (!st) return;
  pl.canPinIcon = !!st.can_pin_icon;
  pl.installed = st.installed;
  pl.unfit = st.unfit ?? [];
  pl.ui = st.ui;
  pl.loaded = true;
  const themes: Theme[] = st.installed.filter((p) => p.theme).map((p) => ({ id: themeId(p.id), name: p.name, ...p.theme! }));
  setPluginThemes(themes);
  const texts: Record<string, Record<string, string>> = JSON.parse(JSON.stringify(st.ui.strings));
  for (const p of st.ui.pages) (texts.en ??= {})[`plugin-page.${p.id}`] = p.name;
  setOverrides(texts);
  // Their pictures join the background picker (App.svelte applies the looks).
  void loadBackgrounds();
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

// ---- what a plugin takes over, and conflicts between plugins -------------

/** One thing a plugin changes that only one plugin can: `key` to compare, `label` (i18n key) to show. */
export interface Slot { key: string; label: string }

/** What `m` changes that only one plugin can change at a time. */
export function pluginSlots(m: PluginListing): Slot[] {
  const l = m.look, out: Slot[] = [];
  if (l?.font) out.push({ key: "font", label: "plugins.part.font" });
  if (l?.mono) out.push({ key: "mono", label: "plugins.part.mono" });
  if (l?.app_icon) out.push({ key: "app_icon", label: "plugins.part.app_icon" });
  if (l?.radius != null) out.push({ key: "corners", label: "plugins.part.corners" });
  if (l?.splash) out.push({ key: "splash", label: "plugins.part.splash" });
  if (l?.sidebar_position) out.push({ key: "sidebar_position", label: "plugins.part.sidebar_position" });
  for (const a of ["home", "sidebar", "tabs"] as const) if (l?.[a]) out.push({ key: a, label: "plugins.part.layout" });
  for (const n of l?.icon_names ?? []) out.push({ key: `icon:${n}`, label: "plugins.part.icons" });
  for (const k of m.replaces ?? []) out.push({ key: `text:${k}`, label: "plugins.part.texts" });
  return out;
}

/** Your own settings `m` replaces while it is on (labels, i18n keys). */
export function pluginOverrides(m: PluginListing): string[] {
  const l = m.look, out: string[] = [];
  if (l?.css || (m.theme && m.kind !== "theme")) out.push("plugins.part.background");
  for (const s of pluginSlots(m)) if (!out.includes(s.label)) out.push(s.label);
  return out;
}

export interface Conflict { other: PluginInstalled; parts: string[]; wins: boolean }

/** Whether `a` wins over installed `b` (when both change the same thing). */
function beats(a: Named, b: Named): boolean {
  const list: Named[] = pl.installed.some((x) => x.id === a.id) ? pl.installed : sortLikeGlyph([...pl.installed, a], app.config.plugins.order ?? []);
  return list.findIndex((x) => x.id === a.id) < list.findIndex((x) => x.id === b.id);
}

/** Resolves a conflict: `id` wins over `over` from now on. */
export async function preferPlugin(id: string, over: string) {
  const order = preferOrder(pl.installed.map((p) => p.id), id, over);
  await app.setConfig((c) => { c.plugins.order = order; });
  await backend().call("plugin_prefer", { id, over });
  await loadPlugins();
  const name = pl.installed.find((p) => p.id === id)?.name ?? id;
  app.toast(tf("plugins.conflict.resolved", { name }));
}

/** Other plugins in use that change the same things as `m`. Like Glyph
 *  applies them, the first plugin wins: the user's pick, else by name. */
export function pluginConflicts(m: PluginListing): Conflict[] {
  const mine = pluginSlots(m);
  if (!mine.length) return [];
  const keys = new Set(mine.map((s) => s.key));
  const theme = app.config.visuals.theme;
  return pl.installed
    .filter((o) => o.id !== m.id && o.enabled && (o.kind !== "theme" || theme === themeId(o.id)))
    .flatMap((o) => {
      const parts = [...new Set(pluginSlots(o).filter((s) => keys.has(s.key)).map((s) => s.label))];
      return parts.length ? [{ other: o, parts, wins: beats(m, o) }] : [];
    });
}

/** An installed plugin, running or paused. */
export function findInstalled(id: string): PluginInstalled | undefined {
  return pl.installed.find((p) => p.id === id) ?? pl.unfit.find((p) => p.id === id);
}

/** `a` is a higher version than `b` (numbers only, like 1.2.0). */
export function newer(a: string, b: string): boolean {
  const pa = a.split(".").map((x) => parseInt(x, 10) || 0), pb = b.split(".").map((x) => parseInt(x, 10) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) if ((pa[i] ?? 0) !== (pb[i] ?? 0)) return (pa[i] ?? 0) > (pb[i] ?? 0);
  return false;
}

/** Kept at its version: no automatic updates, none from "Update all". */
export function isHeld(id: string): boolean { return (app.config.plugins.held ?? []).includes(id); }
export function setHeld(id: string, on: boolean) {
  if (isHeld(id) === on) return;
  app.setConfig((c) => { c.plugins.held = (c.plugins.held ?? []).filter((x) => x !== id).concat(on ? [id] : []); });
}

/** Installed plugins (paused ones too) with an update to take, except kept ones. */
export function pluginUpdates(): PluginListing[] {
  return [...pl.installed, ...pl.unfit].flatMap((p) => (p.update && !isHeld(p.id) ? [p.update] : []));
}

/** `base` (a listing or an installed plugin) as its earlier version `v`. */
export function atVersion(base: PluginListing, v: PluginVersion): PluginListing {
  return { ...base, version: v.version, rev: v.rev, min_app: v.min_app, max_app: v.max_app, changes: v.changes, code: v.code, permissions: v.permissions, installable: v.fits ? "ok" : "needs_newer_app", versions: [], fitting: null };
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
  return pluginUpdates().filter((l) => { const o = findInstalled(l.id); return o && newPermissions(l, o).length === 0; });
}

/** Installs or updates `l`. A first install shows the warning first (plugins
 *  are made by other people, installed at your own risk), unless turned off;
 *  plugins with code always show it. Updates go straight through unless they
 *  ask for more permissions. */
export function installPlugin(l: PluginListing, hold = false) {
  const old = findInstalled(l.id);
  const fresh = !old;
  const asksMore = old ? newPermissions(l, old).length > 0 : l.code;
  if (asksMore || (fresh && !app.config.plugins.skip_install_warning)) {
    pl.detail = l.id;
    pl.confirm = l;
    pl.confirmSkip = false;
    pl.confirmHold = hold;
    return;
  }
  return installNow(l, hold);
}

/** "Install anyway" on the warning. */
export function acceptWarning() {
  const l = pl.confirm;
  if (!l) return;
  pl.confirm = null;
  if (pl.confirmSkip && !l.code) app.setConfig((c) => { c.plugins.skip_install_warning = true; });
  return installNow(l, pl.confirmHold);
}
export function cancelWarning() { pl.confirm = null; }

/** Installs `l`; `hold` keeps it at that version afterwards (an earlier
 *  version picked by hand), otherwise a kept plugin takes updates again. */
async function installNow(l: PluginListing, hold = false) {
  if (pl.busy[l.id]) return;
  pl.busy[l.id] = true;
  delete pl.failed[l.id];
  const fresh = !findInstalled(l.id);
  try {
    await backend().call("plugin_install", { id: l.id, repo: l.repo, path: l.path, rev: l.rev });
    setHeld(l.id, hold);
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
  const name = findInstalled(id)?.name ?? id;
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
  if (app.config.visuals.bg_image?.startsWith(`plugin:${id}/`)) app.setConfig((c) => { c.visuals.bg_image = ""; });
  try {
    await backend().call("plugin_uninstall", { id });
    setHeld(id, false);
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

/** Asks Android to put a Glyph shortcut with the plugin's app icon on the home screen. */
export async function pinPluginIcon(id: string) {
  try {
    await backend().call("plugin_pin_icon", { id });
    app.toast(t("plugins.pin_icon.asked"));
  } catch (e) { app.toastInfo(errText(e)); }
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
