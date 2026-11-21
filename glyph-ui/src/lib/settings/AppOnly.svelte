<script lang="ts">
  // Web player: a settings row in place of what only the app can do
  // ("Available in the app" + what it is, and the download / install action).
  import Row from "./Row.svelte";
  import LinkButton from "./LinkButton.svelte";
  import Button from "../components/Button.svelte";
  import { t } from "../i18n.svelte";
  import { GET_APP_URL } from "../web";
  import { openInstallSheet, pwa } from "../pwa.svelte";
  let { info }: { info?: string } = $props();
</script>

<Row label={t("web.app_only")} info={info ?? t("web.app_only.sub")}>
  {#if pwa.ios}
    {#if !pwa.installed}
      <Button icon="smartphone" onclick={openInstallSheet}>{t("web.pwa.add_home")}</Button>
    {/if}
  {:else}
    <LinkButton icon="download" label={t("web.get_app")} url={GET_APP_URL} />
  {/if}
</Row>
