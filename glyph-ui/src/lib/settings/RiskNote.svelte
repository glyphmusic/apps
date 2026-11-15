<script lang="ts">
  // plugins.rs risk_note: always on the catalog and detail views, plugins
  // are other people's work and installed at your own risk. (i) icon 14,
  // text 12.5 / 18, white 50 %, pt 12.
  import Icon from "../components/Icon.svelte";
  import { t } from "../i18n.svelte";
</script>

<div class="rn">
  <span class="i"><Icon name="info" size={14} color="rgba(255,255,255,.45)" /></span>
  <div class="tx">{t("plugins.risk_note")}</div>
</div>

<style>
  .rn { display: flex; align-items: flex-start; gap: 8px; padding: 4px 0 12px; }
  .i { padding-top: 1px; display: flex; }
  .tx { flex: 1; min-width: 0; font-size: 12px; line-height: 17px; color: rgba(255, 255, 255, .44); }
</style>
