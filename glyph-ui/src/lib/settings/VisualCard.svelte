<script lang="ts">
  // Flat dark surface card for visual settings previews. Registers its label
  // (and any extra labels) with PluginHost and highlights + scrolls into view
  // when picked from the settings search.
  import { getContext, onMount, type Snippet } from "svelte";
  import Icon from "../components/Icon.svelte";
  import { tip } from "../ui.svelte";
  import { sui } from "./settings.svelte";
  import { app } from "../state.svelte";
  import { hiddenLabels, pl } from "./plugins-state.svelte";
  import { t } from "../i18n.svelte";
  import type { PluginHost } from "./host";
  import PluginSection from "./PluginSection.svelte";

  let {
    label,
    info,
    extraLabels = [],
    dim = false,
    right,
    children,
  }: {
    label: string;
    info?: string | null;
    extraLabels?: string[];
    dim?: boolean;
    right?: Snippet;
    children: Snippet;
  } = $props();

  // Phones: the explanation under the label (two lines, a tap shows all).
  let more = $state(false);
  const host = getContext<PluginHost | undefined>("plugin-host");
  let el: HTMLDivElement | undefined = $state();

  const allLabels = $derived([label, ...extraLabels]);
  const hidden = $derived(hiddenLabels().has(label));
  const lit = $derived(sui.flash != null && allLabels.includes(sui.flash.label));
  const after = $derived(
    host
      ? pl.ui.sections.filter((s) => s.page === host.page && s.after && allLabels.includes(t(s.after)))
      : [],
  );

  $effect(() => {
    if (lit && el) el.scrollIntoView({ block: "center", behavior: "smooth" });
  });

  onMount(() => {
    if (!host) return;
    for (const l of allLabels) host.labels.add(l);
    return () => { for (const l of allLabels) host.labels.delete(l); };
  });
</script>

{#if !hidden}
  <div class="vcard" class:lit class:dim bind:this={el}>
    <div class="head">
      <div class="lt">
        <span class="lab">{label}</span>
        {#if info && !app.compact}
          <span class="info" use:tip={{ label: info, wrap: true }}>
            <Icon name="info" size={13} color="rgba(255,255,255,.42)" />
          </span>
        {/if}
      </div>
      {#if right}<div class="hr">{@render right()}</div>{/if}
    </div>
    {#if info && app.compact}<button class="desc" class:more onclick={() => (more = !more)} aria-expanded={more}>{info}</button>{/if}
    <div class="body">{@render children()}</div>
  </div>
  {#each after as s (s.plugin + s.title + s.after)}<PluginSection section={s} named />{/each}
{/if}

<style>
  .vcard {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 16px 18px;
    border-radius: 12px;
    background: rgba(255, 255, 255, .035);
    transition: opacity .2s ease-out;
  }
  .vcard.dim { opacity: .45; }
  .vcard.lit { animation: vflash 1.8s linear forwards; }
  @keyframes vflash {
    0%, 39% { background: rgba(var(--accent-rgb, 255, 95, 210), .14); }
    100% { background: rgba(255, 255, 255, .035); }
  }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
  .lt { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .lab { font-size: 14px; font-weight: 500; color: #fff; }
  .info { flex: none; width: 18px; height: 18px; border-radius: 9px; display: grid; place-items: center; transition: background-color .12s; }
  .info:hover { background: rgba(255, 255, 255, .08); }
  .desc { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; overflow: hidden; margin-top: 2px; padding: 0; text-align: left; font: 400 13px/18px var(--font); color: rgba(255, 255, 255, .55); }
  .desc.more { -webkit-line-clamp: unset; line-clamp: unset; }
  .vcard > .desc { margin-top: -10px; }
  /* Bigger tap target on touch screens (a tap shows the explanation). */
  .info { position: relative; cursor: help; }
  .info::after { content: ""; position: absolute; inset: -10px; }
  .hr { display: flex; align-items: center; gap: 8px; }
  .body { display: flex; flex-direction: column; gap: 14px; }
</style>
