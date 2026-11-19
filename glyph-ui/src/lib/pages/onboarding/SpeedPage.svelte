<script lang="ts">
  // The speed page: full effects or performance mode, the same cards as
  // Settings > Appearance > Performance. The one that suits this device
  // (checked on first start, glow.ts recommendVisuals) is marked
  // "Recommended"; below, who draws the background (graphics card or
  // processor, also picked on first start).
  import { app } from "../../state.svelte";
  import { t } from "../../i18n.svelte";
  import { recommendVisuals } from "../../glow";
  import PerfPicker from "../../settings/PerfPicker.svelte";
  import { easeOut, k } from "./anim.svelte";
  let { s, w }: { s: number; w: number } = $props();

  const rec = recommendVisuals().perf;
  const gpu = $derived(app.config?.visuals.render_mode !== "cpu" && !app.config?.visuals.gpu_blocked);
  const pw = $derived(Math.min(w - 32, 480));
</script>

<div class="p">
  <div class="pick" style:width="{pw}px" style:opacity={easeOut(k(s, 0.05, 0.35))}>
    <PerfPicker recommended={rec} compact={pw < 440} />
  </div>
  <div class="dev" style:opacity={easeOut(k(s, 0.3, 0.4))}>
    <span class="chip mono">{gpu ? "GPU" : "CPU"}</span>
    <span>{gpu ? t("onboarding.speed.gpu") : t("onboarding.speed.cpu")}</span>
  </div>
</div>

<style>
  .p { width: 100%; height: 100%; padding: 16px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; }
  .pick { max-width: 100%; }
  .dev { display: flex; align-items: center; gap: 8px; font-size: 12px; color: rgba(255, 255, 255, .55); text-align: center; }
  .chip { flex: none; padding: 1px 6px; border-radius: 4px; font-size: 10px; color: #fff; background: rgba(255, 255, 255, .12); }
</style>
