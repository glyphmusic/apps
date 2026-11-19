<script lang="ts">
  // Appearance > Performance: full effects or performance mode (the two
  // picture cards live in PerfPicker, shared with the onboarding).
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import VisualCard from "./VisualCard.svelte";
  import PerfPicker from "./PerfPicker.svelte";

  const perf = $derived(!!app.config.visuals.performance_mode);
</script>

<VisualCard label={t("sp.perf")} info={perf ? t("sp.perf.on.desc") : t("sp.perf.off.desc")}>
  <PerfPicker />
</VisualCard>
