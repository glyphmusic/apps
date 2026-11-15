import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";
import { installAndroidBack } from "./lib/compact/cstate.svelte";
import { isTauri } from "./lib/api/backend";
import { installBrowserKeyBlocker } from "./lib/keys";

installAndroidBack();
if (isTauri()) installBrowserKeyBlocker();

// Right click belongs to Glyph's own menus, not the webview's.
addEventListener("contextmenu", (e) => {
  const el = e.target as HTMLElement;
  if (!el.closest("input, textarea, [contenteditable]")) e.preventDefault();
});

export default mount(App, { target: document.getElementById("app")! });
