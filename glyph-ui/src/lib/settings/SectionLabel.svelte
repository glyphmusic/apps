<script lang="ts">
  // controls.rs section_label: pt 18, pb 2, 11.5 semibold, white 45 %, upper case.
  import type { Snippet } from "svelte";
  let { text, children }: { text?: string; children?: Snippet } = $props();
</script>

<div class="sl">{#if text}{text}{/if}{#if children}{@render children()}{/if}</div>

<style>
  .sl { padding: 18px 0 2px; font-size: 11.5px; font-weight: 600; color: rgba(255, 255, 255, .45); text-transform: uppercase; }
</style>
