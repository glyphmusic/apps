// URL fragment persistence for Settings (`#settings`, `#settings/<page>`,
// `#settings/<page>/<subtab>`, `#settings/plugins/browse/<id>`). Keeps the
// active settings location in `location.hash` via `history.replaceState` so a
// reload re-opens Settings at the same page, sub-tab, or plugin detail.
import { untrack } from "svelte";
import { app, type SettingsPage } from "../state.svelte";
import { phone } from "../compact/cstate.svelte";
import { PAGES, pageShown, subtabs } from "./pages";
import { closeDetail, pl, showPlugin } from "./plugins-state.svelte";
import { sui } from "./settings.svelte";

const PREFIX = "#settings";

const SUB_SLUGS: Record<string, string> = {
  "sp.sub.general": "general",
  "sp.sub.content": "content",
  "settings.sources": "sources",
  "sp.sub.player": "player",
  "sp.sub.overview": "overview",
  "sp.sub.profile": "profile",
  "sp.sub.devices": "devices",
  "account.security": "security",
  "sp.sub.connection": "connection",
  "settings.keys": "keys",
  "privacy.tab": "privacy",
  "sp.discord.section": "discord",
  "sp.mcp.section": "mcp",
  "plugins.tab.installed": "installed",
  "plugins.tab.browse": "browse",
  "sp.sub.version": "version",
  "sp.support": "support",
  "sp.sub.shortcuts": "shortcuts",
};

const RESERVED_PLUGIN_SLUGS = new Set(["installed", "browse", "page"]);
let pendingPluginPage = $state<string | null>(null);

const enc = encodeURIComponent;
const dec = (s: string): string => {
  try { return decodeURIComponent(s); } catch { return s; }
};

export function isSettingsHash(hash: string): boolean {
  return hash === PREFIX || hash.startsWith(`${PREFIX}/`);
}

function pluginOwnHash(id: string): string {
  return RESERVED_PLUGIN_SLUGS.has(id)
    ? `${PREFIX}/plugins/page/${enc(id)}`
    : `${PREFIX}/plugins/${enc(id)}`;
}

function setSub(page: SettingsPage, sub: number) {
  app.settingsPage = page;
  sui.lastPage = page;
  sui.sub = sub;
  sui.collapsed[page] = false;
  phone.settingsInPage = true;
  phone.settingsSub = sub;
}

/** URL fragment for the current Settings UI state, or `""` when closed. */
export function settingsHash(): string {
  if (!app.settingsOpen) return "";
  if (app.compact && !phone.settingsInPage) return PREFIX;
  const page = app.settingsPage;
  const sub = app.compact ? phone.settingsSub : sui.sub;

  if (page === "plugins") {
    if (pendingPluginPage && !pl.loaded && sub === 0 && !pl.detail) {
      return pluginOwnHash(pendingPluginPage);
    }
    if (sub < 2 && pl.detail) {
      return `${PREFIX}/plugins/${sub === 1 ? "browse" : "installed"}/${enc(pl.detail)}`;
    }
    if (sub >= 2) {
      const ownId = pl.ui.pages[sub - 2]?.id;
      if (ownId) return pluginOwnHash(ownId);
    }
    return sub === 1 ? `${PREFIX}/plugins/browse` : `${PREFIX}/plugins`;
  }

  const key = subtabs(page)[sub];
  const slug = key ? SUB_SLUGS[key] : undefined;
  return sub > 0 && slug ? `${PREFIX}/${page}/${slug}` : `${PREFIX}/${page}`;
}

/** Restores Settings open, page, sub-tab, and plugin detail state from `hash`. */
export function applySettingsHash(hash: string) {
  pendingPluginPage = null;
  if (!isSettingsHash(hash)) {
    app.settingsOpen = false;
    return;
  }

  const raw = hash.slice(PREFIX.length).replace(/^\/+/, "");
  const parts = raw ? raw.split("/").filter(Boolean).map(dec) : [];

  if (parts.length === 0) {
    app.settingsOpen = true;
    app.settingsPage = "overview";
    sui.lastPage = "overview";
    sui.sub = 0;
    sui.collapsed.overview = false;
    phone.settingsInPage = false;
    phone.settingsSub = 0;
    closeDetail();
    return;
  }

  const [rawPage, subTok, extra] = parts;
  const candidate = rawPage as SettingsPage;
  const page: SettingsPage = PAGES.includes(candidate) && pageShown(candidate) ? candidate : "overview";

  app.settingsOpen = true;

  if (page !== "plugins") {
    closeDetail();
    const tabs = subtabs(page);
    let sub = 0;
    if (subTok) {
      const idx = tabs.findIndex((k) => SUB_SLUGS[k] === subTok);
      if (idx >= 0) sub = idx;
      else if (/^\d+$/.test(subTok)) {
        const n = Number(subTok);
        if (n >= 0 && n < tabs.length) sub = n;
      }
    }
    setSub(page, sub);
    return;
  }

  if (!subTok || subTok === "installed") {
    setSub("plugins", 0);
    if (subTok === "installed" && extra) showPlugin(extra);
    else closeDetail();
    return;
  }
  if (subTok === "browse") {
    setSub("plugins", 1);
    if (extra) showPlugin(extra);
    else closeDetail();
    return;
  }

  const ownId = subTok === "page" ? extra : subTok;
  closeDetail();
  if (!ownId) {
    setSub("plugins", 0);
    return;
  }
  if (pl.loaded) {
    const idx = pl.ui.pages.findIndex((p) => p.id === ownId);
    setSub("plugins", idx >= 0 ? 2 + idx : 0);
  } else {
    pendingPluginPage = ownId;
    setSub("plugins", 0);
  }
}

/** Starts syncing Settings state with `location.hash`. Call once after `app.init()`. */
export function installSettingsHash(): () => void {
  applySettingsHash(location.hash);

  const onHash = () => applySettingsHash(location.hash);
  addEventListener("hashchange", onHash);

  const stop = $effect.root(() => {
    // Wait for `loadPlugins()` before resolving a plugin's own page on reload.
    $effect(() => {
      if (!pendingPluginPage) return;
      const sub = app.compact ? phone.settingsSub : sui.sub;
      if (!app.settingsOpen || app.settingsPage !== "plugins" || (app.compact && !phone.settingsInPage) || sub !== 0 || pl.detail) {
        pendingPluginPage = null;
        return;
      }
      if (!pl.loaded) return;
      const target = pendingPluginPage;
      pendingPluginPage = null;
      const idx = pl.ui.pages.findIndex((p) => p.id === target);
      if (idx >= 0) {
        const nextSub = 2 + idx;
        sui.sub = nextSub;
        phone.settingsSub = nextSub;
      }
    });

    $effect(() => {
      const next = settingsHash();
      untrack(() => {
        const cur = location.hash;
        if (next === cur) return;
        if (!next && !isSettingsHash(cur)) return;
        history.replaceState(history.state, "", location.pathname + location.search + next);
      });
    });
  });

  return () => {
    stop();
    removeEventListener("hashchange", onHash);
  };
}
