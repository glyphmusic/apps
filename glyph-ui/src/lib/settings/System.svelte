<script lang="ts">
  // settings/system.rs system (desktop only): start with the computer, and
  // visual start mode selector (45 % while autostart is off).
  import { app } from "../state.svelte";
  import { backend } from "../api/backend";
  import { t, tf } from "../i18n.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import Tog from "./Tog.svelte";
  import PluginHost from "./PluginHost.svelte";
  import StartModeVisual from "./StartModeVisual.svelte";
  import { errText } from "./util";

  let {}: { sub?: number } = $props();
  const d = $derived(app.config.desktop);

  async function autostart(on: boolean) {
    try {
      await backend().call("set_autostart", { on });
      app.setConfig((c) => { c.desktop.autostart = on; });
    } catch (e) { app.toast(tf("sp.system.autostart.error", { e: errText(e) })); }
  }
</script>

<PluginHost page="system">
  <div class="col">
    <Rows>
      <Row label={t("sp.system.autostart")} info={t("sp.system.autostart.desc")}>
        <Tog on={d.autostart} onchange={autostart} label={t("sp.system.autostart")} />
      </Row>
    </Rows>
    <StartModeVisual />
  </div>
</PluginHost>

<style>
  .col { display: flex; flex-direction: column; gap: 14px; }
</style>
