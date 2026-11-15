<script lang="ts">
  // app.spinner() + widgets/bars.rs spinner_line: | / - \ every 110 ms in
  // mono, then a label (sm, dim).
  import { onMount } from "svelte";
  let { label = "" }: { label?: string } = $props();
  const F = ["|", "/", "-", "\\"];
  let i = $state(0);
  onMount(() => { const id = setInterval(() => (i = (i + 1) % 4), 110); return () => clearInterval(id); });
</script>

<span class="sp"><span class="mono ch">{F[i]}</span>{#if label}<span>{label}</span>{/if}</span>

<style>
  .sp { display: inline-flex; align-items: center; gap: 8px; font-size: var(--sm); color: var(--dim); }
  .ch { color: var(--text); width: 1ch; text-align: center; }
</style>
