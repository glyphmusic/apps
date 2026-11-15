<script lang="ts">
  // settings.rs row / row_status / row_full: a settings row that plugins can
  // hide (plugin_ui::row_hidden) and add sections under (sections_after).
  import { getContext, onMount, type Snippet } from "svelte";
  import RowBase from "./RowBase.svelte";
  import PluginSection from "./PluginSection.svelte";
  import { hiddenLabels, pl } from "./plugins-state.svelte";
  import { t } from "../i18n.svelte";
  import type { PluginHost } from "./host";
  let { label, info, status, statusColor, leading, children }: { label: string; info?: string | null; status?: string | null; statusColor?: string; leading?: Snippet; children?: Snippet } = $props();
  const host = getContext<PluginHost | undefined>("plugin-host");
  const hidden = $derived(hiddenLabels().has(label));
  const after = $derived(host ? pl.ui.sections.filter((s) => s.page === host.page && s.after && t(s.after) === label) : []);
  onMount(() => { host?.labels.add(label); return () => host?.labels.delete(label); });
</script>

{#if !hidden}
  <div class="srow">
    <RowBase {label} {info} {status} {statusColor} {leading} {children} />
    {#each after as s (s.plugin + s.title + s.after)}<PluginSection section={s} named />{/each}
  </div>
{/if}
