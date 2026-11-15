<script lang="ts">
  // lists.rs verified_badge: round blue check (hsl 212 90 % 56 %), white
  // check at 62 % of the size, tooltip "Verified artist".
  import Icon from "../../components/Icon.svelte";
  import { tip } from "../../ui.svelte";
  import { t } from "../../i18n.svelte";
  let { size = 14 }: { size?: number } = $props();
</script>

<span class="vb" style:width="{size}px" style:height="{size}px" use:tip={t("tip.verified")}><Icon name="check" size={size * 0.62} color="#fff" /></span>

<style>
  .vb { flex: none; display: grid; place-items: center; border-radius: 50%; background: hsl(212 90% 56%); }
</style>
