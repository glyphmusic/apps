// Per-page context of the plugin rows (settings/plugin_ui.rs begin_page /
// end_page): the plugin page name and the labels of the rows on the page,
// so sections whose row is not there go to the end.
import { SvelteSet } from "svelte/reactivity";

export interface PluginHost { page: string; labels: SvelteSet<string> }
export const newHost = (page: string): PluginHost => ({ page, labels: new SvelteSet<string>() });
