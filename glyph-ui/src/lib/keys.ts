// Keyboard shortcuts (ui/keys.rs SHORTCUTS, ui/mod.rs actions). Keys are
// written like the app's config: "secondary-k" = Ctrl on Windows/Linux,
// Cmd on macOS. The user's own keys come from config.ui.keys.
import { app } from "./state.svelte";
import { videoState } from "./pages/nowplaying/video-state.svelte";
import { trackKey } from "./api/types";
import { currentListNav, stepRowIndex } from "./state/list-nav.svelte";
import { isMac, close } from "./window";

export interface Shortcut { id: string; label: string; key: string; scope: "main" | "window" | "global"; group: "playback" | "navigation" | "app" }

const s = (id: string, label: string, key: string, scope: Shortcut["scope"], group: Shortcut["group"]): Shortcut => ({ id, label, key, scope, group });
export const SHORTCUTS: Shortcut[] = [
  s("play", "sp.keys.play", "space", "main", "playback"),
  s("seek_back", "sp.keys.seek_back", "left", "main", "playback"),
  s("seek_forward", "sp.keys.seek_forward", "right", "main", "playback"),
  s("prev", "sp.keys.prev", "secondary-left", "window", "playback"),
  s("next", "sp.keys.next", "secondary-right", "window", "playback"),
  s("volume_up", "sp.keys.volume_up", "secondary-up", "window", "playback"),
  s("volume_down", "sp.keys.volume_down", "secondary-down", "window", "playback"),
  s("mute", "sp.keys.mute", "m", "main", "playback"),
  s("like", "sp.keys.like", "secondary-shift-l", "window", "playback"),
  s("shuffle", "sp.keys.shuffle", "secondary-s", "window", "playback"),
  s("repeat", "sp.keys.repeat", "secondary-r", "window", "playback"),
  s("download", "sp.keys.download", "secondary-d", "main", "playback"),
  s("loop_start", "sp.keys.loop_start", "alt-1", "global", "playback"),
  s("loop_end", "sp.keys.loop_end", "alt-2", "global", "playback"),
  s("loop_clear", "sp.keys.loop_clear", "alt-0", "global", "playback"),
  s("search", "sp.keys.search", "secondary-k", "global", "navigation"),
  s("lyrics", "sp.keys.lyrics", "secondary-l", "global", "navigation"),
  s("zen", "sp.keys.zen", "z", "main", "navigation"),
  s("queue", "sp.keys.queue", "secondary-j", "window", "navigation"),
  s("settings", "sp.keys.settings", "secondary-0", "global", "navigation"),
  // Always reaches Settings > Plugins, where plugin looks are paused (a look
  // that hides the interface can be turned off from there).
  s("plugins", "sp.keys.plugins", "secondary-alt-p", "global", "app"),
  s("feedback", "sp.keys.feedback", "secondary-shift-f", "global", "app"),
  s("quit", "sp.keys.quit", "secondary-q", "global", "app"),
];
/** Keys that stay fixed (lists, sidebar, closing). */
export const FIXED = ["up", "down", "enter", "delete", "backspace", "escape", "tab", "shift-tab", ...Array.from({ length: 9 }, (_, i) => `secondary-${i + 1}`)];

export function keyOf(id: string): string {
  return app.config?.ui.keys?.[id] ?? SHORTCUTS.find((x) => x.id === id)?.key ?? "";
}

/** The key of an event, in the config's notation. */
export function eventKey(e: KeyboardEvent): string {
  if (!e || !e.key) return "";
  const parts: string[] = [];
  const secondary = isMac() ? e.metaKey : e.ctrlKey;
  if (secondary) parts.push("secondary");
  if (isMac() && e.ctrlKey) parts.push("ctrl");
  if (!isMac() && e.metaKey) parts.push("cmd");
  if (e.altKey) parts.push("alt");
  if (e.shiftKey) parts.push("shift");
  const k = e.key === " " ? "space" : e.key.startsWith("Arrow") ? e.key.slice(5).toLowerCase() : e.key.length === 1 ? e.key.toLowerCase() : e.key.toLowerCase();
  parts.push(k);
  return parts.join("-");
}

/** How a key reads on this system ("Ctrl K", "⌘K"). */
export function keyLabel(key: string): string {
  if (!key) return "";
  const mac = isMac();
  return key.split("-").map((p) => ({
    secondary: mac ? "⌘" : "Ctrl", ctrl: mac ? "⌃" : "Ctrl", cmd: mac ? "⌘" : "Win", alt: mac ? "⌥" : "Alt", shift: mac ? "⇧" : "Shift",
    space: "Space", left: "←", right: "→", up: "↑", down: "↓", escape: "Esc", enter: "Enter",
  } as Record<string, string>)[p] ?? p.toUpperCase()).join(mac ? "" : " ");
}

function run(id: string, focusSearch: () => void) {
  switch (id) {
    case "play": return videoState.handlePlayPauseShortcut() ? undefined : app.togglePlay();
    case "seek_back": return app.seekRelative(-5);
    case "seek_forward": return app.seekRelative(5);
    case "prev": return app.previous();
    case "next": return app.next();
    case "volume_up": return app.setVolume(Math.min(1, app.volume + 0.05));
    case "volume_down": return app.setVolume(Math.max(0, app.volume - 0.05));
    case "mute": return app.toggleMute();
    case "like": return app.player.current && app.toggleLike(app.player.current);
    case "shuffle": return app.toggleShuffle();
    case "repeat": return app.cycleRepeat();
    case "download": {
      const cur = app.player.current;
      if (!cur) return;
      return trackKey(cur) in app.downloading ? app.cancelDownloads([cur]) : app.download([cur]);
    }
    case "loop_start": return app.setLoop(app.position, app.player.loop_b ?? null);
    case "loop_end": return app.setLoop(app.player.loop_a ?? 0, app.position);
    case "loop_clear": return app.setLoop(null, null);
    case "search": if (app.view !== "search") app.navigate("search"); return focusSearch();
    case "lyrics": if (app.view !== "now_playing") app.navigate("now_playing"); else app.showLyrics = !app.showLyrics; return;
    case "zen": app.zen = !app.zen; if (app.zen && app.view !== "now_playing") app.navigate("now_playing"); return;
    case "queue": app.showQueue = !app.showQueue; if (app.showQueue) app.showDevices = false; return;
    case "settings": return app.navigate("settings");
    case "plugins": return app.openSettings("plugins");
    case "feedback": app.feedbackOpen = !app.feedbackOpen; return;
    case "quit": return close();
  }
}

