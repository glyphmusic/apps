<script lang="ts">
  // Settings > Notifications (Android only, notify.rs): two switches, like
  // System's rows. Each kind also has its own channel in Android's settings.
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import Tog from "./Tog.svelte";
  import PluginHost from "./PluginHost.svelte";

  let {}: { sub?: number } = $props();
  const n = $derived(app.config.notifications ?? { app_updates: true, new_releases: true });
  function set(key: "app_updates" | "new_releases", on: boolean) {
    app.setConfig((c) => { c.notifications = { ...(c.notifications ?? { app_updates: true, new_releases: true }), [key]: on }; });
  }
</script>

<PluginHost page="notifications">
  <div class="col">
    <Rows>
      <Row label={t("sp.notify.updates")} info={t("sp.notify.updates.desc")}>
        <Tog on={n.app_updates} onchange={(on) => set("app_updates", on)} label={t("sp.notify.updates")} />
      </Row>
      <Row label={t("sp.notify.releases")} info={t("sp.notify.releases.desc")}>
        <Tog on={n.new_releases} onchange={(on) => set("new_releases", on)} label={t("sp.notify.releases")} />
      </Row>
    </Rows>
  </div>
</PluginHost>

<style>
  .col { display: flex; flex-direction: column; gap: 14px; }
</style>
