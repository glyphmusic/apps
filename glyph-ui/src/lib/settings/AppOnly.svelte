<script lang="ts">
  // Web player: a settings row in place of what only the app can do
  // ("Available in the app" + what it is, and the download link).
  import Row from "./Row.svelte";
  import LinkButton from "./LinkButton.svelte";
  import { t } from "../i18n.svelte";
  import { GET_APP_URL } from "../web";
  let { info }: { info?: string } = $props();
</script>

<Row label={t("web.app_only")} info={info ?? t("web.app_only.sub")}>
  <LinkButton icon="download" label={t("web.get_app")} url={GET_APP_URL} />
</Row>
