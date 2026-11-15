<script lang="ts" generics="T extends string | number | null">
  // controls.rs select: a button with the current value (h 34, pl 12, pr 10,
  // radius 8, white 6 % / 10 % open, border white 6 %, 13.5, value max 220
  // px) and a list that floats below it, right-aligned (min w 240, p 4,
  // radius 10, #1f1f1f, entries h 36 with a check on the selected one).
  // Portaled to document.body so ancestor transforms / overflow never displace it.
  import { onDestroy } from "svelte";
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Icon from "../components/Icon.svelte";
  import ListCover from "../components/ListCover.svelte";
  import { fadeEdges } from "../ui.svelte";
  import type { ListKey } from "../api/types";

  type SubOpt<V> = { value?: V; label?: string; sub?: string; icon?: string; list?: ListKey; sep?: boolean };
  type Opt<V> = SubOpt<V> & { children?: SubOpt<V>[] };

  let { value, options, onpick, label }: { value: T; options: Opt<T>[]; onpick: (v: T) => void; label?: string } = $props();
  let open = $state(false);
  let openSub = $state<number | null>(null);
  let drillSub = $state<number | null>(null);
  let wrap: HTMLDivElement | undefined = $state();
  let btn: HTMLButtonElement | undefined = $state();
  let listEl: HTMLDivElement | undefined = $state();
  let subEl: HTMLDivElement | undefined = $state();
  let rowEls: Record<number, HTMLButtonElement | undefined> = {};
  let pos = $state<{ top?: number; bottom?: number; right: number; maxH: number; up: boolean }>({ top: 0, right: 0, maxH: 400, up: false });
  let subTop = $state(0);
  let subFlip = $state(false);

  function findSelected(opts: Opt<T>[]): { label: string } | undefined {
    for (const o of opts) {
      if (o.sep) continue;
      if (o.children) {
        const hit = findSelected(o.children);
        if (hit) return hit;
      } else if (o.value === value && o.label != null) {
        return { label: o.label };
      }
    }
    return undefined;
  }
  function hasSelected(opts: SubOpt<T>[]): boolean {
    return findSelected(opts) !== undefined;
  }

  const shown = $derived(label ?? findSelected(options)?.label ?? String(value ?? ""));
  const subOpt = $derived(openSub != null ? options[openSub] : undefined);
  const subItems = $derived(subOpt && !subOpt.sep && subOpt.children ? subOpt.children : null);
  const drillOpt = $derived(drillSub != null ? options[drillSub] : undefined);
  const drillItems = $derived(drillOpt && !drillOpt.sep && drillOpt.children ? drillOpt.children : null);

  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        node.remove();
      },
    };
  }

  function openFlyout(i: number) {
    if (innerWidth < 540) return;
    openSub = i;
    const row = rowEls[i];
    if (!row || !listEl) return;
    const rr = row.getBoundingClientRect();
    const lr = listEl.getBoundingClientRect();
    subFlip = innerWidth - lr.right < 260 && lr.left >= 240;
    subTop = rr.top - lr.top - 4;
  }

  $effect(() => {
    if (openSub == null || !subEl || !listEl) return;
    const sr = subEl.getBoundingClientRect();
    const overBottom = sr.bottom - (innerHeight - 8);
    if (overBottom > 0) subTop = Math.max(8 - listEl.getBoundingClientRect().top, subTop - overBottom);
  });

  function toggle() {
    if (open) return close();
    const r = btn!.getBoundingClientRect();
    const spaceBelow = innerHeight - r.bottom - 16;
    const spaceAbove = r.top - 16;
    const up = spaceBelow < 180 && spaceAbove > spaceBelow;
    const right = Math.max(8, Math.min(innerWidth - r.right, innerWidth - 248));
    pos = up
      ? { bottom: innerHeight - r.top + 6, right, maxH: Math.max(120, spaceAbove - 6), up: true }
      : { top: r.bottom + 6, right, maxH: Math.max(120, spaceBelow - 6), up: false };
    openSub = null;
    drillSub = null;
    open = true;
    addEventListener("pointerdown", away, true);
    addEventListener("keydown", key, true);
    addEventListener("resize", close);
    addEventListener("scroll", scrolled, true);
  }
  function close() {
    open = false;
    openSub = null;
    drillSub = null;
    removeEventListener("pointerdown", away, true);
    removeEventListener("keydown", key, true);
    removeEventListener("resize", close);
    removeEventListener("scroll", scrolled, true);
  }
  function scrolled(e: Event) {
    const t = e.target as Node;
    if (!listEl?.contains(t) && !subEl?.contains(t)) close();
  }
  function away(e: PointerEvent) {
    const t = e.target as Node;
    if (!wrap?.contains(t) && !listEl?.contains(t) && !subEl?.contains(t)) close();
  }
  // Escape shuts the list, not the settings behind it.
  function key(e: KeyboardEvent) { if (e.key === "Escape") { e.stopImmediatePropagation(); e.preventDefault(); close(); } }
  onDestroy(close);
</script>

