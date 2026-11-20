// Things the backend asks the UI to do for plugins, AI assistants and imports
// (src/app/plugin_api.rs ui.open / ui.search / ui.open_artist / lyrics.publish,
// controls.rs mcp_approval, import.rs open_list after an import).
import { backend } from "../api/backend";
import { app } from "../state.svelte";
import { tf } from "../i18n.svelte";
import { openPlugin, setTab } from "./settings.svelte";
import { loadPlugins, pl } from "./plugins-state.svelte";
import { lyr } from "../pages/nowplaying/lyrics-state.svelte";
import type { Lyrics } from "../api/types";

/** An AI assistant asking for access (oldest first); the dialog answers the first. */
export interface McpApproval { id: string; client: string; code: string }
export const mcpq = $state<{ approvals: McpApproval[] }>({ approvals: [] });

export function answerMcp(id: string, allow: boolean) {
  mcpq.approvals = mcpq.approvals.filter((a) => a.id !== id);
  backend().call("mcp_approve", { id, allow });
}

let started = false;

export async function initPluginEvents() {
  if (started) return;
  started = true;
  const api = backend();

  api.on("plugin_ui", async (e) => {
    if (e.kind === "page") {
      if (!pl.loaded) await loadPlugins();
      const i = pl.ui.pages.findIndex((p) => p.id === e.id);
      if (i >= 0) { app.settingsOpen = true; setTab("plugins", 2 + i); }
    } else if (e.kind === "search") app.searchFor(e.query);
    else if (e.kind === "artist") app.openArtistPage(e.name);
    else if (e.kind === "install") openPlugin(e.id);
    // An AI assistant installed, removed or switched a plugin: show the new list and say so.
    else if (e.kind === "changed") {
      await loadPlugins();
      app.toast(tf(`mcp.plugins.${e.what}`, { name: e.name }), "info");
    }
  });

  // Lyrics a plugin published: shown when the song has none (or already shows plugin lyrics).
  api.on("plugin_lyrics", (e) => {
    if (e.key !== app.currentKey) return;
    const shown = app.lyrics.lyrics;
    if (!shown || shown.source?.startsWith("Plugin: ")) app.lyrics = { key: e.key, lyrics: e.lyrics as Lyrics, loading: false };
    // The choices list is read again the next time it is needed.
    lyr.loaded = false;
  });

  // A plugin stopped: its lyrics are no choice anymore, and shown ones are replaced.
  api.on("plugin_lyrics_reset", () => {
    lyr.loaded = false;
    if (app.lyrics.lyrics?.source?.startsWith("Plugin: ")) app.loadLyrics();
  });

  api.on("open_playlist", (id) => app.openListView({ kind: "playlist", id }));

  api.on("mcp_approvals", (list) => (mcpq.approvals = list));
  const pending = await backend().call<McpApproval[] | undefined>("mcp_pending");
  if (pending?.length) mcpq.approvals = pending;
}
