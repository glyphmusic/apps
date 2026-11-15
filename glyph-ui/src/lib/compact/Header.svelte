<script lang="ts">
  // compact.rs header: profile picture (opens the drawer, as a swipe right
  // does) + page title 22 bold + actions on the right. h 60, px 16, gap 12.
  import type { Snippet } from "svelte";
  import Avatar from "./Avatar.svelte";
  import { phone, drag, SWIPE_ACT } from "./cstate.svelte";
  let { title, actions }: { title?: string; actions?: Snippet } = $props();
</script>

<header class="hd" use:drag={{ axis: "x", end: (dx) => dx > SWIPE_ACT && (phone.drawerOpen = true) }}>
  <button class="pfp" onclick={() => (phone.drawerOpen = true)} aria-label="Profile"><Avatar size={40} /></button>
  {#if title}<h1 class="t trunc">{title}</h1>{:else}<span class="flex1"></span>{/if}
  {#if actions}{@render actions()}{/if}
</header>

<style>
  .hd { flex: none; height: 60px; padding: 0 16px; display: flex; align-items: center; gap: 12px; touch-action: pan-y; }
  .pfp { flex: none; border-radius: 50%; }
  .t { flex: 1; margin: 0; font-size: 22px; font-weight: 700; color: var(--text); }
</style>
