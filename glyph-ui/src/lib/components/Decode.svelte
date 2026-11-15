<script lang="ts">
  // Text that decodes in from ASCII noise the first time `key` is shown
  // (widgets/indicators.rs reveal_k + decode).
  import { decode, revealK } from "../ui.svelte";
  let { text, key, dur = 0.9 }: { text: string; key?: string; dur?: number } = $props();
  const k = $derived(key ? revealK(key, dur) : 1);
</script>

{decode(text, k)}
