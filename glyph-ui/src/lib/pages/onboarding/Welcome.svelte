<script lang="ts">
  // onboarding.rs welcome(): the note assembles out of its characters and
  // dodges the mouse; row_h = clamp((h - 34) / 33, 3.5, 6); a mono hint
  // (11, white 40 %) fades in at 2.2 s (not on touch).
  import NoteLogo from "../../overlays/NoteLogo.svelte";
  let { h }: { s?: number; h: number } = $props();
  const rowH = $derived(Math.min(6, Math.max(3.5, (h - 34) / 33)));
</script>

<div class="w">
  {#key rowH}<NoteLogo {rowH} flyIn />{/key}
</div>

<style>
  .w { width: 100%; height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; }
</style>
