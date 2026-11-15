<script lang="ts">
  // lists.rs back_button: floating "Back" pill, only when there is history
  // (not on phones). h 30, px 12, radius 8, black 55 %, 1 px white 10 %,
  // 12.5 white 90 %, 14 px arrow, gap 6, hover black 75 %.
  import Button from "../../components/Button.svelte";
  import { app } from "../../state.svelte";
  import { t } from "../../i18n.svelte";
  const touch = $derived(app.compact || app.platform === "android" || app.platform === "ios");
</script>

{#if app.history.length && !touch}
  <div class="bp"><Button kind="back" icon="arrow-left" onclick={() => app.back()}>{t("common.back")}</Button></div>
{/if}

<style>
  .bp { flex: none; display: flex; align-self: flex-start; }
</style>