export function isBareShortcut(key: string): boolean {
  return !/secondary|ctrl|alt|cmd/.test(key) && !/^f\d+$/.test(key.split("-").pop()!);
}

export function installKeys(focusSearch: () => void): () => void {
  const views = ["home", "search", "library", "now_playing"] as const;
  const down = (e: KeyboardEvent) => {
    const target = e.target as HTMLElement | null;
    const typing = target?.closest("input, textarea, [contenteditable]") != null;
    const k = eventKey(e);
    if (k === "escape") {
      if (app.ctx || app.controlMenu) return app.closeMenus();
      if (app.feedbackOpen) return (app.feedbackOpen = false);
      if (app.nameDialog) return (app.nameDialog = null);
      if (app.confirm) return (app.confirm = null);
      if (app.settingsOpen) return (app.settingsOpen = false);
      if (app.zen) return (app.zen = false);
      if (app.picking) return app.stopPicking();
      if (!typing && app.history.length) return app.back();
      return;
    }
    if (k === "alt-left") { e.preventDefault(); app.back(); return; }
    if (k === "alt-right") { e.preventDefault(); app.forward(); return; }
    const nav = /^secondary-([1-9])$/.exec(k);
    if (nav) { e.preventDefault(); const v = views[+nav[1] - 1]; if (v) app.navigate(v); return; }

    // Track-list keyboard navigation (↑ / ↓ / Enter / Delete) when no overlay/input is active.
    const listNav = !typing && !app.compact && !app.searchFocused && !app.settingsOpen && !app.nameDialog && !app.confirm && !app.feedbackOpen && !app.ctx && !app.controlMenu
      ? currentListNav()
      : null;
    if (listNav) {
      if (k === "down" || k === "shift-down" || k === "up" || k === "shift-up") {
        const count = listNav.count();
        const delta = k.endsWith("down") ? 1 : -1;
        const next = stepRowIndex(app.selectedRow, delta, count);
        if (next != null) {
          e.preventDefault();
          app.selectedRow = next;
          listNav.scrollTo?.(next);
          if (e.shiftKey && listNav.isPicking?.() && listNav.togglePick) {
            listNav.togglePick(next, true);
          }
          return;
        }
      } else if (k === "enter" && app.selectedRow != null) {
        const interactive = target?.closest("button, a, select, [role='button'], [role='menuitem']") != null;
        if (!interactive) {
          const count = listNav.count();
          if (app.selectedRow >= 0 && app.selectedRow < count) {
            e.preventDefault();
            if (listNav.isPicking?.() && listNav.togglePick) listNav.togglePick(app.selectedRow, e.shiftKey);
            else listNav.play(app.selectedRow);
            return;
          }
        }
      } else if ((k === "delete" || k === "backspace") && app.selectedRow != null && listNav.remove) {
        const interactive = target?.closest("button, a, select") != null;
        const count = listNav.count();
        if (!interactive && app.selectedRow >= 0 && app.selectedRow < count) {
          e.preventDefault();
          const idx = app.selectedRow;
          listNav.remove(idx);
          const remaining = count - 1;
          app.selectedRow = remaining > 0 ? Math.min(idx, remaining - 1) : null;
          return;
        }
      }
    }

    for (const sc of SHORTCUTS) {
      const key = keyOf(sc.id);
      if (!key || key !== k) continue;
      if (isBareShortcut(key) && typing) return;
      if (sc.scope === "main" && (app.settingsOpen || app.nameDialog || app.feedbackOpen)) return;
      e.preventDefault();
      run(sc.id, focusSearch);
      return;
    }
  };
  const mouse = (e: MouseEvent) => {
    if (e.button === 3) app.back();
    else if (e.button === 4) app.forward();
  };
  addEventListener("keydown", down);
  addEventListener("mouseup", mouse);
  return () => { removeEventListener("keydown", down); removeEventListener("mouseup", mouse); };
}

const BLOCKED_BROWSER_KEYS = new Set([
  "f3", "f5", "f7", "f12",
  "secondary-shift-i", "secondary-shift-j", "secondary-shift-c", "secondary-shift-k",
  "secondary-shift-p", "secondary-shift-r", "secondary-f5",
  "cmd-alt-i", "cmd-alt-j", "cmd-alt-c", "cmd-alt-u",
  "secondary-u", "secondary-r", "secondary-f", "secondary-g",
  "secondary-p", "secondary-s", "secondary-o",
  "alt-left", "alt-right",
]);

/** Prevents DevTools and native browser shortcuts in the desktop shell without
 *  stopping propagation so Glyph's own shortcuts (e.g. Ctrl+R, Ctrl+S) still run. */
export function installBrowserKeyBlocker(): () => void {
  const onKeyDown = (e: KeyboardEvent) => {
    if (BLOCKED_BROWSER_KEYS.has(eventKey(e))) e.preventDefault();
  };
  addEventListener("keydown", onKeyDown, true);
  return () => removeEventListener("keydown", onKeyDown, true);
}
