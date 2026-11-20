// "Advanced" settings: pages, sub-tabs and rows that stay hidden until the
// user turns on the Advanced switch, so new users see a short, calm list.
// The switch lives on this device only (localStorage), off by default.
import type { SettingsPage } from "../state.svelte";
import { t } from "../i18n.svelte";

const KEY = "glyph:settings-advanced";

function load(): boolean {
  try { return typeof localStorage !== "undefined" && localStorage.getItem(KEY) === "1"; } catch { return false; }
}

export const adv = $state({ on: load() });

export function setAdvanced(on: boolean) {
  adv.on = on;
  try { localStorage.setItem(KEY, on ? "1" : "0"); } catch { /* memory only */ }
}

/** Whole pages that only show with Advanced on. */
export const ADVANCED_PAGES = new Set<SettingsPage>(["server", "integrations", "transfer"]);

/** Sub-tabs (label keys) that only show with Advanced on. */
export const ADVANCED_SUBTABS = new Set<string>(["settings.sources"]);

/** Rows (label keys) that only show with Advanced on. */
export const ADVANCED_ROWS = new Set<string>([
  "sp.audio.channels",
  "sp.eq.headroom",
  "sp.appearance.startup",
  "sp.appearance.throttle",
  "sp.storage.limit",
  "update.channel",
  "sp.player.wheel_seek",
  "sp.player.wheel_volume",
]);

export const pageAdvanced = (p: SettingsPage) => ADVANCED_PAGES.has(p);
export const subHidden = (key: string) => !adv.on && ADVANCED_SUBTABS.has(key);

/** Translated labels of the rows hidden right now (rows only know their label). */
export function hiddenAdvancedLabels(): Set<string> {
  return adv.on ? new Set() : new Set([...ADVANCED_ROWS].map((k) => t(k)));
}
