<script lang="ts">
  // plugin_ui.rs begin_page / end_page: wraps the body of a Glyph page so
  // plugin sections can join it (after a row, or at the end).
  import { setContext, type Snippet } from "svelte";
  import PluginSection from "./PluginSection.svelte";
  import { newHost } from "./host";
  import { pl } from "./plugins-state.svelte";
  import { t } from "../i18n.svelte";
  let { page, children }: { page: string; children: Snippet } = $props();
  const host = newHost("");
  $effect(() => { host.page = page; });
  setContext("plugin-host", host);
  // Sections without a row to hang under, or whose row is not on this page.
  const rest = $derived(pl.ui.sections.filter((s) => s.page === host.page && (!s.after || !host.labels.has(t(s.after)))));
</script>

<div class="body">
  {@render children()}
  {#each rest as s (s.plugin + s.title + s.after)}<PluginSection section={s} named />{/each}
</div>

<style>
  .body { display: flex; flex-direction: column; }
</style>
