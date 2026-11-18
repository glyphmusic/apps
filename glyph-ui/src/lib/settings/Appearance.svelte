<script lang="ts">
  // settings/appearance.rs appearance + theme_swatches: language, visual text
  // size card, background shape + theme + glow intensity & reactivity, mini
  // player style, ASCII covers & effects, throttle, startup animation.
  import { onMount } from "svelte";
  import { app } from "../state.svelte";
  import { t } from "../i18n.svelte";
  import { LANGS } from "../i18n.svelte";
  import Row from "./Row.svelte";
  import Rows from "./Rows.svelte";
  import Tog from "./Tog.svelte";
  import Select from "./Select.svelte";
  import PluginHost from "./PluginHost.svelte";
  import TextScaleVisual from "./TextScaleVisual.svelte";
  import GlowVisual from "./GlowVisual.svelte";
  import MiniPlayerVisual from "./MiniPlayerVisual.svelte";
  import AsciiVisual from "./AsciiVisual.svelte";
  import PerformanceVisual from "./PerformanceVisual.svelte";
  import { loadPlugins, pl } from "./plugins-state.svelte";
  import { onWeb } from "../web";
  import { isMobile } from "./util";

  let {}: { sub?: number } = $props();
  onMount(() => { if (!onWeb() && !pl.loaded) loadPlugins(); });

  const mobile = $derived(isMobile());
  const v = $derived(app.config.visuals);
  const langOptions = LANGS.map(([code, name]) => ({ value: code as string, label: name }));
  const lang = $derived(LANGS.find(([c]) => c === (app.config.language || ""))?.[0] ?? LANGS.find(([c]) => c === document.documentElement.lang)?.[0] ?? "en");
</script>

<PluginHost page="appearance">
  <div class="col">
    <Rows>
      <Row label={t("settings.language")} info={t("sp.appearance.lang.desc")}>
        <Select value={lang} options={langOptions} onpick={(c) => app.setConfig((cfg) => { cfg.language = c; })} />
      </Row>
    </Rows>

    <TextScaleVisual />

    <GlowVisual />

    {#if !onWeb() && !mobile}
      <MiniPlayerVisual />
    {/if}

    <AsciiVisual />

    <PerformanceVisual />

    <Rows>
      {#if !mobile}
        <Row label={t("sp.appearance.throttle")} info={t("sp.appearance.throttle.desc")}>
          <Tog on={v.throttle_unfocused} onchange={(x) => app.setConfig((c) => { c.visuals.throttle_unfocused = x; })} label={t("sp.appearance.throttle")} />
        </Row>
      {/if}
      <Row label={t("sp.appearance.startup")} info={t("sp.appearance.startup.desc")}>
        <Tog on={v.startup_animation} onchange={(x) => app.setConfig((c) => { c.visuals.startup_animation = x; })} label={t("sp.appearance.startup")} />
      </Row>
    </Rows>
  </div>
</PluginHost>

<style>
  .col { display: flex; flex-direction: column; gap: 14px; }
</style>