{#snippet renderOption(o: Opt<T>, i: number, inSub: boolean)}
  {#if o.sep}
    <div class="sep"></div>
  {:else if o.children}
    {@const active = hasSelected(o.children)}
    {@const subOpenNow = openSub === i}
    <button
      class="opt"
      class:on={active || subOpenNow}
      bind:this={rowEls[i]}
      role="menuitem"
      aria-haspopup="menu"
      aria-expanded={subOpenNow || drillSub === i}
      onpointerenter={() => openFlyout(i)}
      onclick={() => { if (innerWidth < 540) drillSub = i; else openFlyout(i); }}
    >
      <span class="opt-l">
        {#if o.icon}<Icon name={o.icon} size={14} color="currentColor" />{/if}
        <span class="trunc">{o.label}</span>
      </span>
      <span class="opt-r">
        {#if active}<span class="dot"></span>{/if}
        <Icon name="chevron-right" size={14} color="rgba(255,255,255,.45)" />
      </span>
    </button>
  {:else}
    {@const selected = o.value === value}
    <button
      class="opt"
      class:rich={!!(o.list || o.sub)}
      class:on={selected}
      role="option"
      aria-selected={selected}
      onpointerenter={() => { if (!inSub) openSub = null; }}
      onclick={() => { if (o.value !== undefined) { close(); onpick(o.value); } }}
    >
      <span class="opt-l">
        {#if o.list}
          <ListCover list={o.list} size={28} />
        {:else if o.icon}
          <Icon name={o.icon} size={14} color="currentColor" />
        {/if}
        {#if o.sub}
          <span class="opt-tx">
            <span class="opt-nm trunc">{o.label}</span>
            <span class="opt-sub trunc">{o.sub}</span>
          </span>
        {:else}
          <span class="trunc">{o.label}</span>
        {/if}
      </span>
      {#if selected}<Icon name="check" size={14} color="#fff" />{/if}
    </button>
  {/if}
{/snippet}

<div class="sel" bind:this={wrap}>
  <button class="btn" class:open bind:this={btn} onclick={toggle} aria-haspopup="listbox" aria-expanded={open}>
    <span class="v">{shown}</span>
    <Icon name="chevron-down" size={14} color="rgba(255,255,255,.6)" />
  </button>
  {#if open}
    <div
      class="list-wrap"
      use:portal
      style:top={pos.top != null ? `${pos.top}px` : undefined}
      style:bottom={pos.bottom != null ? `${pos.bottom}px` : undefined}
      style:right="{pos.right}px"
      transition:fly={{ y: pos.up ? 6 : -6, duration: 140, easing: cubicOut }}
    >
      <div class="list" role="listbox" bind:this={listEl}>
        <div class="list-in scroll" use:fadeEdges={{ top: 10, bottom: 10, minOverflow: 8 }} style:max-height="{pos.maxH}px">
          {#if drillItems && drillOpt && !drillOpt.sep}
            <button class="opt back" onclick={() => (drillSub = null)}>
              <span class="opt-l">
                <Icon name="chevron-left" size={14} color="rgba(255,255,255,.6)" />
                <span class="trunc">{drillOpt.label}</span>
              </span>
            </button>
            <div class="sep"></div>
            {#each drillItems as child, ci (`d-${ci}`)}
              {@render renderOption(child, ci, true)}
            {/each}
          {:else}
            {#each options as o, i (`o-${i}`)}
              {@render renderOption(o, i, false)}
            {/each}
          {/if}
        </div>
      </div>

      {#if subItems}
        <div
          class="sub-list"
          class:flip={subFlip}
          bind:this={subEl}
          role="menu"
          style:top="{subTop}px"
          transition:fly={{ x: subFlip ? 6 : -6, duration: 120, easing: cubicOut }}
        >
          <div class="sub-box">
            <div class="list-in scroll" use:fadeEdges={{ top: 10, bottom: 10, minOverflow: 8 }}>
              {#each subItems as child, ci (`s-${ci}`)}
                {@render renderOption(child, ci, true)}
              {/each}
            </div>
          </div>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .sel { position: relative; max-width: 100%; }
  .btn { height: 34px; padding: 0 10px 0 12px; display: flex; align-items: center; gap: 8px; max-width: 100%; border-radius: 8px; background: #1e1e20; font-size: 13.5px; color: #fff; transition: background-color .12s ease-out; }
  .btn:hover, .btn.open { background: #2a2a2e; }
  .v { max-width: 220px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .list-wrap { position: fixed; z-index: 70; min-width: 240px; }
  .list { border-radius: 10px; background: #1f1f1f; overflow: hidden; }
  .sub-list { position: absolute; left: calc(100% + 6px); width: 256px; z-index: 1; }
  .sub-list::before { content: ""; position: absolute; inset: 0 -8px; z-index: -1; }
  .sub-list.flip { left: auto; right: calc(100% + 6px); }
  .sub-box { border-radius: 10px; background: #1f1f1f; overflow: hidden; }
  .sub-box .list-in { max-height: min(340px, calc(100vh - 24px)); }
  .list-in { padding: 4px; display: flex; flex-direction: column; gap: 2px; }
  .sep { flex: none; height: 1px; margin: 4px 6px; background: rgba(255, 255, 255, .07); }
  .opt { flex: none; width: 100%; height: 36px; padding: 0 10px; display: flex; align-items: center; justify-content: space-between; gap: 12px; border-radius: 7px; font-size: 13.5px; color: rgba(255, 255, 255, .7); transition: background-color .12s, color .12s; text-align: left; }
  .opt.rich { height: auto; min-height: 40px; padding: 5px 10px; }
  .opt.back { color: #fff; font-weight: 500; }
  .opt-l { flex: 1; min-width: 0; display: flex; align-items: center; gap: 9px; }
  .opt-r { flex: none; display: flex; align-items: center; gap: 6px; }
  .dot { width: 5px; height: 5px; border-radius: 50%; background: #fff; }
  .opt-tx { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .opt-nm { font-size: 13px; color: inherit; font-weight: 500; }
  .opt-sub { font-size: 11.5px; color: rgba(255, 255, 255, .5); }
  .opt:hover { background: rgba(255, 255, 255, .05); color: #fff; }
  .opt.on { background: rgba(255, 255, 255, .08); color: #fff; }
</style>
